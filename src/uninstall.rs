//! `grh uninstall`: undo what the receipt says grh did, dependents first, and
//! take a thing out only when its owner (or its bytes) confirms it is ours.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::catalog::{Catalog, Part, Unknown};
use crate::exit::Exit;
use crate::extension;
use crate::fetch::sha256_hex;
use crate::ops::{
    bin_for, confirm, git_root, is_integration, run_tool, text, worst_of, Consent, Summary, Ui,
};
use crate::owner::{self, Answer};
use crate::receipt::{self, Entry, Kind, Level, Store};
use crate::skills;

#[derive(Debug, Default)]
pub struct Opts {
    pub names: Vec<String>,
    pub repo: bool,
    pub keep_deps: bool,
    pub dry_run: bool,
    pub yes: bool,
    pub quiet: bool,
}

/// The entries of `part` this run may touch.
fn in_scope(e: &Entry, part: &str, only_repo: Option<&Path>) -> bool {
    e.part == part
        && match only_repo {
            None => true,
            Some(r) => e.level == Level::Repo && e.repo.as_deref() == Some(&*r.to_string_lossy()),
        }
}

pub fn run(cat: &Catalog, opts: &Opts) -> Result<(), Exit> {
    let ui = Ui::new(opts.quiet);
    let only_repo = if opts.repo { Some(git_root()?) } else { None };
    let state_dir = receipt::state_dir()?;
    let before = receipt::read_in(&state_dir)?;
    let held: BTreeSet<String> = before.entries.iter().map(|e| e.part.clone()).collect();

    // The parts to remove, dependents before what they need.
    let mut targets: BTreeSet<String> = BTreeSet::new();
    if opts.names.is_empty() {
        targets.extend(held.iter().cloned());
    } else {
        for n in &opts.names {
            if let Some(members) = cat.profiles.get(n) {
                targets.extend(members.iter().cloned());
            } else if cat.part(n).is_some() || n.starts_with(extension::PREFIX) {
                targets.insert(n.clone());
            } else {
                return Err(Exit::Usage(format!("unknown part or profile: {n}")));
            }
        }
    }
    let all: Vec<String> = cat
        .parts
        .iter()
        .map(|planned| planned.name.clone())
        .collect();
    let order: Vec<&Part> = cat
        .resolve(&all)
        .map_err(|Unknown::Name(n)| Exit::Usage(format!("unknown part {n}")))?
        .into_iter()
        .rev()
        .filter(|planned| targets.contains(&planned.name))
        .collect();
    let ext_todo: Vec<String> = targets
        .iter()
        .filter(|t| t.starts_with(extension::PREFIX) && held.contains(*t))
        .cloned()
        .collect();
    let (todo, absent): (Vec<&Part>, Vec<&Part>) = order.into_iter().partition(|planned| {
        before
            .entries
            .iter()
            .any(|e| in_scope(e, &planned.name, only_repo.as_deref()))
    });
    for planned in &absent {
        ui.say(&format!(
            "{}: nothing recorded by grh, skipped",
            planned.name
        ));
    }
    if todo.is_empty() && (ext_todo.is_empty() || only_repo.is_some()) {
        ui.say("nothing to uninstall");
        return Ok(());
    }

    // A part another installed part needs.
    if only_repo.is_none() {
        let remaining: BTreeSet<String> = held
            .iter()
            .filter(|h| !targets.contains(*h))
            .cloned()
            .collect();
        for planned in &todo {
            for dep in cat.dependents(&planned.name, &remaining) {
                if opts.keep_deps {
                    return Err(Exit::Failed(format!(
                        "{} is needed by {dep}: run `grh uninstall {dep}` first (nothing changed)",
                        planned.name
                    )));
                }
                ui.warn(&format!(
                    "{dep} still needs {}: it will stop working; remove it with `grh uninstall {dep}`",
                    planned.name
                ));
            }
        }
    }

    ui.say("grh will remove:");
    for planned in &todo {
        let n = before
            .entries
            .iter()
            .filter(|e| in_scope(e, &planned.name, only_repo.as_deref()))
            .count();
        ui.say(&format!(
            "  {}: {n} recorded step(s), undone newest scope first",
            planned.name
        ));
    }
    for x in ext_todo.iter().filter(|_| only_repo.is_none()) {
        let n = before.entries.iter().filter(|e| e.part == *x).count();
        ui.say(&format!(
            "  {x}: {n} recorded step(s), undone newest scope first"
        ));
    }
    if opts.dry_run {
        ui.say("dry run: nothing changed");
        return Ok(());
    }
    if !confirm(Consent::from_yes_flag(opts.yes), "uninstall")? {
        ui.say("nothing changed");
        return Ok(());
    }

    let mut store = Store::open_in(&state_dir)?;
    let mut sum = Summary::default();
    let mut failures: Vec<Exit> = Vec::new();
    for part in &todo {
        ui.say(&part.name);
        failures.extend(remove_part(
            &ui,
            &mut store,
            &mut sum,
            part,
            only_repo.as_deref(),
        ));
    }
    if only_repo.is_none() {
        for x in &ext_todo {
            ui.say(x);
            failures.extend(extension::remove(&ui, &mut store, x));
        }
    }
    sum.print(&ui);
    match worst_of(failures) {
        None => Ok(()),
        Some(e) => Err(e),
    }
}

