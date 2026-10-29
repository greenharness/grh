//! `grh install --extension <dir>`: place an extension's skills and agents,
//! register its MCP servers with `claude`, print what the person adds by
//! hand, and record everything under the part `ext:<name>`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::catalog::Part;
use crate::exit::Exit;
use crate::manifest::Manifest;
use crate::ops::{confirm, on_path, run_tool, Consent, Summary, Ui};
use crate::receipt::{Entry, Kind, State, Store};
use crate::skills::{self, Bundle};

pub const PREFIX: &str = "ext:";

#[derive(Debug, Default)]
pub struct Opts {
    pub dir: PathBuf,
    pub replace: bool,
    pub dry_run: bool,
    pub yes: bool,
    pub quiet: bool,
}

fn read_tree(dir: &Path, rel: &str, out: &mut BTreeMap<String, Vec<u8>>) -> Result<(), Exit> {
    let fail =
        |p: &Path, e: std::io::Error| Exit::Failed(format!("cannot read {}: {e}", p.display()));
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .map_err(|e| fail(dir, e))?
        .collect::<Result<_, _>>()
        .map_err(|e| fail(dir, e))?;
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let path = e.path();
        let name = format!("{rel}/{}", e.file_name().to_string_lossy());
        if path.is_dir() {
            read_tree(&path, &name, out)?;
        } else {
            out.insert(name, std::fs::read(&path).map_err(|x| fail(&path, x))?);
        }
    }
    Ok(())
}

