//! `grh install`: place each part's binary, run the tool's own install, place
//! skills and agents, and record every step in the receipt.

use std::path::{Path, PathBuf};

use crate::audit;
use crate::catalog::{Catalog, Part, Unknown, Verb};
use crate::checklist;
use crate::dispatch;
use crate::exit::Exit;
use crate::fetch::{install_binary, sha256_hex};
use crate::manifest::{stays_inside, RepoFile, ToolLevel};
use crate::ops::{
    absolute, command_line, confirm_rerun, git_root, listing, on_path, recorded_bin, run_tool,
    same_scope, subst, text, worst_of, Consent, Summary, Ui,
};
use crate::owner::{self, Answer};
use crate::receipt::{
    self, write_atomic, Entry, Kind, Level, RecordedKeepBinary, RecordedStatus, State, Store,
};
use crate::skills::{self, Bundle};

#[derive(Debug, Default)]
pub struct Opts {
    pub names: Vec<String>,
    pub all: bool,
    pub repo: bool,
    pub replace: bool,
    pub dry_run: bool,
    pub yes: bool,
    pub quiet: bool,
}

/// One install in progress: the receipt it writes and what it reports.
#[derive(Debug)]
pub struct Run<'a> {
    cat: Option<&'a Catalog>,
    foreign_markers: &'a [String],
    pub ui: Ui,
    pub store: Store,
    pub sum: Summary,
    replace: bool,
    repo: Option<PathBuf>,
    bundle: Option<Result<Bundle, String>>,
}

/// What the person is shown before agreeing to a manifest the catalog does
/// not pin: the binary it would download (from a repository and version the
/// same manifest names, checked against a sha256 it also supplies), every
/// command it would run, every file path it would write and every host
/// command it needs. On stderr, even under `--quiet`.
pub fn show_commands(part: &Part) {
    let line = |what: &str, argv: &[String]| {
        eprintln!("  {what}: {}", command_line(argv));
    };
    let bin = part.bin().map(|b| b.to_string());
    let tool_bin = |argv: &[String]| -> Vec<String> {
        argv.iter()
            .map(|a| a.replace("{bin}", bin.as_deref().unwrap_or("{bin}")))
            .collect()
    };
    eprintln!(
        "{} is not pinned by grh's catalog; it would run and use:",
        part.name()
    );
    if let Some(release) = part.release() {
        eprintln!(
            "  download: {} {} from github.com/{} (when it is not already on PATH)",
            bin.as_deref().unwrap_or("{bin}"),
            release.version,
            release.repo
        );
    }
    for (level, spec) in [("machine", part.machine()), ("repo", part.repo_level())] {
        let Some(spec) = spec else { continue };
        line(&format!("{level} install"), &tool_bin(&spec.install));
        line(&format!("{level} uninstall"), &tool_bin(&spec.uninstall));
        if let Some(status) = &spec.status {
            let mut argv = vec![bin.clone().unwrap_or_default()];
            argv.extend(status.args.iter().cloned());
            line(&format!("{level} status"), &argv);
        }
        for need in &spec.requires {
            eprintln!(
                "  {level} requires: {}",
                command_line(std::slice::from_ref(need))
            );
        }
    }
    if let Some(keep) = part.keep_binary() {
        let mut argv = vec![bin.clone().unwrap_or_default()];
        argv.extend(keep.args.iter().cloned());
        line("keep-binary", &argv);
    }
    for check in &part.doctor {
        line("doctor", std::slice::from_ref(&check.command));
    }
    for file in &part.repo_file {
        for dir in &file.dirs {
            eprintln!(
                "  repo file: {}",
                command_line(&[format!("{dir}/{}", file.path)])
            );
        }
    }
}

impl<'a> Run<'a> {
    pub fn into_parts(self) -> (Ui, Store, Summary) {
        (self.ui, self.store, self.sum)
    }

    pub fn new(
        cat: Option<&'a Catalog>,
        foreign_markers: &'a [String],
        ui: Ui,
        store: Store,
        replace: bool,
        repo: Option<PathBuf>,
        bundle: Option<Bundle>,
    ) -> Run<'a> {
        Run {
            cat,
            foreign_markers,
            ui,
            store,
            sum: Summary::default(),
            replace,
            repo,
            bundle: bundle.map(Ok),
        }
    }
}