/// Report a failure for `part` and return it for the run's result.
fn fail(ui: &Ui, part: &str, msg: String) -> Exit {
    ui.fail(&format!("{part}: {msg}"));
    Exit::Failed(msg)
}

fn remove_part(
    ui: &Ui,
    store: &mut Store,
    sum: &mut Summary,
    part: &Part,
    only_repo: Option<&Path>,
) -> Vec<Exit> {
    let name = part.name.as_str();
    let mut failures: Vec<Exit> = Vec::new();
    let entries: Vec<Entry> = store
        .receipt
        .entries
        .iter()
        .filter(|e| in_scope(e, name, only_repo))
        .cloned()
        .collect();
    let bin = bin_for(&store.receipt, part);

    for level in [Level::Repo, Level::Machine] {
        for e in entries.iter().filter(|e| e.level == level) {
            let outcome = match &e.kind {
                Kind::ToolInstall { undo_argv, .. } => {
                    undo_tool(ui, part, e, undo_argv, bin.as_deref())
                }
                Kind::ToolInstallAdopted => {
                    ui.say(&format!("  {name}: adopted integration left in place"));
                    Ok(())
                }
                Kind::File { .. } if level == Level::Repo => remove_repo_file(ui, sum, e),
                _ => continue,
            };
            match outcome {
                Ok(()) => {
                    if let Err(x) = store.remove_where(|x| x == e) {
                        failures.push(x);
                    }
                }
                Err(m) => failures.push(fail(ui, name, format!("{m}; entry kept"))),
            }
        }
        if level == Level::Machine && only_repo.is_none() {
            match skills::remove(name, store) {
                Ok(r) => {
                    if !r.removed.is_empty() {
                        ui.ok(&format!(
                            "{name}: removed {} skill and agent files",
                            r.removed.len()
                        ));
                    }
                    for k in r.kept {
                        sum.skip(
                            &format!("{}: {}", k.path.display(), k.reason),
                            "remove it by hand if you no longer want it",
                        );
                    }
                }
                Err(e) => failures.push(fail(ui, name, text(&e))),
            }
        }
    }

    // Backups: put the original back when our file is gone.
    for e in entries
        .iter()
        .filter(|e| matches!(e.kind, Kind::Backup { .. }))
    {
        if let Kind::Backup { path, from } = &e.kind {
            let (backup, from) = (PathBuf::from(path), PathBuf::from(from));
            if !backup.exists() {
                // Already restored or removed by hand.
            } else if !from.exists() {
                let restored =
                    std::fs::copy(&backup, &from).and_then(|_| std::fs::remove_file(&backup));
                match restored {
                    Ok(()) => ui.ok(&format!("{name}: restored {}", from.display())),
                    Err(x) => {
                        failures.push(fail(
                            ui,
                            name,
                            format!("cannot restore {}: {x}; backup kept", from.display()),
                        ));
                        continue;
                    }
                }
            } else {
                sum.backup(
                    &format!(
                        "{} was changed since; the original stays at {}",
                        from.display(),
                        backup.display()
                    ),
                    &format!("cp {} {}", backup.display(), from.display()),
                );
            }
            if let Err(x) = store.remove_where(|x| x == e) {
                failures.push(x);
            }
        }
    }

    // The binary goes last, and only when every integration recorded for it
    // is confirmed gone.
    if only_repo.is_none() {
        failures.extend(remove_binary(ui, store, sum, name));
    }
    failures
}

