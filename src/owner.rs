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
        let _ = out.read_to_string(&mut text);
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
                let _ = child.kill();
                let _ = child.wait();
                let _ = reader.join();
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
