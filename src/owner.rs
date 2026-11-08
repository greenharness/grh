//! What each tool says about its own integration, read ONLY through the
//! query its manifest declares (`status`, `keep_binary`). A missing
//! binary, a crash, a timeout or an answer that does not parse is `Unknown`,
//! and `Unknown` never authorises a change.

use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::manifest::{KeepBinary, Reading, Status, ToolLevel};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer {
    Present,
    Absent,
    Partial,
    Unknown,
}

impl Answer {
    pub fn as_str(self) -> &'static str {
        match self {
            Answer::Present => "present",
            Answer::Absent => "absent",
            Answer::Partial => "partial",
            Answer::Unknown => "unknown",
        }
    }
}

const DEFAULT_TIMEOUT_SECS: u64 = 30;

fn timeout() -> Duration {
    let secs = std::env::var("GRH_QUERY_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(DEFAULT_TIMEOUT_SECS);
    Duration::from_secs(secs)
}

/// Run `bin args` (in `cwd`) and return its exit code and stdout. `None` when
/// it cannot start, is killed by a signal, or runs past the timeout.
fn ask(bin: &Path, args: &[&str], cwd: Option<&Path>) -> Option<(i32, String)> {
    let mut cmd = Command::new(bin);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    if let Some(d) = cwd {
        cmd.current_dir(d);
    }
    let mut child = cmd.spawn().ok()?;
    let mut out = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut text = String::new();
        let _ = out.read_to_string(&mut text); // a partial read is still the answer; the exit code decides
        text
    });
    let deadline = Instant::now() + timeout();
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(10));
            }
            _ => {
                let _ = child.kill(); // already exited is fine; we are abandoning it
                let _ = child.wait(); // reaping only; the query is already Unknown
                let _ = reader.join(); // the text is discarded on a timeout
                return None;
            }
        }
    };
    let text = reader.join().ok()?;
    Some((status.code()?, text))
}

/// The state string a tool reports, mapped to an `Answer`.
pub fn parse_state(word: &str) -> Answer {
    match word.trim().to_ascii_lowercase().as_str() {
        "present" | "installed" => Answer::Present,
        "absent" | "missing" | "none" | "not-installed" => Answer::Absent,
        "partial" => Answer::Partial,
        _ => Answer::Unknown,
    }
}

/// Read `path` (for example `["integration", "state"]`) out of a JSON document.
pub fn json_state(text: &str, path: &[&str]) -> Answer {
    let Ok(mut v) = serde_json::from_str::<serde_json::Value>(text) else {
        return Answer::Unknown;
    };
    for key in path {
        match v.get_mut(*key) {
            Some(next) => v = next.take(),
            None => return Answer::Unknown,
        }
    }
    v.as_str().map_or(Answer::Unknown, parse_state)
}

fn exit_pair(bin: &Path, args: &[&str], cwd: Option<&Path>, present: i32, absent: i32) -> Answer {
    match ask(bin, args, cwd) {
        Some((c, _)) if c == present => Answer::Present,
        Some((c, _)) if c == absent => Answer::Absent,
        _ => Answer::Unknown,
    }
}

fn json_query(bin: &Path, args: &[&str], cwd: Option<&Path>, path: &[&str]) -> Answer {
    ask(bin, args, cwd).map_or(Answer::Unknown, |(_, text)| json_state(&text, path))
}

/// Run a manifest's `status` query on `bin` (in `cwd`) and read the answer
/// as its manifest says; a failure to run or parse is `Unknown`.
pub fn run_status(status: &Status, bin: &Path, cwd: Option<&Path>) -> Answer {
    let args: Vec<&str> = status.args.iter().map(String::as_str).collect();
    match &status.reading {
        Reading::Exit(pair) => exit_pair(bin, &args, cwd, pair.present, pair.absent),
        Reading::Json(path) => {
            let path: Vec<&str> = path.iter().map(String::as_str).collect();
            json_query(bin, &args, cwd, &path)
        }
    }
}

