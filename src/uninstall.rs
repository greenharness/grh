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
    if let Some(notice) = cat.retired_notice(&opts.names, crate::catalog::Verb::Uninstall) {
        return Err(Exit::Usage(notice));
    }
    let ui = Ui::new(opts.quiet);
    cat.warn_aliases(&ui, &opts.names, crate::catalog::Verb::Uninstall);
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
            if cat.part(n).is_some() || n.starts_with(extension::PREFIX) {
                targets.insert(canonical(cat, n));
            } else {
                return Err(Exit::Usage(format!("unknown part: {n}")));
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

    let mut store = Store::open_migrating(|name| cat.name_of_alias(name))?;
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
            &cat.foreign_markers,
        ));
    }
    if only_repo.is_none() {
        for x in &ext_todo {
            ui.say(x);
            failures.extend(
                remove_extension(&ui, &mut store, x, &cat.foreign_markers).settle(&mut sum),
            );
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
    foreign_markers: &[String],
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
            match skills::remove(|recorded| part.answers_to(recorded), foreign_markers, store) {
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
fn remove_extension(ui: &Ui, store: &mut Store, name: &str, foreign_markers: &[String]) -> Undone {
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
    failures.extend(extension::remove(ui, store, name, foreign_markers));
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

/// Copy `backup` over the absent `from`: create its parent directory, copy to
/// a hidden temporary file beside it, and rename that into place. On failure
/// the backup stays and no temporary file is left. Once the original is back,
/// the backup is removed; a failure there is returned as `Ok(Some(_))`, since
/// the restore itself succeeded. (A file created at `from` after the caller's
/// check is replaced: grh is the only writer it expects there.)
fn restore_file(backup: &Path, from: &Path) -> std::io::Result<Option<std::io::Error>> {
    let parent = from.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)?;
    let mut staging_name = std::ffi::OsString::from(".");
    staging_name.push(from.file_name().unwrap_or_default());
    staging_name.push(format!(".grh-restore-{}", std::process::id()));
    let staging = parent.join(staging_name);
    let placed = std::fs::copy(backup, &staging).and_then(|_| std::fs::rename(&staging, from));
    if let Err(x) = placed {
        let _ = std::fs::remove_file(&staging); // best effort: the copy error is the one reported
        return Err(x);
    }
    Ok(std::fs::remove_file(backup).err())
}

/// `path` quoted for a POSIX shell, for a command the person copies.
fn quoted(path: &Path) -> String {
    crate::ops::shell_quote(&path.to_string_lossy())
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
                match restore_file(&backup, &from) {
                    Ok(None) => ui.ok(&format!("{name}: restored {}", from.display())),
                    Ok(Some(x)) => ui.ok(&format!(
                        "{name}: restored {}; could not remove its backup {}: {x}",
                        from.display(),
                        backup.display()
                    )),
                    Err(x) => {
                        let parent = from.parent().unwrap_or_else(|| Path::new("."));
                        failures.push(fail(
                            ui,
                            name,
                            format!(
                                "cannot restore {}: {x}; backup kept at {}; run: mkdir -p -- {} && cp -- {} {}",
                                from.display(),
                                backup.display(),
                                quoted(parent),
                                quoted(&backup),
                                quoted(&from)
                            ),
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
                    &format!("cp -- {} {}", quoted(&backup), quoted(&from)),
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
    let status = match (catalog_status, recorded) {
        (Some(status), _) => status,
        (None, Some(raw)) => Status::try_from(raw.clone())
            .map_err(|e| format!("the recorded status query is invalid: {e}"))?,
        (None, None) => {
            return Err("no status query is declared or recorded for this integration".to_string());
        }
    };
    let Some(bin) = bin else {
        return Err(format!(
            "the binary of {name} is gone, so the integration cannot be undone or checked"
        ));
    };
    let code = run_tool(ui, undo_argv, cwd)?;
    let state = owner::run_status(&status, bin, cwd);
    match state {
        Answer::Absent => {
            ui.ok(&format!("{name}: integration removed"));
            Ok(())
        }
        Answer::Unknown => {
            let mut query = vec![bin
                .file_name()
                .map_or_else(|| name.to_string(), |n| n.to_string_lossy().into_owned())];
            query.extend(status.args.iter().cloned());
            Err(format!(
                "{} exited {code}, and the status query `{}` gave no answer grh recognises",
                crate::ops::command_line(undo_argv),
                crate::ops::command_line(&query)
            ))
        }
        Answer::Present | Answer::Partial => Err(format!(
            "{} exited {code} and the integration still reports {}",
            crate::ops::command_line(undo_argv),
            state.as_str()
        )),
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
                                            "grh uninstall {name}, once `{}` reports {} at {}",
                                            crate::ops::command_line(
                                                &std::iter::once(path.to_string())
                                                    .chain(spec.args.iter().cloned())
                                                    .collect::<Vec<_>>()
                                            ),
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
    failures.extend(remove_aliases(ui, store, sum, name, belongs, blocked));
    failures
}

/// Remove the copies of a binary grh placed under other names, once the
/// binary itself is gone: a copy changed since grh placed it is left.
fn remove_aliases(
    ui: &Ui,
    store: &mut Store,
    sum: &mut Summary,
    name: &str,
    belongs: &dyn Fn(&str) -> bool,
    blocked: bool,
) -> Vec<Exit> {
    let mut failures: Vec<Exit> = Vec::new();
    let binary_kept = store
        .receipt
        .entries
        .iter()
        .any(|e| belongs(&e.part) && matches!(e.kind, Kind::BinaryPlaced { .. }));
    let aliases: Vec<Entry> = store
        .receipt
        .entries
        .iter()
        .filter(|e| belongs(&e.part) && matches!(e.kind, Kind::BinaryAlias { .. }))
        .cloned()
        .collect();
    for e in aliases {
        let Kind::BinaryAlias { path, sha256 } = &e.kind else {
            continue;
        };
        if blocked {
            sum.gap(
                &format!("{path} kept: an integration of {name} is not confirmed removed"),
                &format!("grh uninstall {name}"),
            );
            continue;
        }
        if binary_kept {
            sum.gap(
                &format!("{path} kept: the {name} binary it copies is still in place"),
                &format!("grh uninstall {name}"),
            );
            continue;
        }
        match std::fs::read(path) {
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
                if let Err(x) = std::fs::remove_file(path) {
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
    failures
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A receipt holding `kinds` as done entries of the part `p`.
    fn receipt_with(state: &Path, kinds: Vec<Kind>) -> Store {
        let mut store = Store::open_in(state).unwrap();
        for kind in kinds {
            let idx = store.begin(Entry::machine("p", kind)).unwrap();
            store.done(idx).unwrap();
        }
        store
    }

    #[test]
    fn a_backup_is_restored_into_a_directory_the_removal_pruned() {
        let dir = tempfile::tempdir().unwrap();
        let claude = dir.path().join("claude");
        let skill = claude.join("skills/alpha/SKILL.md");
        let backup = dir.path().join("state/backup/1/skills/alpha/SKILL.md");
        std::fs::create_dir_all(skill.parent().unwrap()).unwrap();
        std::fs::create_dir_all(backup.parent().unwrap()).unwrap();
        std::fs::write(&skill, b"placed by grh").unwrap();
        std::fs::write(&backup, b"the original").unwrap();
        let mut store = receipt_with(
            &dir.path().join("state/grh"),
            vec![
                Kind::Backup {
                    path: backup.to_string_lossy().into_owned(),
                    from: skill.to_string_lossy().into_owned(),
                },
                Kind::File {
                    path: skill.to_string_lossy().into_owned(),
                    sha256: sha256_hex(b"placed by grh"),
                },
            ],
        );

        let undone = remove_extension(&Ui::new(true), &mut store, "p", &[]);

        assert!(undone.failures.is_empty(), "{:?}", undone.failures);
        assert_eq!(std::fs::read(&skill).unwrap(), b"the original");
        assert!(!backup.exists());
        assert!(store.receipt.entries.iter().all(|e| e.part != "p"));
        let left: Vec<_> = std::fs::read_dir(skill.parent().unwrap())
            .unwrap()
            .flatten()
            .map(|e| e.file_name())
            .collect();
        assert_eq!(left, vec![std::ffi::OsString::from("SKILL.md")]);
    }

    #[test]
    fn a_failed_restore_keeps_the_backup_and_names_the_command() {
        let dir = tempfile::tempdir().unwrap();
        let blocker = dir.path().join("blocker dir");
        let from = blocker.join("SKILL.md");
        let backup = dir.path().join("state/backup/SKILL.md");
        std::fs::create_dir_all(backup.parent().unwrap()).unwrap();
        std::fs::write(&backup, b"the original").unwrap();
        std::fs::write(&blocker, b"a file where the directory should be").unwrap();
        let mut store = receipt_with(
            &dir.path().join("state/grh"),
            vec![Kind::Backup {
                path: backup.to_string_lossy().into_owned(),
                from: from.to_string_lossy().into_owned(),
            }],
        );
        let entries = store.receipt.entries.clone();

        let undone = restore_backups(&Ui::new(true), &mut store, "p", &entries);

        assert_eq!(undone.failures.len(), 1);
        let Exit::Failed(text) = &undone.failures[0] else {
            panic!("{:?}", undone.failures[0]);
        };
        assert!(
            text.contains(&format!("backup kept at {}", backup.display())),
            "{text}"
        );
        let cp = format!("cp -- {} {}", quoted(&backup), quoted(&from));
        assert!(text.contains(&cp), "{text}");
        assert!(text.contains("'"), "the paths are quoted: {text}");
        assert!(backup.exists());
        assert_eq!(store.receipt.entries.len(), 1);
    }
}
