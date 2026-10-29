//! `grh install`: place each part's binary, run the tool's own install, place
//! skills and agents, and record every step in the receipt.

use std::path::{Path, PathBuf};

use crate::catalog::{Catalog, Integration, Part, Unknown};
use crate::dispatch;
use crate::exit::Exit;
use crate::fetch::{install_binary, sha256_hex};
use crate::ops::{
    absolute, confirm, git_root, listing, on_path, recorded_bin, run_tool, same_scope, subst, text,
    worst_of, Consent, Summary, Ui,
};
use crate::owner::{self, Answer};
use crate::receipt::{write_atomic, Entry, Kind, Level, State, Store};
use crate::skills::{self, Bundle};

const ATTEST_WORKFLOW: &str = "\
name: attest
on:
  pull_request:
jobs:
  attest:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: fredericrous/attest@v1
";

#[derive(Debug, Default)]
pub struct Opts {
    pub names: Vec<String>,
    pub repo: bool,
    pub replace: bool,
    pub dry_run: bool,
    pub yes: bool,
    pub quiet: bool,
}

struct Run<'a> {
    cat: &'a Catalog,
    ui: Ui,
    store: Store,
    sum: Summary,
    replace: bool,
    repo: Option<PathBuf>,
    bundle: Option<Result<Bundle, String>>,
    claude: bool,
}

fn plan(ui: &Ui, parts: &[&Part], repo: Option<&Path>) {
    ui.say("grh will install:");
    for part in parts {
        ui.say(&format!("  {}: {}", part.name, part.summary));
        if let (Some(bin), Some(v)) = (&part.bin, &part.version) {
            ui.say(&format!(
                "      binary {bin} {v}: adopt the one on PATH, else download"
            ));
        }
        if let Some(machine) = &part.machine {
            ui.say(&format!(
                "      machine: {} (unless already present)",
                machine.install.join(" ")
            ));
        }
        if let (Some(r), Some(_)) = (&part.repo_level, repo) {
            ui.say(&format!(
                "      repo: {} (unless already present)",
                r.install.join(" ")
            ));
        }
        if !part.skills.is_empty() || !part.agents.is_empty() {
            ui.say(&format!(
                "      {} skills and {} agents into the Claude config directory",
                part.skills.len(),
                part.agents.len()
            ));
        }
        if part.workflow.is_some() {
            ui.say("      the attest CI workflow (needs --repo)");
        }
        if let Some(h) = &part.hint {
            ui.say(&format!("      then by hand: {h}"));
        }
        if part.extension.is_some() {
            ui.say("      extension: not published yet");
        }
    }
}

pub fn run(cat: &Catalog, opts: &Opts) -> Result<(), Exit> {
    let ui = Ui::new(opts.quiet);
    if opts.names.is_empty() {
        return Err(Exit::Usage(listing(cat)));
    }
    let parts = cat.resolve(&opts.names).map_err(|Unknown::Name(n)| {
        Exit::Usage(format!("unknown part or profile: {n}\n{}", listing(cat)))
    })?;
    let repo = if opts.repo { Some(git_root()?) } else { None };
    plan(&ui, &parts, repo.as_deref());
    if opts.dry_run {
        ui.say("dry run: nothing changed");
        return Ok(());
    }
    if !confirm(Consent::from_yes_flag(opts.yes), "install")? {
        ui.say("nothing changed");
        return Ok(());
    }
    let mut run = Run {
        cat,
        ui,
        store: Store::open()?,
        sum: Summary::default(),
        replace: opts.replace,
        repo,
        bundle: None,
        claude: on_path("claude"),
    };
    let mut failures: Vec<Exit> = Vec::new();
    for part in &parts {
        run.ui.say(&part.name);
        failures.extend(run.part(part));
    }
    run.sum.print(&run.ui);
    match worst_of(failures) {
        None => Ok(()),
        Some(e) => Err(e),
    }
}

