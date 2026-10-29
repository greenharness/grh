//! Place a part's skills and agents from the pinned `greenharness/skills`
//! release into the Claude config dir, and take them out again.
//!
//! Every file placed is recorded in the receipt with its sha256. A file grh
//! did not place is never overwritten without `replace` (and then a backup is
//! kept first); a file carrying a `relais:begin` line belongs to relais and is
//! never written or removed; removal deletes only files whose bytes still
//! match the receipt.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Component, Path, PathBuf};

use crate::catalog::{Catalog, Part};
use crate::exit::Exit;
use crate::fetch::{download_verified, read_tarball, release_base, sha256_hex};
use crate::receipt::{write_atomic, Entry, Kind, Store};

const RELAIS_MARK: &str = "relais:begin";

/// `$CLAUDE_CONFIG_DIR`, else `~/.claude`.
pub fn claude_dir() -> Result<PathBuf, Exit> {
    if let Some(d) = std::env::var_os("CLAUDE_CONFIG_DIR").filter(|d| !d.is_empty()) {
        return Ok(PathBuf::from(d));
    }
    match std::env::var_os("HOME").filter(|h| !h.is_empty()) {
        Some(h) => Ok(PathBuf::from(h).join(".claude")),
        None => Err(Exit::Failed(
            "cannot find the Claude config directory: set HOME or CLAUDE_CONFIG_DIR".into(),
        )),
    }
}

/// The files of the skills release, by path relative to the config dir
/// (`skills/<name>/…`, `agents/<name>.md`).
#[derive(Debug, Default)]
pub struct Bundle {
    files: BTreeMap<String, Vec<u8>>,
}

pub fn skills_url(base: &str, repo: &str, version: &str) -> String {
    format!(
        "{}/{repo}/releases/download/v{version}/greenharness-skills-{version}.tar.gz",
        base.trim_end_matches('/')
    )
}

impl Bundle {
    /// A bundle of files by path relative to the config dir.
    pub fn from_files(files: BTreeMap<String, Vec<u8>>) -> Bundle {
        Bundle { files }
    }

    /// Read a tarball whose `skills/` and `agents/` dirs may sit under a wrapper
    /// dir and `parts/<part>/` (the greenharness/skills release layout).
    pub fn from_tarball(bytes: &[u8]) -> Result<Bundle, Exit> {
        let mut files = BTreeMap::new();
        for (path, data, _) in read_tarball(bytes)? {
            let comps: Vec<Component> = Path::new(&path).components().collect();
            if comps
                .iter()
                .any(|c| !matches!(c, Component::Normal(_) | Component::CurDir))
            {
                continue;
            }
            let names: Vec<String> = comps
                .iter()
                .filter_map(|c| match c {
                    Component::Normal(n) => Some(n.to_string_lossy().into_owned()),
                    _ => None,
                })
                .collect();
            let Some(at) = names.iter().position(|n| n == "skills" || n == "agents") else {
                continue;
            };
            if names.len() > at + 1 {
                files.insert(names[at..].join("/"), data);
            }
        }
        Ok(Bundle { files })
    }

    /// Fetch, verify against the pin, and read the release at `base`.
    pub fn fetch_from(base: &str, repo: &str, version: &str, pin: &str) -> Result<Bundle, Exit> {
        let bytes = download_verified(&skills_url(base, repo, version), pin)?;
        Bundle::from_tarball(&bytes)
    }

    /// The release the catalog pins, from `$GRH_RELEASE_BASE` or GitHub.
    pub fn fetch(catalog: &Catalog) -> Result<Bundle, Exit> {
        Bundle::fetch_from(
            &release_base(),
            &catalog.skills_repo,
            &catalog.skills_version,
            &catalog.skills_sha256,
        )
    }

