//! What each tool says about its own integration. One function per `query`
//! name in `catalog.toml`, reading ONLY the tool's published query. A missing
//! binary, a crash, a timeout or an answer that does not parse is `Unknown`,
//! and `Unknown` never authorises a change.

use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

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

/// `<aval> mcp install --check`: exit 0 present, 1 absent.
pub fn aval_mcp(bin: &Path) -> Answer {
    exit_pair(bin, &["mcp", "install", "--check"], None, 0, 1)
}

/// `<aval> hook uninstall --check` in the repo: exit 1 present, 0 absent.
pub fn aval_hook(bin: &Path, repo: Option<&Path>) -> Answer {
    exit_pair(bin, &["hook", "uninstall", "--check"], repo, 1, 0)
}

/// `<amont> list --json` in the repo: `installed.state`.
pub fn amont_list(bin: &Path, repo: Option<&Path>) -> Answer {
    json_query(bin, &["list", "--json"], repo, &["installed", "state"])
}

/// What `<amont> list --json` says about the template directory a placed
/// `amont` binary may be baked into.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Template {
    /// Safe to remove the binary; repositories armed by hand with it will fail.
    Gone,
    /// Keep the binary, for this reason.
    Keep(String),
}

/// `<amont> list --json`: `template_dir.state` and `template_dir.shims[].binary`.
/// Anything but a clear answer keeps the binary.
pub fn amont_template(bin: &Path, placed: &str) -> Template {
    let Some((0, text)) = ask(bin, &["list", "--json"], None) else {
        return Template::Keep("`amont list --json` failed".into());
    };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else {
        return Template::Keep("`amont list --json` did not answer JSON".into());
    };
    let dir = &v["template_dir"];
    match dir["state"]
        .as_str()
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("absent") => Template::Gone,
        Some("present" | "partial") => {
            let same = |b: &str| {
                b == placed
                    || std::fs::canonicalize(b)
                        .ok()
                        .is_some_and(|c| std::fs::canonicalize(placed).ok().is_some_and(|p| p == c))
            };
            // Every shim must name its binary; anything less is an answer
            // grh cannot read, and an unreadable answer never authorises a
            // removal.
            let Some(binaries) = dir["shims"].as_array().and_then(|shims| {
                shims
                    .iter()
                    .map(|s| s["binary"].as_str())
                    .collect::<Option<Vec<_>>>()
            }) else {
                return Template::Keep("amont's template directory state is unknown".into());
            };
            if binaries.into_iter().any(same) {
                Template::Keep("amont's template directory shims bake it".into())
            } else {
                Template::Gone
            }
        }
        _ => Template::Keep("amont's template directory state is unknown".into()),
    }
}

/// `<amont-agent> doctor --json`: `integration.state`.
pub fn amont_agent_doctor(bin: &Path) -> Answer {
    json_query(bin, &["doctor", "--json"], None, &["integration", "state"])
}

/// `<relais> doctor --json`: `integration.state` when present, else Unknown.
pub fn relais_doctor(bin: &Path) -> Answer {
    json_query(bin, &["doctor", "--json"], None, &["integration", "state"])
}

/// Run the query a catalog entry names. An unknown name is `Unknown`.
pub fn query(name: &str, bin: &Path, repo: Option<&Path>) -> Answer {
    match name {
        "aval-mcp" => aval_mcp(bin),
        "aval-hook" => aval_hook(bin, repo),
        "amont-list" => amont_list(bin, repo),
        "amont-agent-doctor" => amont_agent_doctor(bin),
        "relais-doctor" => relais_doctor(bin),
        _ => Answer::Unknown,
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
        assert_eq!(aval_mcp(nope), Answer::Unknown);
        assert_eq!(query("amont-list", nope, None), Answer::Unknown);
        assert_eq!(query("no-such-query", nope, None), Answer::Unknown);
    }
}