impl Run<'_> {
    /// Install one part; every failure is reported and returned.
    fn part(&mut self, part: &Part) -> Vec<Exit> {
        let mut failures: Vec<Exit> = Vec::new();
        let mut note = |ui: &Ui, e: Exit| {
            ui.fail(&format!("{}: {}", part.name, text(&e)));
            failures.push(e);
        };
        let bin = if part.bin.is_some() {
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
            if let Some(machine) = &part.machine {
                if let Err(e) = self.integrate(part, Level::Machine, None, bin, machine) {
                    note(&self.ui, e);
                }
            }
            if let (Some(r), Some(repo)) = (&part.repo_level, repo) {
                if let Err(e) = self.integrate(part, Level::Repo, Some(&repo), bin, r) {
                    note(&self.ui, e);
                }
            }
        }
        if !part.skills.is_empty() || !part.agents.is_empty() {
            if let Err(e) = self.place_skills(part) {
                note(&self.ui, e);
            }
        }
        if let Some(w) = &part.workflow {
            if let Err(e) = self.workflow(part, w) {
                note(&self.ui, e);
            }
        }
        if let Some(h) = &part.hint {
            self.ui.warn(&format!("{}: {h}", part.name));
        }
        if part.extension.is_some() {
            let msg = format!("{} is an extension that is not published yet", part.name);
            self.sum.gap(&msg, "grh install again once it is published");
            note(&self.ui, Exit::Unpublished(msg));
        }
        failures
    }

    /// The part's binary: the receipt's, else one on PATH (adopted), else
    /// fetched and placed.
    fn binary(&mut self, part: &Part) -> Result<PathBuf, Exit> {
        let (Some(name), Some(repo), Some(version)) = (&part.bin, &part.repo, &part.version) else {
            return Err(Exit::Failed(format!("{} names no binary", part.name)));
        };
        if let Some((path, placed)) = recorded_bin(&self.store.receipt, &part.name) {
            let done = self.store.receipt.entries.iter().any(|e| {
                e.part == part.name
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
                e.part == part.name
                    && matches!(
                        e.kind,
                        Kind::BinaryPlaced { .. } | Kind::BinaryAdopted { .. }
                    )
            })?;
        }
        if let Some(found) = dispatch::find(name) {
            let path = absolute(&found);
            let idx = self.store.begin(Entry::machine(
                &part.name,
                Kind::BinaryAdopted {
                    path: path.to_string_lossy().into_owned(),
                },
            ))?;
            self.store.done(idx)?;
            self.ui.ok(&format!("{name}: adopted {}", path.display()));
            return Ok(path);
        }
        let dir = dispatch::bin_dir().map(|d| absolute(&d)).ok_or_else(|| {
            Exit::Failed("cannot find a bin directory: set HOME or GRH_BIN_DIR".into())
        })?;
        let idx = self.store.begin(Entry::machine(
            &part.name,
            Kind::BinaryPlaced {
                path: dir.join(name).to_string_lossy().into_owned(),
                sha256: String::new(),
            },
        ))?;
        self.ui.say(&format!("  fetching {name} {version}"));
        let placed = match install_binary(repo, name, version, &part.sha256, &dir) {
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
            e.part == part.name
                && e.state == State::Pending
                && matches!(&e.kind, Kind::BinaryPlaced { path, .. }
                    if Path::new(path).starts_with(&bin_root) && Path::new(path).is_file())
        });
        let Some(idx) = found else {
            return Ok(None);
        };
        let Kind::BinaryPlaced { path, .. } = &self.store.receipt.entries[idx].kind else {
            return Ok(None);
        };
        let path = path.clone();
        let bytes =
            std::fs::read(&path).map_err(|e| Exit::Failed(format!("cannot read {path}: {e}")))?;
        self.store.receipt.entries[idx].kind = Kind::BinaryPlaced {
            path: path.clone(),
            sha256: sha256_hex(&bytes),
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
        integ: &Integration,
    ) -> Result<(), Exit> {
        let which = if level == Level::Machine {
            "machine"
        } else {
            "repo"
        };
        let name = &part.name;
        match owner::query(&integ.query, bin, repo) {
            Answer::Unknown => {
                self.sum.gap(
                    &format!(
                        "{name}: the {which} integration state is unknown ({}); left alone",
                        integ.query
                    ),
                    &format!("check the tool, then grh install {name}"),
                );
                self.ui
                    .warn(&format!("{name}: {which} integration unknown, skipped"));
                Ok(())
            }
            Answer::Present => {
                let known = self.store.receipt.entries.iter().any(|e| {
                    same_scope(e, name, level, repo)
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
                    same_scope(e, name, level, repo)
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
                if level == Level::Machine && !self.claude {
                    self.sum.gap(
                        &format!("{name}: claude is not on PATH, so its Claude Code integration was not installed"),
                        &format!("install Claude Code, then grh install {name}"),
                    );
                    self.ui
                        .warn(&format!("{name}: claude not found, integration skipped"));
                    return Ok(());
                }
                self.store.remove_where(|e| {
                    same_scope(e, name, level, repo)
                        && matches!(e.kind, Kind::ToolInstall { .. } | Kind::ToolInstallAdopted)
                })?;
                let argv = subst(&integ.install, bin);
                let undo_argv = subst(&integ.uninstall, bin);
                let kind = Kind::ToolInstall {
                    argv: argv.clone(),
                    undo_argv,
                };
                let entry = match repo {
                    Some(r) => Entry::in_repo(name, r, kind),
                    None => Entry::machine(name, kind),
                };
                let idx = self.store.begin(entry)?;
                let code = run_tool(&self.ui, &argv, repo).map_err(Exit::Failed)?;
                if owner::query(&integ.query, bin, repo) == Answer::Present {
                    self.store.done(idx)?;
                    self.ui
                        .ok(&format!("{name}: {which} integration installed"));
                    Ok(())
                } else {
                    // Left pending: the step may have written part of it, and
                    // uninstall will run the undo and check.
                    Err(Exit::Failed(format!(
                        "{} exited {code} and the {which} integration is not confirmed present",
                        argv.join(" ")
                    )))
                }
            }
        }
    }

    fn place_skills(&mut self, part: &Part) -> Result<(), Exit> {
        if self.bundle.is_none() {
            self.bundle = Some(Bundle::fetch(self.cat).map_err(|e| text(&e)));
        }
        let bundle = match &self.bundle {
            Some(Ok(b)) => b,
            Some(Err(m)) => return Err(Exit::Failed(m.clone())),
            None => return Ok(()),
        };
        let claude = skills::claude_dir()?;
        let placement = skills::place(bundle, part, &claude, &mut self.store, self.replace)?;
        let name = &part.name;
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

    fn workflow(&mut self, part: &Part, kind: &str) -> Result<(), Exit> {
        if kind != "attest" {
            return Err(Exit::Failed(format!("unknown workflow {kind}")));
        }
        let Some(repo) = self.repo.clone() else {
            self.sum.gap(
                &format!(
                    "{}: the CI workflow is written inside a repository",
                    part.name
                ),
                "grh install attest --repo, from the repository",
            );
            return Ok(());
        };
        let dir = if repo.join(".forgejo").is_dir() {
            ".forgejo"
        } else {
            ".github"
        };
        let path = repo.join(dir).join("workflows/attest.yml");
        let bytes = ATTEST_WORKFLOW.as_bytes();
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
                write = false;
            } else if recorded.as_deref() != Some(cur_sha.as_str()) {
                if !self.replace {
                    self.sum.skip(
                        &format!("{want}: exists and grh did not place it"),
                        "grh install attest --repo --replace",
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
                    .join("workflows/attest.yml");
                let idx = self.store.begin(Entry::in_repo(
                    &part.name,
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
                    "grh uninstall attest restores it",
                );
            }
        }
        if !write && recorded.as_deref() == Some(new_sha.as_str()) {
            self.ui.ok(&format!("{}: {want} (unchanged)", part.name));
            return Ok(());
        }
        self.store
            .remove_where(|e| matches!(&e.kind, Kind::File { path, .. } if *path == want))?;
        let idx = self.store.begin(Entry::in_repo(
            &part.name,
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
        self.ui.ok(&format!("{}: wrote {want}", part.name));
        Ok(())
    }
}
