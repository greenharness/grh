//! The install checklist through a real pseudo-terminal: the built binary, a
//! fixture HOME and catalog, and a harness that answers the inline viewport's
//! cursor-position query (ESC [ 6 n).

use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::thread::sleep;
use std::time::{Duration, Instant};

use rexpect::session::{spawn_command, PtySession};
use rexpect::ReadUntil;

const CURSOR_QUERY: &str = "\x1b[6n";
const CURSOR_ANSWER: &str = "\x1b[1;1R";
const TIMEOUT_MS: u64 = 20_000;

const CATALOG: &str = r#"
skills_repo = "o/r"
skills_version = "1"
skills_sha256 = "0"
foreign_markers = ["other:begin"]
order = ["alpha", "beta", "lab"]

[[part]]
[part.extension]
name = "alpha"
version = "1.0.0"
harness = ">=0.4"
summary = "the first thing a person runs"
needs = []

[[part]]
[part.extension]
name = "beta"
version = "1.0.0"
harness = ">=0.4"
summary = "a second thing that wants the first"
needs = ["alpha"]

[[part]]
[part.extension]
name = "lab"
version = "1.0.0"
harness = ">=0.4"
summary = "a risky try-out"
needs = []
experimental = true
"#;

struct Fixture {
    _tmp: tempfile::TempDir,
    root: PathBuf,
}

impl Fixture {
    fn new() -> Fixture {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        for dir in ["home", "state", "work"] {
            fs::create_dir_all(root.join(dir)).unwrap();
        }
        fs::write(root.join("catalog.toml"), CATALOG).unwrap();
        Fixture { _tmp: tmp, root }
    }

    fn assert_no_receipt(&self) {
        assert!(
            !self.root.join("state/grh/receipt.json").exists(),
            "the receipt was written"
        );
    }

    /// `grh install` in a pty of `rows` x `cols`, followed by the exit code and
    /// the terminal's settings, all printed by the shell around it.
    fn spawn(&self, rows: u16, cols: u16) -> PtySession {
        let script = format!(
            "echo shell=$$; stty echo rows {rows} cols {cols}; '{}' install; echo rc=$?; stty -a",
            env!("CARGO_BIN_EXE_grh")
        );
        let mut command = Command::new("sh");
        command
            .args(["-c", &script])
            .current_dir(self.root.join("work"))
            .env_clear()
            .env("HOME", self.root.join("home"))
            .env("XDG_STATE_HOME", self.root.join("state"))
            .env("GRH_CATALOG", self.root.join("catalog.toml"))
            .env("PATH", "/usr/bin:/bin:/usr/local/bin:/opt/homebrew/bin")
            .env("TERM", "xterm")
            .env("NO_COLOR", "1");
        spawn_command(command, Some(TIMEOUT_MS)).unwrap()
    }

    /// Spawn at 80x24 and answer the cursor query; the checklist is then drawn.
    /// Returns the session and the pid of its shell.
    fn open(&self) -> (PtySession, String) {
        let mut session = self.spawn(24, 80);
        let shell = shell_pid(&mut session);
        expect(&mut session, "Choose what to install");
        (session, shell)
    }
}

fn shell_pid(session: &mut PtySession) -> String {
    let (_, matched) = session.exp_regex("shell=[0-9]+").unwrap();
    matched.trim_start_matches("shell=").to_string()
}

/// Read until the process ends, answering every cursor-position query on the
/// way: the inline viewport asks when it opens and again when it is cleared.
fn read_to_end(session: &mut PtySession) -> String {
    let mut seen = String::new();
    loop {
        let (before, matched) = session
            .exp_any(vec![ReadUntil::EOF, ReadUntil::String(CURSOR_QUERY.into())])
            .unwrap();
        seen.push_str(&before);
        if matched == CURSOR_QUERY {
            send(session, CURSOR_ANSWER);
        } else {
            seen.push_str(&matched);
            return seen;
        }
    }
}

/// Wait for `needle`, answering every cursor-position query on the way.
fn expect(session: &mut PtySession, needle: &str) {
    let mut seen = String::new();
    loop {
        let (before, matched) = session
            .exp_any(vec![
                ReadUntil::String(needle.into()),
                ReadUntil::String(CURSOR_QUERY.into()),
            ])
            .unwrap();
        seen.push_str(&before);
        if matched == CURSOR_QUERY {
            send(session, CURSOR_ANSWER);
        } else {
            return;
        }
    }
}

