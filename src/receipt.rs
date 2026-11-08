//! The receipt: what grh placed, at `$XDG_STATE_HOME/grh/receipt.json`.
//!
//! One grh at a time holds `receipt.lock` (an exclusive `File::lock` whose
//! holder writes its pid into the file). Every write of the receipt is a
//! temp file, fsync and rename. A step is written `pending` before it runs
//! and set `done` after, so a crash leaves the unfinished step visible.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::exit::Exit;
use crate::manifest::{ExitPair, KeepBinary, RawStatus, Status};

pub const VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Level {
    Machine,
    Repo,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum State {
    Pending,
    Done,
}

/// A status query as the receipt keeps it. The receipt is a persisted file
/// older and newer grh both read, so these records ignore unknown keys; the
/// manifest types they mirror refuse them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordedStatus {
    pub args: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit: Option<RecordedExitPair>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub json: Option<Vec<String>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordedExitPair {
    pub present: i32,
    pub absent: i32,
}

/// A `keep_binary` query as the receipt keeps it; see `RecordedStatus`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordedKeepBinary {
    pub args: Vec<String>,
    pub state: Vec<String>,
    pub absent: String,
    pub shims: Vec<String>,
    pub binary: String,
    pub warning: String,
}

impl From<&Status> for RecordedStatus {
    fn from(status: &Status) -> RecordedStatus {
        let raw = RawStatus::from(status);
        RecordedStatus {
            args: raw.args,
            exit: raw.exit.map(|pair| RecordedExitPair {
                present: pair.present,
                absent: pair.absent,
            }),
            json: raw.json,
        }
    }
}

impl TryFrom<RecordedStatus> for Status {
    type Error = String;

    fn try_from(recorded: RecordedStatus) -> Result<Status, String> {
        Status::try_from(RawStatus {
            args: recorded.args,
            exit: recorded.exit.map(|pair| ExitPair {
                present: pair.present,
                absent: pair.absent,
            }),
            json: recorded.json,
        })
    }
}

impl From<&KeepBinary> for RecordedKeepBinary {
    fn from(spec: &KeepBinary) -> RecordedKeepBinary {
        RecordedKeepBinary {
            args: spec.args.clone(),
            state: spec.state.clone(),
            absent: spec.absent.clone(),
            shims: spec.shims.clone(),
            binary: spec.binary.clone(),
            warning: spec.warning.clone(),
        }
    }
}

