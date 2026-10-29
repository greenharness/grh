//! What install, uninstall and status share: output, confirmation, running a
//! tool's own command, finding a part's binary and the closing summary.

use std::io::{BufRead, IsTerminal};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::catalog::{Catalog, Part};
use crate::dispatch;
use crate::exit::Exit;
use crate::receipt::{Entry, Kind, Level, Receipt};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mark {
    Ok,
    Warn,
    Fail,
}

/// Colour is off for `NO_COLOR` or when the stream is not a terminal.
pub fn use_color(is_tty: bool) -> bool {
    is_tty && std::env::var_os("NO_COLOR").is_none_or(|v| v.is_empty())
}

pub fn mark(kind: Mark, color: bool) -> String {
    let (glyph, code) = match kind {
        Mark::Ok => ("\u{2713}", "32"),
        Mark::Warn => ("!", "33"),
        Mark::Fail => ("\u{2717}", "31"),
    };
    if color {
        format!("\x1b[{code}m{glyph}\x1b[0m")
    } else {
        glyph.to_string()
    }
}

/// Progress on stderr. `quiet` keeps only errors and the summary.
#[derive(Debug, Clone, Copy)]
pub struct Ui {
    pub quiet: bool,
    pub color: bool,
}

impl Ui {
    pub fn new(quiet: bool) -> Ui {
        Ui {
            quiet,
            color: use_color(std::io::stderr().is_terminal()),
        }
    }

    pub fn say(&self, message: &str) {
        if !self.quiet {
            eprintln!("{message}");
        }
    }

    pub fn ok(&self, message: &str) {
        self.say(&format!("{} {message}", mark(Mark::Ok, self.color)));
    }

    pub fn warn(&self, message: &str) {
        self.say(&format!("{} {message}", mark(Mark::Warn, self.color)));
    }

    /// An error: shown even when quiet.
    pub fn fail(&self, message: &str) {
        eprintln!("{} {message}", mark(Mark::Fail, self.color));
    }
}

#[derive(Debug)]
pub struct Line {
    pub what: String,
    pub next: String,
}

/// What the run could not do, with the command that does it.
#[derive(Debug, Default)]
pub struct Summary {
    pub gaps: Vec<Line>,
    pub skipped: Vec<Line>,
    pub backups: Vec<Line>,
}

fn line(what: &str, next: &str) -> Line {
    Line {
        what: what.to_string(),
        next: next.to_string(),
    }
}

impl Summary {
    pub fn gap(&mut self, what: &str, next: &str) {
        self.gaps.push(line(what, next));
    }

    pub fn skip(&mut self, what: &str, next: &str) {
        self.skipped.push(line(what, next));
    }

    pub fn backup(&mut self, what: &str, next: &str) {
        self.backups.push(line(what, next));
    }

    /// Printed on stderr even when quiet; nothing when there is nothing.
    pub fn print(&self, ui: &Ui) {
        let sections = [
            ("gaps", &self.gaps),
            ("skipped files", &self.skipped),
            ("backups", &self.backups),
        ];
        if sections.iter().all(|(_, v)| v.is_empty()) {
            return;
        }
        eprintln!("summary:");
        for (title, lines) in sections {
            for l in lines {
                eprintln!(
                    "  {} {title}: {}\n      next: {}",
                    mark(Mark::Warn, ui.color),
                    l.what,
                    l.next
                );
            }
        }
    }
}

/// The exit a run ends with: the lowest non-zero code among `exits`, if any.
pub fn worst_of(exits: impl IntoIterator<Item = Exit>) -> Option<Exit> {
    exits.into_iter().reduce(Exit::worst)
}

pub fn text(e: &Exit) -> String {
    match e {
        Exit::Failed(m)
        | Exit::Usage(m)
        | Exit::Unpublished(m)
        | Exit::NotInstalled(m)
        | Exit::Locked(m) => m.clone(),
        Exit::Interrupted => "interrupted".into(),
    }
}

/// Whether the person already agreed on the command line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Consent {
    /// `--yes` was given.
    Given,
    /// Ask on the terminal.
    Ask,
}

impl Consent {
    pub fn from_yes_flag(yes: bool) -> Consent {
        if yes {
            Consent::Given
        } else {
            Consent::Ask
        }
    }
}