fn stem(path: &str) -> String {
    let p = Path::new(path);
    p.file_stem()
        .or_else(|| p.file_name())
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// The part the extension is recorded as, and its files by config-dir path.
fn gather(dir: &Path, m: &Manifest) -> Result<(Part, Bundle), Exit> {
    let mut files = BTreeMap::new();
    let mut skills = Vec::new();
    let mut agents = Vec::new();
    for s in &m.skill {
        let src = dir.join(&s.path);
        let name = Path::new(&s.path)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        if src.is_dir() {
            read_tree(&src, &format!("skills/{name}"), &mut files)?;
        } else if src.is_file() {
            let name = stem(&s.path);
            let bytes = std::fs::read(&src)
                .map_err(|e| Exit::Failed(format!("cannot read {}: {e}", src.display())))?;
            files.insert(format!("skills/{name}/SKILL.md"), bytes);
            skills.push(name);
            continue;
        } else {
            return Err(Exit::Usage(format!("skill path {} does not exist", s.path)));
        }
        skills.push(name);
    }
    for a in &m.agent {
        let src = dir.join(&a.path);
        let name = stem(&a.path);
        let bytes = std::fs::read(&src)
            .map_err(|_| Exit::Usage(format!("agent path {} is not a file", a.path)))?;
        files.insert(format!("agents/{name}.md"), bytes);
        agents.push(name);
    }
    let part = Part {
        name: format!("{PREFIX}{}", m.extension.name),
        summary: format!("extension {} {}", m.extension.name, m.extension.version),
        needs: Vec::new(),
        bin: None,
        repo: None,
        version: None,
        sha256: BTreeMap::new(),
        machine: None,
        repo_level: None,
        skills,
        agents,
        hint: None,
        workflow: None,
        extension: None,
    };
    Ok((part, Bundle::from_files(files)))
}

fn mcp_argv(m: &crate::manifest::Mcp) -> (Vec<String>, Vec<String>) {
    let mut add: Vec<String> = [
        "claude", "mcp", "add", "--scope", "user", &m.name, "--", &m.command,
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    add.extend(m.args.iter().cloned());
    let remove = ["claude", "mcp", "remove", "--scope", "user", &m.name]
        .iter()
        .map(|s| s.to_string())
        .collect();
    (add, remove)
}

/// Does `claude mcp get <name>` find a server? Err when claude cannot run.
fn mcp_exists(name: &str) -> Result<bool, String> {
    Command::new("claude")
        .args(["mcp", "get", name])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .map_err(|e| format!("cannot run claude: {e}"))
}

/// The SessionStart block for `[[session_context]]`, printed, never written.
fn session_block(m: &Manifest) -> serde_json::Value {
    let hooks: Vec<_> = m
        .session_context
        .iter()
        .map(|c| serde_json::json!({ "type": "command", "command": c.command }))
        .collect();
    serde_json::json!({ "hooks": { "SessionStart": [ { "hooks": hooks } ] } })
}

pub fn run(opts: &Opts) -> Result<(), Exit> {
    let ui = Ui::new(opts.quiet);
    let m = Manifest::load(&opts.dir)?;
    let dir = &opts.dir;
    let (part, bundle) = gather(dir, &m)?;
    let name = part.name.clone();
    ui.say("grh will install:");
    ui.say(&format!(
        "  {name}: {} skills, {} agents, {} mcp servers",
        part.skills.len(),
        part.agents.len(),
        m.mcp.len()
    ));
    if opts.dry_run {
        ui.say("dry run: nothing changed");
        return Ok(());
    }
    if !confirm(Consent::from_yes_flag(opts.yes), "install")? {
        ui.say("nothing changed");
        return Ok(());
    }
    let mut store = Store::open()?;
    let mut failures: Vec<Exit> = Vec::new();
    let mut sum = Summary::default();

    if !part.skills.is_empty() || !part.agents.is_empty() {
        let claude = skills::claude_dir()?;
        let placement = skills::place(&bundle, &part, &claude, &mut store, opts.replace)?;
        ui.ok(&format!(
            "{name}: {} skill and agent files in {}",
            placement.placed.len(),
            claude.display()
        ));
        for s in &placement.skipped {
            ui.warn(&format!("{}: {}", s.path.display(), s.reason));
        }
    }

    for mcp in &m.mcp {
        let (argv, undo_argv) = mcp_argv(mcp);
        let known = store.receipt.entries.iter().any(|e| {
            e.part == name
                && e.state == State::Done
                && matches!(&e.kind, Kind::ToolInstall { argv: a, .. } if *a == argv)
        });
        if known {
            ui.ok(&format!("{name}: mcp {} registered (unchanged)", mcp.name));
            continue;
        }
        if !on_path("claude") {
            let e = Exit::Failed(format!(
                "claude is not on PATH, so mcp {} was not registered",
                mcp.name
            ));
            ui.fail(&format!("{name}: {}", crate::ops::text(&e)));
            failures.push(e);
            continue;
        }
        match mcp_exists(&mcp.name) {
            Ok(true) => {
                // A server by that name that grh has no done entry for is not
                // grh's: leave it, record nothing, and never undo it.
                let pending = store.receipt.entries.iter().position(|e| {
                    e.part == name
                        && e.state == State::Pending
                        && matches!(&e.kind, Kind::ToolInstall { argv: a, .. } if *a == argv)
                });
                if let Some(idx) = pending {
                    store.done(idx)?;
                    ui.ok(&format!("{name}: mcp {} registered (recovered)", mcp.name));
                } else {
                    ui.warn(&format!(
                        "{name}: mcp {} already exists, adopted and left alone",
                        mcp.name
                    ));
                    sum.skip(
                        &format!("mcp {} already exists and is not grh's", mcp.name),
                        "leave it, or claude mcp remove it and grh install --extension again",
                    );
                }
                continue;
            }
            Ok(false) => {}
            Err(msg) => {
                let e = Exit::Failed(msg);
                ui.fail(&format!("{name}: {}", crate::ops::text(&e)));
                failures.push(e);
                continue;
            }
        }
        store.remove_where(|e| {
            e.part == name && matches!(&e.kind, Kind::ToolInstall { argv: a, .. } if *a == argv)
        })?;
        let idx = store.begin(Entry::machine(
            &name,
            Kind::ToolInstall {
                argv: argv.clone(),
                undo_argv,
            },
        ))?;
        match run_tool(&ui, &argv, None) {
            Ok(0) => {
                store.done(idx)?;
                ui.ok(&format!("{name}: mcp {} registered", mcp.name));
            }
            Ok(code) => {
                // The add did not happen: drop its pending step.
                store.receipt.entries.remove(idx);
                store.save()?;
                let e = Exit::Failed(format!("{} exited {code}", argv.join(" ")));
                ui.fail(&format!("{name}: {}", crate::ops::text(&e)));
                failures.push(e);
            }
            Err(msg) => {
                store.receipt.entries.remove(idx);
                store.save()?;
                let e = Exit::Failed(msg);
                ui.fail(&format!("{name}: {}", crate::ops::text(&e)));
                failures.push(e);
            }
        }
    }

    for d in &m.doctor {
        let exists = store.receipt.entries.iter().any(|e| {
            e.part == name && matches!(&e.kind, Kind::Doctor { command } if *command == d.command)
        });
        if !exists {
            let idx = store.begin(Entry::machine(
                &name,
                Kind::Doctor {
                    command: d.command.clone(),
                },
            ))?;
            store.done(idx)?;
        }
    }

    if !m.session_context.is_empty() {
        let block = serde_json::to_string_pretty(&session_block(&m))
            .map_err(|e| Exit::Failed(format!("cannot encode the session block: {e}")))?;
        ui.say("Add this to the \"hooks\" of settings.json (grh did not write it):");
        ui.say(&block);
        sum.gap(
            &format!("{name}: the session block was not written to settings.json"),
            &format!("add this to the \"hooks\" of settings.json: {block}"),
        );
    }
    if let Some(line) = m.conform.as_ref().and_then(|c| c.eslint.as_ref()) {
        ui.say(&format!("conform eslint: {line}"));
        sum.gap(
            &format!("{name}: the conform eslint line was not added"),
            &format!("add to your eslint config: {line}"),
        );
    }
    sum.print(&ui);
    match crate::ops::worst_of(failures) {
        None => Ok(()),
        Some(e) => Err(e),
    }
}

/// Take out what the receipt holds for the extension part `name`.
pub fn remove(ui: &Ui, store: &mut Store, name: &str) -> Vec<Exit> {
    let mut failures = Vec::new();
    let entries: Vec<Entry> = store
        .receipt
        .entries
        .iter()
        .filter(|e| e.part == name)
        .cloned()
        .collect();
    for e in &entries {
        let outcome = match &e.kind {
            Kind::ToolInstall { undo_argv, .. } => {
                let ran = run_tool(ui, undo_argv, None);
                // The entry goes only when claude reports the server gone.
                match (ran, undo_argv.last().map(|n| mcp_exists(n))) {
                    (_, Some(Ok(false))) => Ok(()),
                    (Ok(code), _) => Err(format!(
                        "{} exited {code} and the server is not confirmed gone; entry kept",
                        undo_argv.join(" ")
                    )),
                    (Err(m), _) => Err(format!("{m}; entry kept")),
                }
            }
            Kind::Doctor { .. } => Ok(()),
            _ => continue,
        };
        match outcome {
            Ok(()) => {
                if let Err(x) = store.remove_where(|x| x == e) {
                    failures.push(x);
                }
            }
            Err(m) => {
                ui.fail(&format!("{name}: {m}"));
                failures.push(Exit::Failed(m));
            }
        }
    }
    match skills::remove(name, store) {
        Ok(r) => {
            if !r.removed.is_empty() {
                ui.ok(&format!(
                    "{name}: removed {} skill and agent files",
                    r.removed.len()
                ));
            }
            for k in r.kept {
                ui.warn(&format!("{}: {}", k.path.display(), k.reason));
            }
        }
        Err(e) => failures.push(e),
    }
    failures
}

/// (extension part, first output line) of each recorded `[[doctor]]` command.
pub fn doctor_lines(entries: &[Entry]) -> Vec<(String, String, String)> {
    entries
        .iter()
        .filter_map(|e| match &e.kind {
            Kind::Doctor { command } => {
                Some((e.part.clone(), command.clone(), first_line(command)))
            }
            _ => None,
        })
        .collect()
}

fn first_line(command: &str) -> String {
    let out = Command::new("sh")
        .args(["-c", command])
        .stdin(Stdio::null())
        .output();
    match out {
        Ok(o) => {
            let text = String::from_utf8_lossy(if o.stdout.is_empty() {
                &o.stderr
            } else {
                &o.stdout
            })
            .lines()
            .next()
            .unwrap_or("")
            .trim()
            .to_string();
            if text.is_empty() {
                format!("exited {}", o.status.code().unwrap_or(-1))
            } else {
                text
            }
        }
        Err(e) => format!("cannot run: {e}"),
    }
}