    /// The (relative path, bytes) of everything `part` places.
    fn files_for(&self, part: &Part) -> Result<Vec<(&str, &[u8])>, Exit> {
        let mut out = Vec::new();
        for name in &part.skills {
            let prefix = format!("skills/{name}/");
            let before = out.len();
            for (p, d) in self.files.range(prefix.clone()..) {
                if !p.starts_with(&prefix) {
                    break;
                }
                out.push((p.as_str(), d.as_slice()));
            }
            if out.len() == before {
                return Err(Exit::Failed(format!(
                    "the skills release has no skills/{name}/"
                )));
            }
        }
        for name in &part.agents {
            let p = format!("agents/{name}.md");
            match self.files.get_key_value(&p) {
                Some((k, d)) => out.push((k.as_str(), d.as_slice())),
                None => {
                    return Err(Exit::Failed(format!(
                        "the skills release has no agents/{name}.md"
                    )))
                }
            }
        }
        Ok(out)
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Skipped {
    pub path: PathBuf,
    pub reason: String,
}

#[derive(Debug, Default)]
pub struct Placement {
    /// Files written or adopted, now in the receipt.
    pub placed: Vec<PathBuf>,
    /// Files left alone, with why.
    pub skipped: Vec<Skipped>,
    /// (backup path, original path) for each file `replace` overwrote.
    pub backups: Vec<(PathBuf, PathBuf)>,
}

#[derive(Debug, Default)]
pub struct Removal {
    pub removed: Vec<PathBuf>,
    /// Files kept, with why (changed since placed, or relais's).
    pub kept: Vec<Skipped>,
}

fn is_relais(bytes: &[u8]) -> bool {
    String::from_utf8_lossy(bytes)
        .lines()
        .any(|l| l.contains(RELAIS_MARK))
}

fn read_existing(path: &Path) -> Result<Option<Vec<u8>>, Exit> {
    match fs::read(path) {
        Ok(b) => Ok(Some(b)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(Exit::Failed(format!("cannot read {}: {e}", path.display()))),
    }
}

fn recorded_sha(store: &Store, path: &Path) -> Option<String> {
    let want = path.to_string_lossy();
    store.receipt.entries.iter().find_map(|e| match &e.kind {
        Kind::File { path, sha256 } if *path == *want => Some(sha256.clone()),
        _ => None,
    })
}

/// Record `path` with `sha` for `part`: pending, then `act`, then done.
fn recorded_step(
    store: &mut Store,
    part: &str,
    path: &Path,
    sha: &str,
    act: impl FnOnce() -> Result<(), Exit>,
) -> Result<(), Exit> {
    let want = path.to_string_lossy().into_owned();
    store.remove_where(|e| matches!(&e.kind, Kind::File { path, .. } if *path == want))?;
    let idx = store.begin(Entry::machine(
        part,
        Kind::File {
            path: want,
            sha256: sha.to_string(),
        },
    ))?;
    act()?;
    store.done(idx)
}

/// Place what `part` lists into `claude` (the config dir). The receipt entry
/// is written pending before each file and set done after.
pub fn place(
    bundle: &Bundle,
    part: &Part,
    claude: &Path,
    store: &mut Store,
    replace: bool,
) -> Result<Placement, Exit> {
    let files = bundle.files_for(part)?;
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let backup_root = store.dir().join("backup").join(ts.to_string());
    let mut out = Placement::default();
    for (rel, data) in files {
        let dest = claude.join(rel);
        let new_sha = sha256_hex(data);
        let mut write = true;
        if let Some(cur) = read_existing(&dest)? {
            if is_relais(&cur) {
                out.skipped.push(Skipped {
                    path: dest,
                    reason: "belongs to relais".into(),
                });
                continue;
            }
            let cur_sha = sha256_hex(&cur);
            let ours = recorded_sha(store, &dest).is_some_and(|r| r == cur_sha);
            if cur_sha == new_sha {
                write = false;
            } else if !ours {
                if !replace {
                    out.skipped.push(Skipped {
                        path: dest,
                        reason: "exists and grh did not place it; use --replace".into(),
                    });
                    continue;
                }
                let to = backup_root.join(rel);
                let idx = store.begin(Entry::machine(
                    &part.name,
                    Kind::Backup {
                        path: to.to_string_lossy().into_owned(),
                        from: dest.to_string_lossy().into_owned(),
                    },
                ))?;
                write_atomic(&to, &cur, 0o644)?;
                store.done(idx)?;
                out.backups.push((to, dest.clone()));
            }
        }
        recorded_step(store, &part.name, &dest, &new_sha, || {
            if write {
                write_atomic(&dest, data, 0o644)
            } else {
                Ok(())
            }
        })?;
        out.placed.push(dest);
    }
    Ok(out)
}

/// Remove the files the receipt lists for `part`: only those whose sha still
/// matches; changed and relais-owned files stay and are reported.
pub fn remove(part: &str, store: &mut Store) -> Result<Removal, Exit> {
    let mine: Vec<(String, String)> = store
        .receipt
        .entries
        .iter()
        .filter(|e| e.part == part)
        .filter_map(|e| match &e.kind {
            Kind::File { path, sha256 } => Some((path.clone(), sha256.clone())),
            _ => None,
        })
        .collect();
    let mut out = Removal::default();
    for (path, sha) in &mine {
        let p = PathBuf::from(path);
        match read_existing(&p)? {
            None => {}
            Some(cur) if is_relais(&cur) => out.kept.push(Skipped {
                path: p,
                reason: "belongs to relais".into(),
            }),
            Some(cur) if sha256_hex(&cur) == *sha => {
                fs::remove_file(&p)
                    .map_err(|e| Exit::Failed(format!("cannot remove {}: {e}", p.display())))?;
                prune_empty_dirs(&p);
                out.removed.push(p);
            }
            Some(_) => out.kept.push(Skipped {
                path: p,
                reason: "changed since grh placed it".into(),
            }),
        }
    }
    let names: Vec<&str> = mine.iter().map(|(p, _)| p.as_str()).collect();
    store.remove_where(|e| {
        e.part == part
            && matches!(&e.kind, Kind::File { path, .. } if names.contains(&path.as_str()))
    })?;
    Ok(out)
}

/// Remove directories a removed file left empty, up to `skills`/`agents`.
fn prune_empty_dirs(file: &Path) {
    let mut dir = file.parent();
    while let Some(d) = dir {
        let name = d.file_name().map(|n| n.to_string_lossy().into_owned());
        if matches!(name.as_deref(), Some("skills") | Some("agents") | None) {
            break;
        }
        if fs::remove_dir(d).is_err() {
            break;
        }
        dir = d.parent();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Env {
        _tmp: tempfile::TempDir,
        claude: PathBuf,
        store: Store,
        bundle: Bundle,
        part: Part,
    }

    fn tarball(files: &[(&str, &[u8])]) -> Vec<u8> {
        let gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        let mut b = tar::Builder::new(gz);
        for (name, data) in files {
            let mut h = tar::Header::new_gnu();
            h.set_size(data.len() as u64);
            h.set_mode(0o644);
            h.set_cksum();
            b.append_data(&mut h, name, *data).unwrap();
        }
        b.into_inner().unwrap().finish().unwrap()
    }

    fn env() -> Env {
        let tmp = tempfile::tempdir().unwrap();
        let claude = tmp.path().join("claude");
        // Never the real HOME: everything lives under the tempdir. Tests do
        // not read these variables back; the paths are passed explicitly.
        std::env::set_var("HOME", tmp.path());
        std::env::set_var("XDG_STATE_HOME", tmp.path().join("state"));
        std::env::set_var("CLAUDE_CONFIG_DIR", &claude);
        let tgz = tarball(&[
            ("skills-0.1.0/skills/alpha/SKILL.md", b"alpha v1"),
            ("skills-0.1.0/skills/alpha/ref/more.md", b"more"),
            ("skills-0.1.0/skills/beta/SKILL.md", b"beta v1"),
            ("skills-0.1.0/agents/rev.md", b"agent v1"),
            ("skills-0.1.0/README.md", b"ignored"),
        ]);
        let part: Part =
            toml::from_str("name='p'\nsummary='s'\nneeds=[]\nskills=['alpha']\nagents=['rev']\n")
                .unwrap();
        Env {
            store: Store::open_in(&tmp.path().join("state/grh")).unwrap(),
            bundle: Bundle::from_tarball(&tgz).unwrap(),
            claude,
            part,
            _tmp: tmp,
        }
    }

    #[test]
    fn placement_records_sha256s_and_removal_deletes_them() {
        let mut e = env();
        let r = place(&e.bundle, &e.part, &e.claude, &mut e.store, false).unwrap();
        assert_eq!(r.placed.len(), 3);
        assert!(r.skipped.is_empty());
        assert_eq!(
            fs::read(e.claude.join("skills/alpha/SKILL.md")).unwrap(),
            b"alpha v1"
        );
        assert!(!e.claude.join("skills/beta").exists());
        let shas: Vec<String> = e
            .store
            .receipt
            .entries
            .iter()
            .filter_map(|x| match &x.kind {
                Kind::File { path, sha256 } if path.ends_with("agents/rev.md") => {
                    Some(sha256.clone())
                }
                _ => None,
            })
            .collect();
        assert_eq!(shas, vec![sha256_hex(b"agent v1")]);
        assert!(e.store.pending().is_empty());

        let rm = remove("p", &mut e.store).unwrap();
        assert_eq!(rm.removed.len(), 3);
        assert!(rm.kept.is_empty());
        assert!(!e.claude.join("skills/alpha").exists());
        assert!(e.store.receipt.entries.is_empty());
    }

    #[test]
    fn a_foreign_file_is_skipped_then_backed_up_with_replace() {
        let mut e = env();
        let f = e.claude.join("skills/alpha/SKILL.md");
        fs::create_dir_all(f.parent().unwrap()).unwrap();
        fs::write(&f, b"mine").unwrap();

        let r = place(&e.bundle, &e.part, &e.claude, &mut e.store, false).unwrap();
        assert_eq!(r.skipped.len(), 1);
        assert_eq!(r.skipped[0].path, f);
        assert_eq!(fs::read(&f).unwrap(), b"mine");
        assert_eq!(r.placed.len(), 2);

        let r = place(&e.bundle, &e.part, &e.claude, &mut e.store, true).unwrap();
        assert!(r.skipped.is_empty());
        assert_eq!(r.backups.len(), 1);
        let (backup, from) = &r.backups[0];
        assert_eq!(from, &f);
        assert!(backup.starts_with(e.store.dir().join("backup")));
        assert_eq!(fs::read(backup).unwrap(), b"mine");
        assert_eq!(fs::read(&f).unwrap(), b"alpha v1");
    }

    #[test]
    fn a_changed_file_is_kept_on_removal() {
        let mut e = env();
        place(&e.bundle, &e.part, &e.claude, &mut e.store, false).unwrap();
        let f = e.claude.join("agents/rev.md");
        fs::write(&f, b"edited").unwrap();
        let rm = remove("p", &mut e.store).unwrap();
        assert_eq!(rm.removed.len(), 2);
        assert_eq!(rm.kept.len(), 1);
        assert_eq!(rm.kept[0].path, f);
        assert_eq!(fs::read(&f).unwrap(), b"edited");
    }

    #[test]
    fn a_relais_file_is_never_written_or_removed() {
        let mut e = env();
        let f = e.claude.join("skills/alpha/SKILL.md");
        fs::create_dir_all(f.parent().unwrap()).unwrap();
        let theirs = b"x\n<!-- relais:begin -->\nrelais\n";
        fs::write(&f, theirs).unwrap();

        for replace in [false, true] {
            let r = place(&e.bundle, &e.part, &e.claude, &mut e.store, replace).unwrap();
            assert!(r.skipped.iter().any(|s| s.path == f));
            assert!(r.backups.is_empty());
            assert_eq!(fs::read(&f).unwrap(), theirs);
        }

        // Even if the receipt wrongly lists it with a matching sha.
        let idx = e
            .store
            .begin(Entry::machine(
                "p",
                Kind::File {
                    path: f.to_string_lossy().into_owned(),
                    sha256: sha256_hex(theirs),
                },
            ))
            .unwrap();
        e.store.done(idx).unwrap();
        let rm = remove("p", &mut e.store).unwrap();
        assert!(rm.kept.iter().any(|s| s.path == f));
        assert_eq!(fs::read(&f).unwrap(), theirs);
    }

    #[test]
    fn the_release_layout_with_parts_is_read() {
        // greenharness/skills ships `greenharness-skills-<v>/parts/<part>/…`.
        let tgz = tarball(&[
            (
                "greenharness-skills-0.1.0/parts/gate/skills/tag/SKILL.md",
                b"t",
            ),
            (
                "greenharness-skills-0.1.0/parts/workflow/agents/rev.md",
                b"r",
            ),
            ("greenharness-skills-0.1.0/README.md", b"ignored"),
        ]);
        let b = Bundle::from_tarball(&tgz).unwrap();
        assert!(
            b.files.contains_key("skills/tag/SKILL.md"),
            "{:?}",
            b.files.keys()
        );
        assert!(b.files.contains_key("agents/rev.md"));
        assert_eq!(b.files.len(), 2);
    }

    #[test]
    fn a_pinned_tarball_is_fetched_from_a_local_base() {
        let tmp = tempfile::tempdir().unwrap();
        let tgz = tarball(&[("skills/alpha/SKILL.md", b"a")]);
        let d = tmp.path().join("o/skills/releases/download/v0.1.0");
        fs::create_dir_all(&d).unwrap();
        fs::write(d.join("greenharness-skills-0.1.0.tar.gz"), &tgz).unwrap();
        let base = format!("file://{}", tmp.path().display());
        let b = Bundle::fetch_from(&base, "o/skills", "0.1.0", &sha256_hex(&tgz)).unwrap();
        assert!(b.files.contains_key("skills/alpha/SKILL.md"));
        assert!(Bundle::fetch_from(&base, "o/skills", "0.1.0", &"0".repeat(64)).is_err());
    }
}