/// Run an entry's undo, then ask the owner: Ok only when it reports Absent.
fn undo_tool(
    ui: &Ui,
    part: &Part,
    e: &Entry,
    undo: &[String],
    bin: Option<&Path>,
) -> Result<(), String> {
    let cwd = e.repo.as_deref().map(Path::new);
    if let Some(d) = cwd {
        if !d.is_dir() {
            return Err(format!(
                "{} no longer exists, so its integration cannot be checked",
                d.display()
            ));
        }
    }
    let integ = match e.level {
        Level::Machine => part.machine.as_ref(),
        Level::Repo => part.repo_level.as_ref(),
    }
    .ok_or_else(|| "the catalog no longer lists this integration".to_string())?;
    let code = run_tool(ui, undo, cwd)?;
    let state = match bin {
        Some(b) => owner::query(&integ.query, b, cwd),
        None => Answer::Unknown,
    };
    if state == Answer::Absent {
        ui.ok(&format!("{}: integration removed", part.name));
        Ok(())
    } else {
        Err(format!(
            "{} exited {code} and the integration still reports {}",
            undo.join(" "),
            state.as_str()
        ))
    }
}

/// Remove a repo file whose bytes still match the receipt; prune empty dirs.
fn remove_repo_file(ui: &Ui, sum: &mut Summary, e: &Entry) -> Result<(), String> {
    let Kind::File { path, sha256 } = &e.kind else {
        return Ok(());
    };
    let p = Path::new(path);
    match std::fs::read(p) {
        Err(x) if x.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(x) => Err(format!("cannot read {path}: {x}")),
        Ok(cur) if sha256_hex(&cur) == *sha256 => {
            std::fs::remove_file(p).map_err(|x| format!("cannot remove {path}: {x}"))?;
            let top = e.repo.as_deref().map(Path::new);
            let mut dir = p.parent();
            while let Some(d) = dir {
                if Some(d) == top || std::fs::remove_dir(d).is_err() {
                    break;
                }
                dir = d.parent();
            }
            ui.ok(&format!("removed {path}"));
            Ok(())
        }
        Ok(_) => {
            sum.skip(
                &format!("{path}: changed since grh placed it"),
                "remove it by hand if you no longer want it",
            );
            Ok(())
        }
    }
}

fn remove_binary(ui: &Ui, store: &mut Store, sum: &mut Summary, name: &str) -> Vec<Exit> {
    let mut failures: Vec<Exit> = Vec::new();
    let blocked = store.receipt.entries.iter().any(|e| {
        e.part == name && is_integration(&e.kind) && matches!(e.kind, Kind::ToolInstall { .. })
    });
    let bins: Vec<Entry> = store
        .receipt
        .entries
        .iter()
        .filter(|e| {
            e.part == name
                && matches!(
                    e.kind,
                    Kind::BinaryPlaced { .. } | Kind::BinaryAdopted { .. }
                )
        })
        .cloned()
        .collect();
    for e in bins {
        match &e.kind {
            Kind::BinaryAdopted { .. } => {
                failures.extend(store.remove_where(|x| *x == e).err());
            }
            Kind::BinaryPlaced { path, sha256 } => {
                if blocked {
                    sum.gap(
                        &format!("{path} kept: an integration of {name} is not confirmed removed"),
                        &format!("grh uninstall {name}"),
                    );
                    continue;
                }
                let p = Path::new(path);
                match std::fs::read(p) {
                    Err(x) if x.kind() == std::io::ErrorKind::NotFound => {}
                    Err(x) => {
                        failures.push(fail(
                            ui,
                            name,
                            format!("cannot read {path}: {x}; entry kept"),
                        ));
                        continue;
                    }
                    Ok(cur) if sha256.is_empty() || sha256_hex(&cur) == *sha256 => {
                        if let Err(x) = std::fs::remove_file(p) {
                            failures.push(fail(
                                ui,
                                name,
                                format!("cannot remove {path}: {x}; entry kept"),
                            ));
                            continue;
                        }
                        ui.ok(&format!("removed {path}"));
                    }
                    Ok(_) => sum.skip(
                        &format!("{path}: changed since grh placed it"),
                        "remove it by hand if you no longer want it",
                    ),
                }
                failures.extend(store.remove_where(|x| *x == e).err());
            }
            _ => {}
        }
    }
    failures
}