/// `argv` with `{bin}` replaced by the part's binary name, as the plan shows it.
fn bin_substituted(part: &Part, argv: &[String]) -> Vec<String> {
    match part.bin() {
        Some(bin) => subst(argv, Path::new(bin)),
        None => argv.to_vec(),
    }
}

fn plan(ui: &Ui, parts: &[&Part], repo: Option<&Path>) {
    ui.say("grh will install:");
    for part in parts {
        ui.say(&format!("  {}: {}", part.name(), part.summary()));
        if let (Some(bin), Some(release)) = (part.bin(), part.release()) {
            let v = &release.version;
            ui.say(&format!(
                "      binary {bin} {v}: adopt the one on PATH, else download"
            ));
        }
        if let Some(machine) = part.machine() {
            ui.say(&format!(
                "      machine: {} (unless already present)",
                command_line(&bin_substituted(part, &machine.install))
            ));
        }
        if let (Some(r), Some(_)) = (part.repo_level(), repo) {
            ui.say(&format!(
                "      repo: {} (unless already present)",
                command_line(&bin_substituted(part, &r.install))
            ));
        }
        if !part.bundle_skills().is_empty() || !part.bundle_agents().is_empty() {
            ui.say(&format!(
                "      {} skills and {} agents into the host config directory",
                part.bundle_skills().len(),
                part.bundle_agents().len()
            ));
        }
        for file in &part.repo_file {
            ui.say(&format!("      the CI file {} (needs --repo)", file.path));
        }
        if let Some(h) = part.hint() {
            ui.say(&format!("      then by hand: {h}"));
        }
        if !part.is_published() {
            ui.say("      extension: not published yet");
        }
    }
}

/// A bare `grh install` on a terminal: the checklist, then the install it
/// stands for, run as if typed.
fn choose_and_run(cat: &Catalog, opts: &Opts, ui: &Ui) -> Result<(), Exit> {
    let assessment = audit::assess(cat, None)?;
    let mut receipt = receipt::read_in(&receipt::state_dir()?)?;
    receipt.rename_parts(|name| cat.name_of_alias(name));
    let installed: Vec<String> = receipt.entries.iter().map(|e| e.part.clone()).collect();
    let model = checklist::Checklist::new(cat, assessment.as_ref(), &installed);
    match checklist::open(model)? {
        checklist::Opened::Unavailable => Err(Exit::Usage(listing(cat))),
        checklist::Opened::TooSmall => Err(Exit::Usage(format!(
            "{}\n{}",
            checklist::TOO_SMALL,
            listing(cat)
        ))),
        checklist::Opened::Cancelled => {
            ui.say("nothing changed");
            Ok(())
        }
        checklist::Opened::Confirmed { parts, repo } => {
            eprintln!(
                "grh install {}{}",
                parts.join(" "),
                if repo { " --repo" } else { "" }
            );
            run(
                cat,
                &Opts {
                    names: parts,
                    all: false,
                    repo,
                    replace: opts.replace,
                    dry_run: opts.dry_run,
                    yes: false,
                    quiet: opts.quiet,
                },
            )
        }
    }
}