impl From<RecordedKeepBinary> for KeepBinary {
    fn from(recorded: RecordedKeepBinary) -> KeepBinary {
        KeepBinary {
            args: recorded.args,
            state: recorded.state,
            absent: recorded.absent,
            shims: recorded.shims,
            binary: recorded.binary,
            warning: recorded.warning,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Kind {
    BinaryPlaced {
        path: String,
        sha256: String,
        /// The part's `keep_binary` at install time, so uninstall can ask it
        /// without the manifest.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        keep_binary: Option<Box<RecordedKeepBinary>>,
    },
    BinaryAdopted {
        path: String,
    },
    ToolInstall {
        argv: Vec<String>,
        undo_argv: Vec<String>,
        /// The level's `status` at install time, so uninstall can confirm the
        /// undo without the manifest (an entry from grh 0.1.x has none).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        status: Option<RecordedStatus>,
    },
    ToolInstallAdopted,
    File {
        path: String,
        sha256: String,
    },
    Backup {
        path: String,
        from: String,
    },
    /// An extension's `[[doctor]]` command, run by `grh status`.
    Doctor {
        command: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub part: String,
    pub level: Level,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    pub kind: Kind,
    pub state: State,
}

impl Entry {
    /// A new machine-level entry, pending.
    pub fn machine(part: &str, kind: Kind) -> Entry {
        Entry {
            part: part.to_string(),
            level: Level::Machine,
            repo: None,
            kind,
            state: State::Pending,
        }
    }

    /// A new repo-level entry for the repository at absolute path `repo`.
    pub fn in_repo(part: &str, repo: &Path, kind: Kind) -> Entry {
        Entry {
            part: part.to_string(),
            level: Level::Repo,
            repo: Some(repo.to_string_lossy().into_owned()),
            kind,
            state: State::Pending,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Receipt {
    pub version: u32,
    pub entries: Vec<Entry>,
}

impl Receipt {
    /// Rename entries whose part `rename` maps to another name, in memory:
    /// a receipt written under an alias is read under the part's own name.
    pub fn rename_parts(&mut self, rename: impl Fn(&str) -> Option<String>) {
        for entry in &mut self.entries {
            if let Some(canonical) = rename(&entry.part) {
                entry.part = canonical;
            }
        }
    }
}

impl Default for Receipt {
    fn default() -> Self {
        Receipt {
            version: VERSION,
            entries: Vec::new(),
        }
    }
}

/// `$XDG_STATE_HOME/grh`, else `~/.local/state/grh`.
pub fn state_dir() -> Result<PathBuf, Exit> {
    if let Some(d) = std::env::var_os("XDG_STATE_HOME").filter(|d| !d.is_empty()) {
        return Ok(PathBuf::from(d).join("grh"));
    }
    match std::env::var_os("HOME").filter(|h| !h.is_empty()) {
        Some(h) => Ok(PathBuf::from(h).join(".local/state/grh")),
        None => Err(Exit::Failed(
            "cannot find the state directory: set HOME or XDG_STATE_HOME".into(),
        )),
    }
}

/// Write `bytes` to `path` through a temp file in the same directory, fsync,
/// then rename. The file ends with `mode`.
pub fn write_atomic(path: &Path, bytes: &[u8], mode: u32) -> Result<(), Exit> {
    let fail =
        |what: &str, e: std::io::Error| Exit::Failed(format!("{what} {}: {e}", path.display()));
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(dir).map_err(|e| fail("cannot create the directory of", e))?;
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned());
    let tmp = dir.join(format!(
        ".{}.grh-tmp-{}",
        name.unwrap_or_default(),
        std::process::id()
    ));
    let written = (|| -> std::io::Result<()> {
        let mut f = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(mode)
            .open(&tmp)?;
        // The umask may have narrowed the mode at creation.
        use std::os::unix::fs::PermissionsExt;
        f.set_permissions(fs::Permissions::from_mode(mode))?;
        f.write_all(bytes)?;
        f.sync_all()?;
        fs::rename(&tmp, path)
    })();
    if let Err(e) = written {
        let _ = fs::remove_file(&tmp); // best-effort cleanup; the write error is what we report
        return Err(fail("cannot write", e));
    }
    if let Ok(d) = File::open(dir) {
        let _ = d.sync_all(); // directory fsync is best-effort; the rename already happened
    }
    Ok(())
}

/// The receipt, held under the exclusive lock until dropped.
#[derive(Debug)]
pub struct Store {
    dir: PathBuf,
    // Held for its lock; released when the file closes.
    _lock: File,
    pub receipt: Receipt,
}

impl Store {
    /// Lock and load the receipt in the default state directory.
    pub fn open() -> Result<Store, Exit> {
        Store::open_in(&state_dir()?)
    }

    /// Lock and load the receipt in `dir` (the `grh` state directory).
    pub fn open_in(dir: &Path) -> Result<Store, Exit> {
        fs::create_dir_all(dir)
            .map_err(|e| Exit::Failed(format!("cannot create {}: {e}", dir.display())))?;
        let lock_path = dir.join("receipt.lock");
        let mut lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock_path)
            .map_err(|e| Exit::Failed(format!("cannot open {}: {e}", lock_path.display())))?;
        match lock.try_lock() {
            Ok(()) => {}
            Err(std::fs::TryLockError::WouldBlock) => {
                return Err(Exit::Locked(holder_message(&lock_path)));
            }
            Err(std::fs::TryLockError::Error(e)) => {
                return Err(Exit::Failed(format!(
                    "cannot lock {}: {e}",
                    lock_path.display()
                )));
            }
        }
        let pid = format!("{}\n", std::process::id());
        lock.set_len(0)
            .and_then(|()| lock.rewind())
            .and_then(|()| lock.write_all(pid.as_bytes()))
            .and_then(|()| lock.sync_all())
            .map_err(|e| Exit::Failed(format!("cannot write {}: {e}", lock_path.display())))?;
        let store = Store {
            dir: dir.to_path_buf(),
            _lock: lock,
            receipt: Receipt::default(),
        };
        let receipt = store.load()?;
        Ok(Store { receipt, ..store })
    }

    fn path(&self) -> PathBuf {
        self.dir.join("receipt.json")
    }

    /// The `grh` state directory this store lives in.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn load(&self) -> Result<Receipt, Exit> {
        read_in(&self.dir)
    }

    /// Write the receipt: temp file, fsync, rename.
    pub fn save(&self) -> Result<(), Exit> {
        let mut text = serde_json::to_string_pretty(&self.receipt)
            .map_err(|e| Exit::Failed(format!("cannot encode the receipt: {e}")))?;
        text.push('\n');
        write_atomic(&self.path(), text.as_bytes(), 0o644)
    }

    /// Record `entry` as pending, saved before its step runs. Returns its index.
    pub fn begin(&mut self, mut entry: Entry) -> Result<usize, Exit> {
        entry.state = State::Pending;
        self.receipt.entries.push(entry);
        self.save()?;
        Ok(self.receipt.entries.len() - 1)
    }

    /// Set the entry `begin` returned to done, after its step ran.
    pub fn done(&mut self, index: usize) -> Result<(), Exit> {
        if let Some(e) = self.receipt.entries.get_mut(index) {
            e.state = State::Done;
        }
        self.save()
    }

    /// Drop the entries `drop` selects, and save.
    pub fn remove_where(&mut self, drop: impl Fn(&Entry) -> bool) -> Result<(), Exit> {
        self.receipt.entries.retain(|e| !drop(e));
        self.save()
    }

    /// The entries still pending: steps a crash left unfinished.
    pub fn pending(&self) -> Vec<&Entry> {
        self.receipt
            .entries
            .iter()
            .filter(|e| e.state == State::Pending)
            .collect()
    }
}

/// Read the receipt in the `grh` state directory `dir` without locking or
/// creating anything (status and plans); a missing receipt is empty.
pub fn read_in(dir: &Path) -> Result<Receipt, Exit> {
    let path = dir.join("receipt.json");
    let text = match fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Receipt::default()),
        Err(e) => return Err(Exit::Failed(format!("cannot read {}: {e}", path.display()))),
    };
    let receipt: Receipt = serde_json::from_str(&text)
        .map_err(|e| Exit::Failed(format!("{} is not a receipt: {e}", path.display())))?;
    if receipt.version > VERSION {
        return Err(Exit::Failed(format!(
            "{} is version {}, newer than this grh understands ({VERSION}): upgrade grh",
            path.display(),
            receipt.version
        )));
    }
    Ok(receipt)
}

fn holder_message(lock_path: &Path) -> String {
    // The holder writes its pid just after taking the lock; wait for it.
    for _ in 0..50 {
        let mut text = String::new();
        if let Ok(mut f) = File::open(lock_path) {
            let _ = f.read_to_string(&mut text); // an unreadable lock file just means no pid yet; retry
        }
        if let Ok(pid) = text.trim().parse::<u32>() {
            return format!(
                "another grh (pid {pid}) holds the receipt lock; wait for it to finish"
            );
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    "another grh holds the receipt lock (pid unknown); wait for it to finish".into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file_entry(n: &str) -> Entry {
        Entry::machine(
            "workflow",
            Kind::File {
                path: format!("/x/{n}"),
                sha256: "ab".into(),
            },
        )
    }

    #[test]
    fn round_trip_keeps_every_kind() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("grh");
        let kinds = vec![
            Kind::BinaryPlaced {
                path: "/b/aval".into(),
                sha256: "01".into(),
                keep_binary: None,
            },
            Kind::BinaryAdopted {
                path: "/b/amont".into(),
            },
            Kind::ToolInstall {
                argv: vec!["aval".into(), "mcp".into()],
                undo_argv: vec!["aval".into(), "unmcp".into()],
                status: Some(RecordedStatus {
                    args: vec!["list".into()],
                    exit: None,
                    json: Some(vec!["state".into()]),
                }),
            },
            Kind::ToolInstallAdopted,
            Kind::File {
                path: "/f".into(),
                sha256: "02".into(),
            },
            Kind::Backup {
                path: "/bk/f".into(),
                from: "/f".into(),
            },
        ];
        {
            let mut s = Store::open_in(&dir).unwrap();
            for (i, k) in kinds.iter().enumerate() {
                let e = if i % 2 == 0 {
                    Entry::machine("decisions", k.clone())
                } else {
                    Entry::in_repo("gate", Path::new("/repo"), k.clone())
                };
                let idx = s.begin(e).unwrap();
                s.done(idx).unwrap();
            }
        }
        let s = Store::open_in(&dir).unwrap();
        assert_eq!(s.receipt.version, VERSION);
        assert_eq!(s.receipt.entries.len(), kinds.len());
        for (e, k) in s.receipt.entries.iter().zip(&kinds) {
            assert_eq!(&e.kind, k);
            assert_eq!(e.state, State::Done);
        }
        assert_eq!(s.receipt.entries[1].repo.as_deref(), Some("/repo"));
        assert_eq!(s.receipt.entries[1].level, Level::Repo);
        let json = fs::read_to_string(dir.join("receipt.json")).unwrap();
        assert!(json.contains("\"version\""));
    }

    #[test]
    fn a_pending_entry_survives_a_crash() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("grh");
        {
            let mut s = Store::open_in(&dir).unwrap();
            let a = s.begin(file_entry("a")).unwrap();
            s.done(a).unwrap();
            s.begin(file_entry("b")).unwrap();
            // Dropped without done: the process died mid-step.
        }
        let s = Store::open_in(&dir).unwrap();
        assert_eq!(s.receipt.entries.len(), 2);
        let pending = s.pending();
        assert_eq!(pending.len(), 1);
        assert!(matches!(&pending[0].kind, Kind::File { path, .. } if path == "/x/b"));
    }

    #[test]
    fn a_second_holder_gets_locked_with_the_pid() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("grh");
        let first = Store::open_in(&dir).unwrap();
        let d2 = dir.clone();
        let second = std::thread::spawn(move || Store::open_in(&d2).map(|_| ()))
            .join()
            .unwrap();
        match second {
            Err(Exit::Locked(m)) => {
                assert!(m.contains(&std::process::id().to_string()), "{m}");
            }
            other => panic!("expected Locked, got {other:?}"),
        }
        drop(first);
        assert!(Store::open_in(&dir).is_ok());
    }

    #[test]
    fn write_atomic_leaves_no_temp_file_and_sets_the_mode() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("sub/x");
        write_atomic(&p, b"one", 0o755).unwrap();
        write_atomic(&p, b"two", 0o755).unwrap();
        assert_eq!(fs::read(&p).unwrap(), b"two");
        assert_eq!(
            fs::metadata(&p).unwrap().permissions().mode() & 0o777,
            0o755
        );
        assert_eq!(fs::read_dir(p.parent().unwrap()).unwrap().count(), 1);
    }

    #[test]
    fn an_entry_with_unknown_keys_inside_a_recorded_query_still_reads() {
        let text = r#"{"part":"p","level":"machine","state":"done","kind":{"tool-install":{
            "argv":["t"],"undo_argv":["u"],
            "status":{"args":["s"],"exit":{"present":0,"absent":1,"later":true},"later":1}}}}"#;
        let entry: Entry = serde_json::from_str(text).unwrap();
        let Kind::ToolInstall { status, .. } = entry.kind else {
            panic!("not a tool install");
        };
        let status = Status::try_from(status.unwrap()).unwrap();
        assert_eq!(status.args, vec!["s".to_string()]);

        let text = r#"{"part":"p","level":"machine","state":"done","kind":{"binary-placed":{
            "path":"/b","sha256":"01",
            "keep_binary":{"args":["a"],"state":["s"],"absent":"x","shims":["h"],"binary":"b","warning":"w","later":2}}}}"#;
        let entry: Entry = serde_json::from_str(text).unwrap();
        let Kind::BinaryPlaced { keep_binary, .. } = entry.kind else {
            panic!("not a placed binary");
        };
        assert_eq!(KeepBinary::from(*keep_binary.unwrap()).absent, "x");
    }
}
