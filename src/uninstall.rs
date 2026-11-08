//! `grh uninstall`: undo what the receipt says grh did, dependents first, and
//! take a thing out only when its owner (or its bytes) confirms it is ours.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::catalog::{Catalog, Part, Unknown};
use crate::exit::Exit;
use crate::extension;
use crate::fetch::sha256_hex;
use crate::manifest::{KeepBinary, Status};
use crate::ops::{
    bin_for, confirm, git_root, is_integration, recorded_bin_where, run_tool, text, worst_of,
    Consent, Summary, Ui,
};
use crate::owner::{self, Answer};
use crate::receipt::{self, Entry, Kind, Level, RecordedStatus, Store};
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
fn in_scope(e: &Entry, part: &Part, only_repo: Option<&Path>) -> bool {
    part.answers_to(&e.part)
        && match only_repo {
            None => true,
            Some(r) => e.level == Level::Repo && e.repo.as_deref() == Some(&*r.to_string_lossy()),
        }
}

/// The catalog's name for `name`, which may be an alias; other names stay.
fn canonical(cat: &Catalog, name: &str) -> String {
    cat.part(name)
        .map_or_else(|| name.to_string(), |p| p.name().to_string())
}

pub fn run(cat: &Catalog, opts: &Opts) -> Result<(), Exit> {
    let ui = Ui::new(opts.quiet);
    crate::install::warn_aliases(&ui, cat, &opts.names);
    let only_repo = if opts.repo { Some(git_root()?) } else { None };
    let state_dir = receipt::state_dir()?;
    let mut before = receipt::read_in(&state_dir)?;
    before.rename_parts(|name| cat.name_of_alias(name));
    let held: BTreeSet<String> = before.entries.iter().map(|e| e.part.clone()).collect();

    // The parts to remove, dependents before what they need.
    let mut targets: BTreeSet<String> = BTreeSet::new();
    if opts.names.is_empty() {
        targets.extend(held.iter().cloned());
    } else {
        for n in &opts.names {
            if let Some(members) = cat.profiles.get(n) {
                targets.extend(members.iter().map(|m| canonical(cat, m)));
            } else if cat.part(n).is_some() || n.starts_with(extension::PREFIX) {
                targets.insert(canonical(cat, n));
            } else {
                return Err(Exit::Usage(format!("unknown part or profile: {n}")));
            }
        }
    }
    let all: Vec<String> = cat
        .parts
        .iter()
        .map(|planned| planned.name().to_string())
        .collect();
    let order: Vec<&Part> = cat
        .resolve(&all)
        .map_err(|Unknown::Name(n)| Exit::Usage(format!("unknown part {n}")))?
        .into_iter()
        .rev()
        .filter(|planned| targets.contains(planned.name()))
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
            .any(|e| in_scope(e, planned, only_repo.as_deref()))
    });
    for planned in &absent {
        ui.say(&format!(
            "{}: nothing recorded by grh, skipped",
            planned.name()
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
            for dep in cat.dependents(planned.name(), &remaining) {
                if opts.keep_deps {
                    return Err(Exit::Failed(format!(
                        "{} is needed by {dep}: run `grh uninstall {dep}` first (nothing changed)",
                        planned.name()
                    )));
                }
                ui.warn(&format!(
                    "{dep} still needs {}: it will stop working; remove it with `grh uninstall {dep}`",
                    planned.name()
                ));
            }
        }
    }

    ui.say("grh will remove:");
    for planned in &todo {
        let n = before
            .entries
            .iter()
            .filter(|e| in_scope(e, planned, only_repo.as_deref()))
            .count();
        ui.say(&format!(
            "  {}: {n} recorded step(s), undone newest scope first",
            planned.name()
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
    store.migrate(|name| cat.name_of_alias(name))?;
    let mut sum = Summary::default();
    let mut failures: Vec<Exit> = Vec::new();
    for part in &todo {
        ui.say(part.name());
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
            failures.extend(remove_extension(&ui, &mut store, x).settle(&mut sum));
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
    let name = part.name();
    let mut failures: Vec<Exit> = Vec::new();
    let entries: Vec<Entry> = store
        .receipt
        .entries
        .iter()
        .filter(|e| in_scope(e, part, only_repo))
        .cloned()
        .collect();
    let bin = bin_for(&store.receipt, part);
    let undo = Undo {
        ui,
        name,
        part: Some(part),
        bin: bin.as_deref(),
    };

    for level in [Level::Repo, Level::Machine] {
        failures.extend(undo_entries(&undo, store, &entries, level).settle(sum));
        if level == Level::Machine && only_repo.is_none() {
            match skills::remove(|recorded| part.answers_to(recorded), store) {
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

    failures.extend(restore_backups(ui, store, name, &entries).settle(sum));

    // The binary goes last, and only when every integration recorded for it
    // is confirmed gone.
    if only_repo.is_none() {
        // The catalog's spec wins for a part it pins.
        failures.extend(remove_binary(
            ui,
            store,
            sum,
            name,
            &|recorded| part.answers_to(recorded),
            part.keep_binary(),
        ));
    }
    failures
}

/// What an undo reports: the failures it ends with and the summary lines it
/// collected, for the caller to merge.
#[derive(Debug, Default)]
struct Undone {
    failures: Vec<Exit>,
    summary: Summary,
}

impl Undone {
    /// Merge the summary lines into `sum` and return the failures.
    fn settle(self, sum: &mut Summary) -> Vec<Exit> {
        sum.merge(self.summary);
        self.failures
    }
}

/// Who an undo is for and the binary that answers its status query.
struct Undo<'a> {
    ui: &'a Ui,
    name: &'a str,
    part: Option<&'a Part>,
    bin: Option<&'a Path>,
}

/// Undo what `entries` record at `level`: a tool's undo (confirmed by its
/// status query) and a placed repo file.
fn undo_entries(undo: &Undo, store: &mut Store, entries: &[Entry], level: Level) -> Undone {
    let (ui, name) = (undo.ui, undo.name);
    let mut failures: Vec<Exit> = Vec::new();
    let mut sum = Summary::default();
    for e in entries.iter().filter(|e| e.level == level) {
        let outcome = match &e.kind {
            Kind::ToolInstall {
                undo_argv, status, ..
            } => undo_tool(undo, e, undo_argv, status.as_ref()),
            Kind::ToolInstallAdopted => {
                ui.say(&format!("  {name}: adopted integration left in place"));
                Ok(())
            }
            Kind::File { .. } if level == Level::Repo => remove_repo_file(ui, &mut sum, e),
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
    Undone {
        failures,
        summary: sum,
    }
}

/// Take out an extension: its tool, files, servers and binary, each only as
/// far as the receipt recorded it, with no manifest to read.
fn remove_extension(ui: &Ui, store: &mut Store, name: &str) -> Undone {
    let mut failures: Vec<Exit> = Vec::new();
    let mut sum = Summary::default();
    let entries: Vec<Entry> = store
        .receipt
        .entries
        .iter()
        .filter(|e| e.part == name)
        .cloned()
        .collect();
    let keep = entries.iter().find_map(|e| {
        if let Kind::BinaryPlaced { keep_binary, .. } = &e.kind {
            keep_binary.as_deref().cloned().map(KeepBinary::from)
        } else {
            None
        }
    });
    let bin = recorded_bin_where(&store.receipt, |recorded| recorded == name)
        .map(|(path, _)| path)
        .filter(|path| path.exists());
    // A tool entry that recorded its status, or was adopted, is handled here;
    // the rest are the extension's own `claude` registrations.
    let recorded_tools: Vec<Entry> = entries
        .iter()
        .filter(|e| {
            matches!(
                &e.kind,
                Kind::ToolInstall {
                    status: Some(_),
                    ..
                } | Kind::ToolInstallAdopted
            )
        })
        .cloned()
        .collect();
    let undo = Undo {
        ui,
        name,
        part: None,
        bin: bin.as_deref(),
    };
    for level in [Level::Repo, Level::Machine] {
        failures.extend(undo_entries(&undo, store, &recorded_tools, level).settle(&mut sum));
    }
    let repo_files: Vec<Entry> = entries
        .iter()
        .filter(|e| matches!(e.kind, Kind::File { .. }) && e.level == Level::Repo)
        .cloned()
        .collect();
    failures.extend(undo_entries(&undo, store, &repo_files, Level::Repo).settle(&mut sum));
    failures.extend(extension::remove(ui, store, name));
    failures.extend(restore_backups(ui, store, name, &entries).settle(&mut sum));
    failures.extend(remove_binary(
        ui,
        store,
        &mut sum,
        name,
        &|recorded| recorded == name,
        keep.as_ref(),
    ));
    Undone {
        failures,
        summary: sum,
    }
}

/// Put an original back when the file grh placed over it is gone.
fn restore_backups(ui: &Ui, store: &mut Store, name: &str, entries: &[Entry]) -> Undone {
    let mut failures: Vec<Exit> = Vec::new();
    let mut sum = Summary::default();
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
    Undone {
        failures,
        summary: sum,
    }
}

/// Run an entry's undo, then ask the owner: Ok only when it reports Absent.
/// The query is the catalog's for a part it pins, else the one the receipt
/// recorded at install time.
fn undo_tool(
    undo: &Undo,
    e: &Entry,
    undo_argv: &[String],
    recorded: Option<&RecordedStatus>,
) -> Result<(), String> {
    let (ui, name, part, bin) = (undo.ui, undo.name, undo.part, undo.bin);
    let cwd = e.repo.as_deref().map(Path::new);
    if let Some(d) = cwd {
        if !d.is_dir() {
            return Err(format!(
                "{} no longer exists, so its integration cannot be checked",
                d.display()
            ));
        }
    }
    let catalog_status = part.and_then(|p| {
        match e.level {
            Level::Machine => p.machine(),
            Level::Repo => p.repo_level(),
        }
        .and_then(|level| level.status.clone())
    });
    let status = catalog_status
        .or_else(|| recorded.and_then(|raw| Status::try_from(raw.clone()).ok()))
        .ok_or_else(|| {
            "no status query is declared or recorded for this integration".to_string()
        })?;
    let code = run_tool(ui, undo_argv, cwd)?;
    let state = match bin {
        Some(b) => owner::run_status(&status, b, cwd),
        None => Answer::Unknown,
    };
    if state == Answer::Absent {
        ui.ok(&format!("{name}: integration removed"));
        Ok(())
    } else {
        Err(format!(
            "{} exited {code} and the integration still reports {}",
            undo_argv.join(" "),
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

fn remove_binary(
    ui: &Ui,
    store: &mut Store,
    sum: &mut Summary,
    name: &str,
    belongs: &dyn Fn(&str) -> bool,
    keep: Option<&KeepBinary>,
) -> Vec<Exit> {
    let mut failures: Vec<Exit> = Vec::new();
    let blocked = store.receipt.entries.iter().any(|e| {
        belongs(&e.part) && is_integration(&e.kind) && matches!(e.kind, Kind::ToolInstall { .. })
    });
    let bins: Vec<Entry> = store
        .receipt
        .entries
        .iter()
        .filter(|e| {
            belongs(&e.part)
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
            Kind::BinaryPlaced { path, sha256, .. } => {
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
                        if let Some(spec) = keep {
                            let bin_name = p.file_name().map_or_else(
                                || name.to_string(),
                                |n| n.to_string_lossy().into_owned(),
                            );
                            match owner::keep_binary(spec, &bin_name, p, path) {
                                owner::Template::Keep(why) => {
                                    sum.gap(
                                        &format!("{path} kept: {why}"),
                                        &format!(
                                            "grh uninstall {name}, once `{path} {}` reports {} at {}",
                                            spec.args.join(" "),
                                            spec.absent,
                                            spec.state.join(".")
                                        ),
                                    );
                                    continue;
                                }
                                owner::Template::Gone => {
                                    ui.warn(&format!("{path} is removed: {}", spec.warning));
                                }
                            }
                        }
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