pub fn run(cat: &Catalog, opts: &Opts) -> Result<(), Exit> {
    if let Some(notice) = cat.retired_notice(&opts.names, Verb::Install) {
        return Err(Exit::Usage(notice));
    }
    if opts.all && !opts.names.is_empty() {
        return Err(Exit::Usage(
            "--all installs every published part but the experiments; name no parts with it".into(),
        ));
    }
    let ui = Ui::new(opts.quiet);
    if opts.names.is_empty() && !opts.all {
        if !opts.yes && checklist::terminal_available() {
            return choose_and_run(cat, opts, &ui);
        }
        return Err(Exit::Usage(listing(cat)));
    }
    cat.warn_aliases(&ui, &opts.names, Verb::Install);
    let wanted = if opts.all {
        cat.all_names()
    } else {
        opts.names.clone()
    };
    let parts = cat
        .resolve(&wanted)
        .map_err(|Unknown::Name(n)| Exit::Usage(format!("unknown part: {n}\n{}", listing(cat))))?;
    let repo = if opts.repo { Some(git_root()?) } else { None };
    plan(&ui, &parts, repo.as_deref());
    let left_out = cat.experimental_names();
    if opts.all && !left_out.is_empty() {
        ui.say(&format!(
            "left out: {} (an experiment: install it by name)",
            left_out.join(", ")
        ));
    }
    if opts.dry_run {
        ui.say("dry run: nothing changed");
        return Ok(());
    }
    // What to type again: with `--all` the whole command, built from the flags
    // actually given, so a copied hint does not drop `--repo`, `--replace` or
    // `--quiet`.
    let rerun = if opts.all {
        format!(
            "grh install --all{}{}{} --yes",
            if opts.repo { " --repo" } else { "" },
            if opts.replace { " --replace" } else { "" },
            if opts.quiet { " --quiet" } else { "" }
        )
    } else {
        "--yes".to_string()
    };
    if !confirm_rerun(Consent::from_yes_flag(opts.yes), "install", &rerun)? {
        ui.say("nothing changed");
        return Ok(());
    }
    let store = Store::open_migrating(|name| cat.name_of_alias(name))?;
    let mut run = Run::new(
        Some(cat),
        &cat.foreign_markers,
        ui,
        store,
        opts.replace,
        repo,
        None,
    );
    let mut failures: Vec<Exit> = Vec::new();
    let mut failed: Vec<&str> = Vec::new();
    let mut skipped: Vec<&str> = Vec::new();
    for part in &parts {
        run.ui.say(part.name());
        if opts.all {
            let missing = part
                .needs()
                .iter()
                .filter_map(|n| cat.part(n))
                .find(|need| failed.contains(&need.name()) || skipped.contains(&need.name()));
            if let Some(need) = missing {
                run.ui.fail(&format!(
                    "{}: skipped, because {} did not install",
                    part.name(),
                    need.name()
                ));
                skipped.push(part.name());
                continue;
            }
        }
        let part_failures = run.part(part);
        if !part_failures.is_empty() {
            failed.push(part.name());
        }
        failures.extend(part_failures);
    }
    run.sum.print(&run.ui);
    if opts.all && !failed.is_empty() {
        let skipped = if skipped.is_empty() {
            String::new()
        } else {
            format!("; skipped because of it: {}", skipped.join(", "))
        };
        return Err(Exit::Failed(format!(
            "{} did not install{skipped}; the other parts are installed. Fix the errors above, then run {rerun}",
            failed.join(", "),
        )));
    }
    match worst_of(failures) {
        None => Ok(()),
        Some(e) => Err(e),
    }
}