/// Ask y/N on a terminal; off a terminal, `--yes` is required.
pub fn confirm(consent: Consent, what: &str) -> Result<bool, Exit> {
    if consent == Consent::Given {
        return Ok(true);
    }
    if !std::io::stdin().is_terminal() {
        return Err(Exit::Usage(format!(
            "{what} needs confirmation and stdin is not a terminal: rerun with --yes (nothing changed)"
        )));
    }
    eprint!("Proceed? [y/N] ");
    let mut answer = String::new();
    std::io::stdin()
        .lock()
        .read_line(&mut answer)
        .map_err(|e| Exit::Failed(format!("cannot read the answer: {e}")))?;
    Ok(matches!(
        answer.trim().to_ascii_lowercase().as_str(),
        "y" | "yes"
    ))
}

/// The top of the git work tree around the current directory.
pub fn git_root() -> Result<PathBuf, Exit> {
    let out = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output();
    match out {
        Ok(output) if output.status.success() => {
            let top = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if top.is_empty() {
                Err(not_in_repo())
            } else {
                Ok(PathBuf::from(top))
            }
        }
        _ => Err(not_in_repo()),
    }
}

fn not_in_repo() -> Exit {
    Exit::Usage("--repo needs the current directory to be inside a git work tree".into())
}

pub fn absolute(path: &Path) -> PathBuf {
    std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf())
}

/// `{bin}` replaced by the binary's absolute path.
pub fn subst(argv: &[String], bin: &Path) -> Vec<String> {
    let b = bin.to_string_lossy();
    argv.iter().map(|a| a.replace("{bin}", &b)).collect()
}

/// Run a tool's own command; its output goes to stderr, indented. Returns the
/// exit code (-1 when it died by a signal).
pub fn run_tool(ui: &Ui, argv: &[String], cwd: Option<&Path>) -> Result<i32, String> {
    let Some((prog, args)) = argv.split_first() else {
        return Err("empty command".into());
    };
    ui.say(&format!("  $ {}", argv.join(" ")));
    let mut cmd = Command::new(prog);
    cmd.args(args).stdin(Stdio::null());
    if let Some(d) = cwd {
        cmd.current_dir(d);
    }
    let out = cmd
        .output()
        .map_err(|e| format!("cannot run {prog}: {e}"))?;
    for stream in [&out.stdout, &out.stderr] {
        for l in String::from_utf8_lossy(stream).lines() {
            ui.say(&format!("    {l}"));
        }
    }
    Ok(out.status.code().unwrap_or(-1))
}

/// Is `name` an executable on PATH?
pub fn on_path(name: &str) -> bool {
    use std::os::unix::fs::PermissionsExt;
    let path = std::env::var_os("PATH").unwrap_or_default();
    std::env::split_paths(&path).any(|d| {
        std::fs::metadata(d.join(name))
            .map(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    })
}

/// The binary the receipt records for `part`: its path and whether grh placed it.
pub fn recorded_bin(receipt: &Receipt, part: &str) -> Option<(PathBuf, bool)> {
    receipt
        .entries
        .iter()
        .filter(|e| e.part == part && e.level == Level::Machine)
        .find_map(|e| match &e.kind {
            Kind::BinaryPlaced { path, .. } => Some((PathBuf::from(path), true)),
            Kind::BinaryAdopted { path } => Some((PathBuf::from(path), false)),
            _ => None,
        })
}

/// The part's binary: the receipt's, else the first on PATH or in grh's bin dir.
pub fn bin_for(receipt: &Receipt, part: &Part) -> Option<PathBuf> {
    recorded_bin(receipt, &part.name)
        .map(|(path, _)| path)
        .filter(|path| path.exists())
        .or_else(|| part.bin.as_deref().and_then(dispatch::find))
}

pub fn is_integration(k: &Kind) -> bool {
    matches!(k, Kind::ToolInstall { .. } | Kind::ToolInstallAdopted)
}

/// Does `e` belong to `part` at `level` (and, for a repo, to `repo`)?
pub fn same_scope(e: &Entry, part: &str, level: Level, repo: Option<&Path>) -> bool {
    e.part == part
        && e.level == level
        && e.repo.as_deref() == repo.map(|r| r.to_string_lossy()).as_deref()
}

pub fn listing(cat: &Catalog) -> String {
    let mut out = String::from("name parts or profiles to install.\nparts:\n");
    for part in &cat.parts {
        out.push_str(&format!("  {:<11} {}\n", part.name, part.summary));
    }
    out.push_str("profiles:\n");
    for (name, members) in &cat.profiles {
        out.push_str(&format!("  {:<11} {}\n", name, members.join(" ")));
    }
    out
}