/// The state of one level of a tool's integration, by its declared `status`;
/// a level that declares none is `Unknown`.
pub fn level_state(level: &ToolLevel, bin: &Path, cwd: Option<&Path>) -> Answer {
    level
        .status
        .as_ref()
        .map_or(Answer::Unknown, |s| run_status(s, bin, cwd))
}

/// What a manifest's `keep_binary` query says about a placed binary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Template {
    /// Safe to remove the binary.
    Gone,
    /// Keep the binary, for this reason.
    Keep(String),
}

fn json_at<'a>(v: &'a serde_json::Value, path: &[String]) -> Option<&'a serde_json::Value> {
    path.iter().try_fold(v, |at, key| at.get(key))
}

/// Run the `keep_binary` query of the part whose binary is `bin_name`, and
/// decide whether the placed binary at `placed` may go. Anything but a clear
/// answer keeps the binary.
pub fn keep_binary(spec: &KeepBinary, bin_name: &str, bin: &Path, placed: &str) -> Template {
    let args: Vec<&str> = spec.args.iter().map(String::as_str).collect();
    let mut argv = vec![bin_name.to_string()];
    argv.extend(spec.args.iter().cloned());
    let command = crate::ops::command_line(&argv);
    let Some((0, text)) = ask(bin, &args, None) else {
        return Template::Keep(format!("`{command}` failed"));
    };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else {
        return Template::Keep(format!("`{command}` did not answer JSON"));
    };
    let unknown = || Template::Keep(format!("{bin_name}'s answer about its copies is unknown"));
    let state = json_at(&v, &spec.state)
        .and_then(serde_json::Value::as_str)
        .map(str::to_ascii_lowercase);
    let absent = spec.absent.to_ascii_lowercase();
    match state.as_deref() {
        Some(word) if word == absent => Template::Gone,
        Some(word) if matches!(parse_state(word), Answer::Present | Answer::Partial) => {
            let placed_real = std::fs::canonicalize(placed).ok();
            let same = |b: &str| {
                b == placed
                    || std::fs::canonicalize(b)
                        .ok()
                        .is_some_and(|c| placed_real.as_ref() == Some(&c))
            };
            // Every shim must name its binary; anything less is an answer
            // grh cannot read, and an unreadable answer never authorises a
            // removal.
            let Some(binaries) = json_at(&v, &spec.shims)
                .and_then(serde_json::Value::as_array)
                .and_then(|shims| {
                    shims
                        .iter()
                        .map(|s| s.get(&spec.binary).and_then(serde_json::Value::as_str))
                        .collect::<Option<Vec<_>>>()
                })
            else {
                return unknown();
            };
            // Present with no shims contradicts itself: unknown, not absent.
            if binaries.is_empty() {
                return unknown();
            }
            if binaries.into_iter().any(same) {
                Template::Keep(format!("{bin_name}'s copies bake it"))
            } else {
                Template::Gone
            }
        }
        _ => unknown(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_states_map_and_anything_else_is_unknown() {
        let p = ["integration", "state"];
        assert_eq!(
            json_state(r#"{"integration":{"state":"present"}}"#, &p),
            Answer::Present
        );
        assert_eq!(
            json_state(r#"{"integration":{"state":"absent"}}"#, &p),
            Answer::Absent
        );
        assert_eq!(
            json_state(r#"{"integration":{"state":"partial"}}"#, &p),
            Answer::Partial
        );
        assert_eq!(json_state(r#"{"integration":{}}"#, &p), Answer::Unknown);
        assert_eq!(json_state("not json", &p), Answer::Unknown);
        assert_eq!(
            json_state(r#"{"integration":{"state":3}}"#, &p),
            Answer::Unknown
        );
    }

    #[test]
    fn a_missing_binary_is_unknown() {
        let nope = Path::new("/nonexistent/grh-test-bin");
        let status = Status {
            args: vec!["list".into()],
            reading: Reading::Json(vec!["state".into()]),
        };
        assert_eq!(run_status(&status, nope, None), Answer::Unknown);
    }

    fn spec() -> KeepBinary {
        KeepBinary {
            args: vec!["list".into()],
            state: vec!["state".into()],
            absent: "absent".into(),
            shims: vec!["shims".into()],
            binary: "binary".into(),
            warning: String::new(),
        }
    }

    /// A script that prints `stdout` and exits `code`.
    fn fake(dir: &Path, code: i32, stdout: &str) -> std::path::PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join("tool");
        std::fs::write(
            &path,
            format!("#!/bin/sh\ncat <<'EOF'\n{stdout}\nEOF\nexit {code}\n"),
        )
        .unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    fn keep_with(code: i32, stdout: &str, placed: &str) -> Template {
        let dir = tempfile::tempdir().unwrap();
        let bin = fake(dir.path(), code, stdout);
        keep_binary(&spec(), "tool", &bin, placed)
    }

    fn is_keep(t: &Template, needle: &str) -> bool {
        matches!(t, Template::Keep(why) if why.contains(needle))
    }

    #[test]
    fn keep_binary_keeps_when_the_query_exits_non_zero() {
        let t = keep_with(3, r#"{"state":"absent"}"#, "/x/tool");
        assert!(is_keep(&t, "failed"), "{t:?}");
    }

    #[test]
    fn keep_binary_keeps_when_the_answer_is_not_json() {
        let t = keep_with(0, "not json", "/x/tool");
        assert!(is_keep(&t, "did not answer JSON"), "{t:?}");
    }

    #[test]
    fn keep_binary_keeps_on_an_unknown_state() {
        let t = keep_with(0, r#"{"state":"weird"}"#, "/x/tool");
        assert!(is_keep(&t, "unknown"), "{t:?}");
    }

    #[test]
    fn keep_binary_keeps_when_present_with_no_shims() {
        let t = keep_with(0, r#"{"state":"present","shims":[]}"#, "/x/tool");
        assert!(is_keep(&t, "unknown"), "{t:?}");
    }

    #[test]
    fn keep_binary_reads_installed_as_present_like_every_status_reader() {
        // `parse_state`'s vocabulary: `installed` means present. With no shim
        // baking the placed binary it may go; were `installed` not read as
        // present, the state would be unknown and the binary kept.
        let t = keep_with(
            0,
            r#"{"state":"installed","shims":[{"binary":"/y/tool"}]}"#,
            "/x/tool",
        );
        assert!(matches!(t, Template::Gone), "{t:?}");
    }

    #[test]
    fn keep_binary_keeps_when_a_shim_names_no_binary() {
        let t = keep_with(
            0,
            r#"{"state":"present","shims":[{"other":"/y"}]}"#,
            "/x/tool",
        );
        assert!(is_keep(&t, "unknown"), "{t:?}");
    }

    #[test]
    fn keep_binary_lets_go_when_no_shim_bakes_the_binary() {
        let t = keep_with(
            0,
            r#"{"state":"partial","shims":[{"binary":"/y/tool"}]}"#,
            "/x/tool",
        );
        assert_eq!(t, Template::Gone);
    }

    #[test]
    fn keep_binary_keeps_when_a_shim_matches_through_a_canonical_path() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real");
        std::fs::write(&real, "").unwrap();
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let answer = format!(
            r#"{{"state":"present","shims":[{{"binary":"{}"}}]}}"#,
            link.display()
        );
        let bin_dir = tempfile::tempdir().unwrap();
        let bin = fake(bin_dir.path(), 0, &answer);
        let t = keep_binary(&spec(), "tool", &bin, real.to_str().unwrap());
        assert!(is_keep(&t, "bake it"), "{t:?}");
    }

    #[test]
    fn keep_binary_lets_go_when_the_state_is_absent() {
        let t = keep_with(0, r#"{"state":"absent"}"#, "/x/tool");
        assert_eq!(t, Template::Gone);
    }
}