impl Run<'_> {
    /// Install one part; every failure is reported and returned.
    pub fn part(&mut self, part: &Part) -> Vec<Exit> {
        let mut failures: Vec<Exit> = Vec::new();
        let mut note = |ui: &Ui, e: Exit| {
            ui.fail(&format!("{}: {}", part.name(), text(&e)));
            failures.push(e);
        };
        let bin = if part.bin().is_some() {
            match self.binary(part) {
                Ok(b) => Some(b),
                Err(e) => {
                    note(&self.ui, e);
                    None
                }
            }
        } else {
            None
        };
        if let Some(bin) = &bin {
            let repo = self.repo.clone();
            if let Some(machine) = part.machine() {
                if let Err(e) = self.integrate(part, Level::Machine, None, bin, machine) {
                    note(&self.ui, e);
                }
            }
            if let (Some(r), Some(repo)) = (part.repo_level(), repo) {
                if let Err(e) = self.integrate(part, Level::Repo, Some(&repo), bin, r) {
                    note(&self.ui, e);
                }
            }
        }
        // A part whose tool is not on the machine places neither its skills nor
        // its repository files: they would point at a tool that is not there,
        // and the receipt would record a part that did not install. When the
        // binary is there but an integration step failed, what was placed stays
        // on record, so `grh uninstall` can undo it.
        let binary_missing = part.bin().is_some() && bin.is_none();
        if !binary_missing {
            if !part.bundle_skills().is_empty() || !part.bundle_agents().is_empty() {
                if let Err(e) = self.place_skills(part) {
                    note(&self.ui, e);
                }
            }
            for file in &part.repo_file {
                if let Err(e) = self.repo_file(part, file) {
                    note(&self.ui, e);
                }
            }
        }
        if let Some(h) = part.hint() {
            self.ui.warn(&format!("{}: {h}", part.name()));
        }
        if !part.is_published() {
            let msg = format!("{} is an extension that is not published yet", part.name());
            self.sum.gap(&msg, "grh install again once it is published");
            note(&self.ui, Exit::Unpublished(msg));
        }
        failures
    }

    /// The part's binary: the receipt's, else one on PATH (adopted), else
    /// fetched and placed.
    fn binary(&mut self, part: &Part) -> Result<PathBuf, Exit> {
        let Some(name) = part.bin() else {
            return Err(Exit::Failed(format!("{} names no binary", part.name())));
        };
        if let Some((path, placed)) = recorded_bin(&self.store.receipt, part) {
            let done = self.store.receipt.entries.iter().any(|e| {
                part.answers_to(&e.part)
                    && matches!(
                        e.kind,
                        Kind::BinaryPlaced { .. } | Kind::BinaryAdopted { .. }
                    )
                    && e.state == State::Done
            });
            if !done {
                if let Some(promoted) = self.promote_placed(part)? {
                    self.ui.ok(&format!(
                        "{name}: {} (placed by grh, recovered)",
                        promoted.display()
                    ));
                    return Ok(promoted);
                }
            }
            if done && path.exists() {
                let how = if placed { "placed by grh" } else { "adopted" };
                self.ui
                    .ok(&format!("{name}: {} ({how}, unchanged)", path.display()));
                return Ok(path);
            }
            self.store.remove_where(|e| {
                part.answers_to(&e.part)
                    && matches!(
                        e.kind,
                        Kind::BinaryPlaced { .. } | Kind::BinaryAdopted { .. }
                    )
            })?;
        }
        if let Some(found) = dispatch::find(name) {
            let path = absolute(&found);
            let idx = self.store.begin(Entry::machine(
                part.name(),
                Kind::BinaryAdopted {
                    path: path.to_string_lossy().into_owned(),
                },
            ))?;
            self.store.done(idx)?;
            self.ui.ok(&format!("{name}: adopted {}", path.display()));
            return Ok(path);
        }
        let Some(release) = part.release() else {
            return Err(Exit::Failed(format!(
                "{name} is not on PATH and {} names no release to fetch it from",
                part.name()
            )));
        };
        let (repo, version) = (&release.repo, &release.version);
        let keep_binary = part
            .keep_binary()
            .map(|spec| Box::new(RecordedKeepBinary::from(spec)));
        let dir = dispatch::bin_dir().map(|d| absolute(&d)).ok_or_else(|| {
            Exit::Failed("cannot find a bin directory: set HOME or GRH_BIN_DIR".into())
        })?;
        let idx = self.store.begin(Entry::machine(
            part.name(),
            Kind::BinaryPlaced {
                path: dir.join(name).to_string_lossy().into_owned(),
                sha256: String::new(),
                keep_binary: keep_binary.clone(),
            },
        ))?;
        self.ui.say(&format!("  fetching {name} {version}"));
        let placed = match install_binary(repo, name, version, &release.sha256, &dir) {
            Ok(p) => p,
            Err(e) => {
                // Nothing was written: the pending step did not happen.
                self.store.receipt.entries.remove(idx);
                self.store.save()?;
                return Err(e);
            }
        };
        self.store.receipt.entries[idx].kind = Kind::BinaryPlaced {
            path: placed.path.to_string_lossy().into_owned(),
            sha256: placed.sha256,
            keep_binary,
        };
        self.store.done(idx)?;
        self.ui
            .ok(&format!("{name}: placed {}", placed.path.display()));
        Ok(placed.path)
    }

    /// A pending binary-placed row whose file is in grh's bin dir: the step
    /// finished but was not marked. Record the file's sha and mark it done.
    fn promote_placed(&mut self, part: &Part) -> Result<Option<PathBuf>, Exit> {
        let Some(bin_root) = dispatch::bin_dir().map(|d| absolute(&d)) else {
            return Ok(None);
        };
        let found = self.store.receipt.entries.iter().position(|e| {
            part.answers_to(&e.part)
                && e.state == State::Pending
                && matches!(&e.kind, Kind::BinaryPlaced { path, .. }
                    if Path::new(path).starts_with(&bin_root) && Path::new(path).is_file())
        });
        let Some(idx) = found else {
            return Ok(None);
        };
        let Kind::BinaryPlaced {
            path, keep_binary, ..
        } = &self.store.receipt.entries[idx].kind
        else {
            return Ok(None);
        };
        let (path, keep_binary) = (path.clone(), keep_binary.clone());
        let bytes =
            std::fs::read(&path).map_err(|e| Exit::Failed(format!("cannot read {path}: {e}")))?;
        self.store.receipt.entries[idx].kind = Kind::BinaryPlaced {
            path: path.clone(),
            sha256: sha256_hex(&bytes),
            keep_binary,
        };
        self.store.done(idx)?;
        Ok(Some(PathBuf::from(path)))
    }

    /// Run the tool's own install for one level, if its query allows it.
    fn integrate(
        &mut self,
        part: &Part,
        level: Level,
        repo: Option<&Path>,
        bin: &Path,
        integ: &ToolLevel,
    ) -> Result<(), Exit> {
        let which = if level == Level::Machine {
            "machine"
        } else {
            "repo"
        };
        let name = part.name();
        // A host command the level needs (a CLI it registers with) must be
        // on PATH: without it the level can be neither placed nor queried.
        if let Some(missing) = integ.requires.iter().find(|c| !on_path(c)) {
            self.sum.gap(
                &format!(
                    "{name}: {missing} is not on PATH, so its {which} integration was not installed"
                ),
                &if level == Level::Repo {
                    format!(
                        "install {missing}, then grh install {name} --repo, from the repository"
                    )
                } else {
                    format!("install {missing}, then grh install {name}")
                },
            );
            self.ui.warn(&format!(
                "{name}: {missing} not found, {which} integration skipped"
            ));
            return Ok(());
        }
        let answer = owner::level_state(integ, bin, repo);
        match answer {
            Answer::Unknown => {
                self.sum.gap(
                    &format!(
                        "{name}: the {which} integration state is unknown ({}); left alone",
                        status_command(part, integ)
                    ),
                    &format!("check the tool, then grh install {name}"),
                );
                self.ui
                    .warn(&format!("{name}: {which} integration unknown, skipped"));
                Ok(())
            }
            Answer::Present => {
                let known = self.store.receipt.entries.iter().any(|e| {
                    same_scope(e, part, level, repo)
                        && matches!(e.kind, Kind::ToolInstall { .. } | Kind::ToolInstallAdopted)
                        && e.state == State::Done
                });
                if known {
                    self.ui
                        .ok(&format!("{name}: {which} integration present (unchanged)"));
                    return Ok(());
                }
                // A step an earlier run left pending, now confirmed present.
                let pending = self.store.receipt.entries.iter().position(|e| {
                    same_scope(e, part, level, repo)
                        && matches!(e.kind, Kind::ToolInstall { .. } | Kind::ToolInstallAdopted)
                        && e.state == State::Pending
                });
                if let Some(idx) = pending {
                    self.store.done(idx)?;
                    self.ui
                        .ok(&format!("{name}: {which} integration present (recovered)"));
                    return Ok(());
                }
                let entry = match repo {
                    Some(r) => Entry::in_repo(name, r, Kind::ToolInstallAdopted),
                    None => Entry::machine(name, Kind::ToolInstallAdopted),
                };
                let idx = self.store.begin(entry)?;
                self.store.done(idx)?;
                self.ui.ok(&format!(
                    "{name}: {which} integration already present (adopted)"
                ));
                Ok(())
            }
            Answer::Absent | Answer::Partial => {
                if answer == Answer::Partial {
                    // Partial is installed by someone else unless a done
                    // ToolInstall says grh placed this scope.
                    let ours = self.store.receipt.entries.iter().any(|e| {
                        same_scope(e, part, level, repo)
                            && matches!(e.kind, Kind::ToolInstall { .. })
                            && e.state == State::Done
                    });
                    if !ours {
                        self.sum.gap(
                            &format!(
                                "{name}: the {which} integration is partly installed by someone else; left alone"
                            ),
                            &format!("finish or remove it with {name}'s own commands, then grh install {name}"),
                        );
                        self.ui.warn(&format!(
                            "{name}: {which} integration partial, partly installed by someone else, left alone"
                        ));
                        return Ok(());
                    }
                }
                self.store.remove_where(|e| {
                    same_scope(e, part, level, repo)
                        && matches!(e.kind, Kind::ToolInstall { .. } | Kind::ToolInstallAdopted)
                })?;
                let argv = subst(&integ.install, bin);
                let undo_argv = subst(&integ.uninstall, bin);
                let kind = Kind::ToolInstall {
                    argv: argv.clone(),
                    undo_argv,
                    status: integ.status.as_ref().map(RecordedStatus::from),
                };
                let entry = match repo {
                    Some(r) => Entry::in_repo(name, r, kind),
                    None => Entry::machine(name, kind),
                };
                let idx = self.store.begin(entry)?;
                let code = run_tool(&self.ui, &argv, repo).map_err(Exit::Failed)?;
                if owner::level_state(integ, bin, repo) == Answer::Present {
                    self.store.done(idx)?;
                    self.ui
                        .ok(&format!("{name}: {which} integration installed"));
                    Ok(())
                } else {
                    // Left pending: the step may have written part of it, and
                    // uninstall will run the undo and check.
                    Err(Exit::Failed(format!(
                        "{} exited {code} and the {which} integration is not confirmed present",
                        command_line(&argv)
                    )))
                }
            }
        }
    }

    fn place_skills(&mut self, part: &Part) -> Result<(), Exit> {
        if self.bundle.is_none() {
            self.bundle = Some(match self.cat {
                Some(cat) => Bundle::fetch(cat).map_err(|e| text(&e)),
                None => Err(format!("{} names no skills bundle to fetch", part.name())),
            });
        }
        let bundle = match &self.bundle {
            Some(Ok(b)) => b,
            Some(Err(m)) => return Err(Exit::Failed(m.clone())),
            None => return Ok(()),
        };
        let claude = skills::claude_dir()?;
        let placement = skills::place(
            bundle,
            part,
            &claude,
            &mut self.store,
            self.replace,
            self.foreign_markers,
        )?;
        let name = part.name();
        self.ui.ok(&format!(
            "{name}: {} skill and agent files in {}",
            placement.placed.len(),
            claude.display()
        ));
        for s in &placement.skipped {
            self.sum.skip(
                &format!("{}: {}", s.path.display(), s.reason),
                &format!("grh install {name} --replace"),
            );
        }
        for (backup, original) in &placement.backups {
            self.sum.backup(
                &format!(
                    "{} was backed up to {}",
                    original.display(),
                    backup.display()
                ),
                &format!("grh uninstall {name} restores it when the original is gone"),
            );
        }
        Ok(())
    }

    /// Place a file the manifest lists in the repository: in the first of its
    /// `dirs` that exists, else the last.
    fn repo_file(&mut self, part: &Part, file: &RepoFile) -> Result<(), Exit> {
        let Some(repo) = self.repo.clone() else {
            self.sum.gap(
                &format!(
                    "{}: {} is written inside a repository",
                    part.name(),
                    file.path
                ),
                &format!("grh install {} --repo, from the repository", part.name()),
            );
            return Ok(());
        };
        let dir = file
            .dirs
            .iter()
            .find(|d| repo.join(d).is_dir())
            .or(file.dirs.last())
            .ok_or_else(|| Exit::Failed(format!("{} lists no directory", file.path)))?;
        let path = place_inside(&repo, dir, &file.path)?;
        let bytes = file.content.as_bytes();
        let new_sha = sha256_hex(bytes);
        let want = path.to_string_lossy().into_owned();
        let recorded = self
            .store
            .receipt
            .entries
            .iter()
            .find_map(|e| match &e.kind {
                Kind::File { path, sha256 } if *path == want && e.level == Level::Repo => {
                    Some(sha256.clone())
                }
                _ => None,
            });
        let mut write = true;
        if let Ok(cur) = std::fs::read(&path) {
            let cur_sha = sha256_hex(&cur);
            if cur_sha == new_sha {
                if recorded.is_none() {
                    // Identical bytes do not make it ours: someone else placed it.
                    self.sum.skip(
                        &format!("{want}: exists and grh did not place it"),
                        &format!(
                            "leave it, or remove it and grh install {} --repo",
                            part.name()
                        ),
                    );
                    return Ok(());
                }
                write = false;
            } else if recorded.as_deref() != Some(cur_sha.as_str()) {
                if !self.replace {
                    self.sum.skip(
                        &format!("{want}: exists and grh did not place it"),
                        &format!("grh install {} --repo --replace", part.name()),
                    );
                    return Ok(());
                }
                let ts = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                let to = self
                    .store
                    .dir()
                    .join("backup")
                    .join(ts.to_string())
                    .join(dir)
                    .join(&file.path);
                let idx = self.store.begin(Entry::in_repo(
                    part.name(),
                    &repo,
                    Kind::Backup {
                        path: to.to_string_lossy().into_owned(),
                        from: want.clone(),
                    },
                ))?;
                write_atomic(&to, &cur, 0o644)?;
                self.store.done(idx)?;
                self.sum.backup(
                    &format!("{want} was backed up to {}", to.display()),
                    &format!("grh uninstall {} restores it", part.name()),
                );
            }
        }
        if !write && recorded.as_deref() == Some(new_sha.as_str()) {
            self.ui.ok(&format!("{}: {want} (unchanged)", part.name()));
            return Ok(());
        }
        self.store
            .remove_where(|e| matches!(&e.kind, Kind::File { path, .. } if *path == want))?;
        let idx = self.store.begin(Entry::in_repo(
            part.name(),
            &repo,
            Kind::File {
                path: want.clone(),
                sha256: new_sha,
            },
        ))?;
        if write {
            write_atomic(&path, bytes, 0o644)?;
        }
        self.store.done(idx)?;
        self.ui.ok(&format!("{}: wrote {want}", part.name()));
        Ok(())
    }
}