fn send(session: &mut PtySession, keys: &str) {
    session.send(keys).unwrap();
    session.flush().unwrap();
}

/// Everything up to the end of the session, which must show the expected exit
/// code and a terminal back in canonical mode with echo.
fn finish(session: &mut PtySession, expected_code: i32) {
    let rest = read_to_end(session);
    assert!(
        rest.contains(&format!("rc={expected_code}")),
        "expected rc={expected_code} in {rest:?}"
    );
    let settings: Vec<&str> = rest
        .split(|c: char| c.is_whitespace() || c == ';')
        .collect();
    assert!(settings.contains(&"icanon"), "not canonical: {rest:?}");
    assert!(settings.contains(&"echo"), "echo is off: {rest:?}");
}

/// grh's own pid: the child of the `sh -c` that runs it.
fn grh_pid(shell: &str) -> String {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let out = Command::new("pgrep").args(["-P", shell]).output().unwrap();
        let text = String::from_utf8_lossy(&out.stdout).into_owned();
        if let Some(pid) = text.split_whitespace().next() {
            return pid.to_string();
        }
        assert!(Instant::now() < deadline, "grh has no pid under the shell");
        sleep(Duration::from_millis(50));
    }
}

fn terminate(pid: &str) {
    let status = Command::new("kill").args(["-TERM", pid]).status().unwrap();
    assert!(status.success());
}

#[test]
fn space_then_enter_prints_the_command_and_the_plan_and_n_changes_nothing() {
    let fixture = Fixture::new();
    let (mut session, _) = fixture.open();
    send(&mut session, " ");
    send(&mut session, "\r");
    expect(&mut session, "grh install alpha\r\n");
    expect(&mut session, "grh will install:");
    expect(&mut session, "Proceed? [y/N]");
    send(&mut session, "n\r");
    expect(&mut session, "nothing changed");
    finish(&mut session, 0);
    fixture.assert_no_receipt();
}

#[test]
fn two_enters_in_one_write_leave_proceed_waiting() {
    let fixture = Fixture::new();
    let (mut session, _) = fixture.open();
    send(&mut session, " \r\r");
    expect(&mut session, "Proceed? [y/N]");
    sleep(Duration::from_millis(500));
    let mut seen = String::new();
    while let Some(character) = session.try_read() {
        seen.push(character);
    }
    assert!(
        !seen.contains("nothing changed"),
        "the second enter answered Proceed?: {seen:?}"
    );
    send(&mut session, "n\r");
    expect(&mut session, "nothing changed");
    finish(&mut session, 0);
    fixture.assert_no_receipt();
}

#[test]
fn q_cancels_with_exit_0() {
    let fixture = Fixture::new();
    let (mut session, _) = fixture.open();
    send(&mut session, "q");
    expect(&mut session, "nothing changed");
    finish(&mut session, 0);
    fixture.assert_no_receipt();
}

#[test]
fn ctrl_c_exits_130() {
    let fixture = Fixture::new();
    let (mut session, _) = fixture.open();
    session.send_control('c').unwrap();
    finish(&mut session, 130);
    fixture.assert_no_receipt();
}

#[test]
fn sigterm_in_the_checklist_exits_143() {
    let fixture = Fixture::new();
    let (mut session, shell) = fixture.open();
    terminate(&grh_pid(&shell));
    finish(&mut session, 143);
    fixture.assert_no_receipt();
}

#[test]
fn sigterm_at_proceed_exits_143() {
    let fixture = Fixture::new();
    let (mut session, shell) = fixture.open();
    send(&mut session, " \r");
    expect(&mut session, "Proceed? [y/N]");
    terminate(&grh_pid(&shell));
    finish(&mut session, 143);
    fixture.assert_no_receipt();
}

#[test]
fn a_6_row_terminal_gets_the_notice_and_the_listing() {
    let fixture = Fixture::new();
    let mut session = fixture.spawn(6, 80);
    let notice = "terminal too small for the checklist (needs 8 rows, 40 columns): enlarge it or name the parts";
    expect(&mut session, notice);
    expect(&mut session, "name the parts to install");
    finish(&mut session, 2);
    fixture.assert_no_receipt();
}