/// The command a level's `status` runs, as the person would type it
/// (`<bin> <args>`), or `<bin>: no status declared` when the level has none.
fn status_command(part: &Part, level: &ToolLevel) -> String {
    let bin = part.bin().unwrap_or_default();
    match &level.status {
        Some(status) => {
            let mut argv = vec![bin.to_string()];
            argv.extend(status.args.iter().cloned());
            command_line(&argv)
        }
        None => format!("{bin}: no status declared"),
    }
}

/// `<repo>/<dir>/<rel>`, refused unless both parts are relative and stay
/// inside, and the deepest part of it that exists resolves, symlinks
/// followed, to somewhere under the repository's real path.
fn place_inside(repo: &Path, dir: &str, rel: &str) -> Result<PathBuf, Exit> {
    let refuse = |what: &str| {
        Exit::Failed(format!(
            "{dir}/{rel}: {what}; a repository file is written only inside the repository"
        ))
    };
    if !stays_inside(dir) || !stays_inside(rel) {
        return Err(refuse("the path is not relative or leaves the repository"));
    }
    let root = std::fs::canonicalize(repo)
        .map_err(|e| Exit::Failed(format!("cannot resolve {}: {e}", repo.display())))?;
    let target = repo.join(dir).join(rel);
    let mut probe = target.as_path();
    loop {
        if std::fs::symlink_metadata(probe).is_ok() {
            let real = std::fs::canonicalize(probe)
                .map_err(|_| refuse("a link on the path does not resolve"))?;
            if real.starts_with(&root) {
                return Ok(target);
            }
            return Err(refuse("a link on the path points outside the repository"));
        }
        probe = probe
            .parent()
            .ok_or_else(|| refuse("no part of the path exists"))?;
    }
}
