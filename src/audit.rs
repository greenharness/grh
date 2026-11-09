//! `grh audit [dir] [--json]`: read a repository and recommend parts. The
//! rules and conflicts are each part's own manifest data and the path lists
//! are the catalog's; this module only computes the signals and evaluates
//! them. Reads only: it runs tools' own read-only queries and never reads a
//! tool's files to learn what it installed.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::{json, Value};

use crate::catalog::{Catalog, Part};
use crate::exit::Exit;
use crate::manifest::{render, ConflictWhen, Signal, ToolLevel, Verdict};
use crate::owner::{self, Answer};

fn git(dir: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim_end().to_string())
}

/// `git config --get key`: the value, `Ok(None)` when the key is unset (git
/// exits 1), or why git could not answer.
fn git_config(dir: &Path, key: &str) -> Result<Option<String>, String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["config", "--get", key])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map_err(|e| format!("cannot run git: {e}"))?;
    match out.status.code() {
        Some(0) => Ok(Some(
            String::from_utf8_lossy(&out.stdout).trim_end().to_string(),
        )),
        Some(1) => Ok(None),
        other => Err(format!(
            "git config --get {key} failed ({})",
            other.map_or("killed".to_string(), |c| format!("exit {c}"))
        )),
    }
}

/// Every executable called `name` on PATH, distinct files only.
fn candidates(name: &str) -> Vec<PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    let path = std::env::var_os("PATH").unwrap_or_default();
    let mut found: Vec<PathBuf> = Vec::new();
    for d in std::env::split_paths(&path) {
        let p = d.join(name);
        let ok = std::fs::metadata(&p)
            .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
            .unwrap_or(false);
        if !ok {
            continue;
        }
        let real = std::fs::canonicalize(&p).unwrap_or(p);
        if !found.contains(&real) {
            found.push(real);
        }
    }
    found
}

fn language(file: &str) -> &'static str {
    let ext = Path::new(file)
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "rs" => "rust",
        "ts" | "tsx" | "mts" | "cts" | "js" | "jsx" | "mjs" | "cjs" => "typescript/javascript",
        "py" => "python",
        "go" => "go",
        _ => "other",
    }
}

/// One part's verdict and the reason it gets.
#[derive(Debug)]
pub struct Row<'a> {
    pub part: &'a str,
    pub verdict: Verdict,
    pub reason: String,
}

/// Everything the audit found in one repository; printing it is `run`'s job.
#[derive(Debug)]
pub struct Assessment<'a> {
    pub root: PathBuf,
    pub languages: BTreeMap<&'static str, usize>,
    pub rows: Vec<Row<'a>>,
    pub hooks: Vec<String>,
    pub warnings: Vec<String>,
    pub adr_folder: Option<String>,
    pub ci: Vec<String>,
    pub signals: BTreeMap<String, bool>,
    /// Recommended parts that are not experimental, in catalog order.
    pub install: Vec<&'a str>,
    /// The published experimental parts, whether recommended or not.
    pub experimental: Vec<String>,
}

/// What the audit knows about the repository, before any rule reads it.
struct Facts {
    adr_folder: Option<String>,
    claude_files: Vec<String>,
    ci: Vec<String>,
    language_supported: bool,
    dependencies: Vec<String>,
}

/// What the audit knows about one part in this repository.
struct PartFacts {
    machine_present: bool,
    repo_present: bool,
    repo_file_here: bool,
    /// The levels whose status query ran but gave no answer.
    unknown: Vec<&'static str>,
}

fn dependency_names(root: &Path) -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(root.join("package.json")) else {
        return Vec::new();
    };
    let Ok(manifest) = serde_json::from_str::<Value>(&text) else {
        return Vec::new();
    };
    ["dependencies", "devDependencies", "peerDependencies"]
        .iter()
        .filter_map(|k| manifest.get(*k).and_then(Value::as_object))
        .flat_map(|deps| deps.keys().cloned())
        .collect()
}

fn existing(root: &Path, paths: &[String]) -> Vec<String> {
    paths
        .iter()
        .filter(|p| root.join(p).exists())
        .cloned()
        .collect()
}

/// The answer of a part's declared status query at one level, run on the
/// first candidate binary found; `None` when the level, its query or the
/// binary is missing, so nothing was asked.
fn level_answer(part: &Part, level: Option<&ToolLevel>, repo: Option<&Path>) -> Option<Answer> {
    let level = level.filter(|l| l.status.is_some())?;
    let bin = candidates(part.bin()?).into_iter().next()?;
    Some(owner::level_state(level, &bin, repo))
}

fn part_facts(part: &Part, root: &Path, ci: &[String]) -> PartFacts {
    let repo_file_here = part
        .repo_file
        .first()
        .and_then(|f| Path::new(&f.path).file_name())
        .is_some_and(|name| ci.iter().any(|dir| root.join(dir).join(name).is_file()));
    let machine = level_answer(part, part.machine(), None);
    let repo = level_answer(part, part.repo_level(), Some(root));
    let unknown = [("machine", machine), ("repo", repo)]
        .into_iter()
        .filter(|(_, a)| *a == Some(Answer::Unknown))
        .map(|(level, _)| level)
        .collect();
    PartFacts {
        machine_present: machine == Some(Answer::Present),
        repo_present: repo == Some(Answer::Present),
        repo_file_here,
        unknown,
    }
}

fn signal_holds(signal: &Signal, facts: &Facts, part: &PartFacts) -> bool {
    match signal {
        Signal::MachinePresent => part.machine_present,
        Signal::RepoPresent => part.repo_present,
        Signal::AdrFolder => facts.adr_folder.is_some(),
        Signal::ClaudeSetUp => !facts.claude_files.is_empty(),
        Signal::CiPresent => !facts.ci.is_empty(),
        Signal::LanguageSupported => facts.language_supported,
        Signal::RepoFileHere => part.repo_file_here,
        Signal::Dep(prefix) => facts.dependencies.iter().any(|d| d.starts_with(prefix)),
    }
}

fn hooks_dir(root: &Path) -> Option<PathBuf> {
    let git_hooks_path = git(root, &["rev-parse", "--git-path", "hooks"])?;
    Some(if Path::new(&git_hooks_path).is_absolute() {
        PathBuf::from(git_hooks_path)
    } else {
        root.join(git_hooks_path)
    })
}

fn foreign_hook_names(root: &Path) -> Vec<String> {
    let Some(dir) = hooks_dir(root) else {
        return Vec::new();
    };
    let mut names: Vec<String> = std::fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().is_file())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| !n.ends_with(".sample"))
        .collect();
    names.sort();
    names
}

/// Write `lines` to a locked stdout; a closed pipe ends the output quietly.
fn print_lines(lines: &[String]) -> Result<(), Exit> {
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    for line in lines {
        if let Err(e) = writeln!(out, "{line}") {
            return if e.kind() == std::io::ErrorKind::BrokenPipe {
                Ok(())
            } else {
                Err(Exit::Failed(format!("cannot write the audit: {e}")))
            };
        }
    }
    Ok(())
}

/// Read the repository at `dir` (default `.`) and assess every part; `None`
/// outside a git work tree. Prints nothing.
pub fn assess<'a>(cat: &'a Catalog, dir: Option<&Path>) -> Result<Option<Assessment<'a>>, Exit> {
    let given = dir.unwrap_or_else(|| Path::new("."));
    let Some(root) = git(given, &["rev-parse", "--show-toplevel"])
        .filter(|_| git(given, &["rev-parse", "--is-inside-work-tree"]).as_deref() == Some("true"))
        .map(PathBuf::from)
    else {
        return Ok(None);
    };

    // Languages: tracked and untracked-but-not-ignored files.
    let listing = git(
        &root,
        &["ls-files", "--cached", "--others", "--exclude-standard"],
    )
    .unwrap_or_default();
    let mut languages: BTreeMap<&str, usize> = BTreeMap::new();
    for f in listing.lines().filter(|l| !l.is_empty()) {
        *languages.entry(language(f)).or_default() += 1;
    }

    let ci: Vec<String> = cat
        .audit
        .ci_present
        .iter()
        .filter(|p| root.join(p).is_dir())
        .cloned()
        .collect();
    let facts = Facts {
        adr_folder: existing(&root, &cat.audit.adr_folder).into_iter().next(),
        claude_files: existing(&root, &cat.audit.claude_set_up),
        ci,
        language_supported: languages.keys().any(|l| *l != "other"),
        dependencies: dependency_names(&root),
    };
    let per_part: Vec<PartFacts> = cat
        .parts
        .iter()
        .map(|p| part_facts(p, &root, &facts.ci))
        .collect();

    let adr_text = facts.adr_folder.clone().unwrap_or_default();
    let ci_text = facts.ci.join(", ");
    let claude_text = facts.claude_files.join(", ");
    let values = |part: &Part, hooks_path: &str, manager: &str| -> Vec<(&'static str, String)> {
        vec![
            ("adr_folder", adr_text.clone()),
            ("ci", ci_text.clone()),
            ("claude_files", claude_text.clone()),
            ("part", part.name().to_string()),
            ("hooks_path", hooks_path.to_string()),
            ("manager", manager.to_string()),
        ]
    };
    let fill = |template: &str, vals: &[(&'static str, String)]| -> String {
        let pairs: Vec<(&str, &str)> = vals.iter().map(|(k, v)| (*k, v.as_str())).collect();
        render(template, &pairs)
    };

    // Hooks and warnings: the parts whose repo level is installed, then the
    // conflicts that hold, in catalog order.
    let mut hooks: Vec<String> = cat
        .parts
        .iter()
        .zip(&per_part)
        .filter(|(_, f)| f.repo_present)
        .map(|(p, _)| p.name().to_string())
        .collect();
    let mut warnings: Vec<String> = Vec::new();
    for (part, part_facts) in cat.parts.iter().zip(&per_part) {
        for conflict in &part.conflict {
            match &conflict.when {
                ConflictWhen::GitConfig(key) => {
                    let value = match git_config(&root, key) {
                        Ok(Some(v)) if !v.is_empty() => v,
                        Ok(_) => continue,
                        Err(why) => {
                            warnings.push(format!(
                                "{}: {why}; its conflict check did not run",
                                part.name()
                            ));
                            continue;
                        }
                    };
                    let vals = values(part, &value, "");
                    hooks.extend(conflict.label.iter().map(|l| fill(l, &vals)));
                    warnings.extend(conflict.warning.iter().map(|w| fill(w, &vals)));
                }
                ConflictWhen::ForeignHooks => {
                    if !part_facts.repo_present {
                        hooks.extend(
                            foreign_hook_names(&root)
                                .into_iter()
                                .map(|n| format!("hook {n}")),
                        );
                    }
                }
                ConflictWhen::Paths(paths) => {
                    if !paths.iter().any(|p| root.join(p).exists()) {
                        continue;
                    }
                    let manager = conflict
                        .label
                        .as_deref()
                        .map(|l| fill(l, &values(part, "", "")));
                    let vals = values(part, "", manager.as_deref().unwrap_or_default());
                    hooks.extend(manager);
                    warnings.extend(conflict.warning.iter().map(|w| fill(w, &vals)));
                }
            }
        }
    }

    // Status queries that ran and gave no answer: the row reads them as
    // absent, so say so.
    for (part, part_facts) in cat.parts.iter().zip(&per_part) {
        for level in &part_facts.unknown {
            warnings.push(format!(
                "{}: the {level} status query gave no answer; its row reads it as not installed",
                part.name()
            ));
        }
    }

    // Shadowed binaries.
    for part in &cat.parts {
        if let Some(bin) = part.bin() {
            let found = candidates(bin);
            if found.len() > 1 {
                let list: Vec<String> = found.iter().map(|p| p.display().to_string()).collect();
                warnings.push(format!(
                    "{bin} ({}) is shadowed on PATH: {} candidates, {} wins: {}",
                    part.name(),
                    found.len(),
                    found[0].display(),
                    list.join(", ")
                ));
            }
        }
    }

    // Rows: the first rule whose conditions all hold.
    let rows: Vec<Row> = cat
        .parts
        .iter()
        .zip(&per_part)
        .map(|(part, part_facts)| {
            let rule = part.recommend.iter().find(|r| {
                r.when
                    .iter()
                    .all(|c| signal_holds(&c.signal, &facts, part_facts) != c.negated)
            });
            match rule {
                Some(r) => Row {
                    part: part.name(),
                    verdict: r.verdict,
                    reason: fill(&r.reason, &values(part, "", "")),
                },
                None => Row {
                    part: part.name(),
                    verdict: Verdict::NotUseful,
                    reason: String::new(),
                },
            }
        })
        .collect();
    let is_experiment = |name: &str| cat.part(name).is_some_and(|p| p.is_experimental());
    let install: Vec<&str> = rows
        .iter()
        .filter(|r| r.verdict == Verdict::Recommend && !is_experiment(r.part))
        .map(|r| r.part)
        .collect();

    // Repository facts only: the per-part signals (machine_present,
    // repo_present, repo_file_here) hold for one part and are read in
    // that part's row, so a single flag here would mean "some part".
    let mut signals: BTreeMap<String, bool> = BTreeMap::new();
    let no_part = PartFacts {
        machine_present: false,
        repo_present: false,
        repo_file_here: false,
        unknown: Vec::new(),
    };
    for (name, signal) in Signal::PLAIN {
        let repository_level = match signal {
            Signal::MachinePresent | Signal::RepoPresent | Signal::RepoFileHere => false,
            Signal::AdrFolder
            | Signal::ClaudeSetUp
            | Signal::CiPresent
            | Signal::LanguageSupported
            | Signal::Dep(_) => true,
        };
        if repository_level {
            signals.insert(name.to_string(), signal_holds(&signal, &facts, &no_part));
        }
    }
    let prefixes = cat
        .parts
        .iter()
        .flat_map(|p| &p.recommend)
        .flat_map(|r| &r.when)
        .filter_map(|c| match &c.signal {
            Signal::Dep(prefix) => Some(prefix),
            Signal::MachinePresent
            | Signal::RepoPresent
            | Signal::AdrFolder
            | Signal::ClaudeSetUp
            | Signal::CiPresent
            | Signal::LanguageSupported
            | Signal::RepoFileHere => None,
        });
    for prefix in prefixes {
        let holds = facts.dependencies.iter().any(|d| d.starts_with(prefix));
        signals.insert(format!("dep:{prefix}"), holds);
    }

    Ok(Some(Assessment {
        root,
        languages,
        rows,
        hooks,
        warnings,
        adr_folder: facts.adr_folder,
        ci: facts.ci,
        signals,
        install,
        experimental: cat.experimental_names(),
    }))
}

pub fn run(cat: &Catalog, dir: Option<&Path>, format: crate::status::Format) -> Result<(), Exit> {
    let Some(assessment) = assess(cat, dir)? else {
        let given = dir.unwrap_or_else(|| Path::new("."));
        return Err(Exit::Usage(format!(
            "{} is not a git work tree",
            given.display()
        )));
    };
    let Assessment {
        root,
        languages,
        rows,
        hooks,
        warnings,
        adr_folder,
        ci,
        signals,
        install,
        experimental,
    } = assessment;
    let install = (!install.is_empty()).then(|| format!("grh install {}", install.join(" ")));

    if format == crate::status::Format::Json {
        let doc = json!({
            "schema": 2,
            "dir": root.to_string_lossy(),
            "languages": languages,
            "hooks": hooks,
            "adr_folder": adr_folder,
            "ci": ci,
            "signals": signals,
            "parts": rows.iter().map(|r| json!({
                "part": r.part, "verdict": r.verdict.as_str(), "reason": r.reason
            })).collect::<Vec<_>>(),
            "install": install,
            "experimental": experimental,
            "warnings": warnings,
        });
        let text = serde_json::to_string_pretty(&doc)
            .map_err(|e| Exit::Failed(format!("cannot encode the audit: {e}")))?;
        return print_lines(&[text]);
    }

    let recommended_experiments = rows.iter().filter(|r| {
        r.verdict == Verdict::Recommend && cat.part(r.part).is_some_and(|p| p.is_experimental())
    });
    let width = rows.iter().map(|r| r.part.len()).max().unwrap_or(0);
    let mut lines: Vec<String> = rows
        .iter()
        .map(|r| {
            format!("{:<width$} {:<13} {}", r.part, r.verdict.as_str(), r.reason)
                .trim_end()
                .to_string()
        })
        .collect();
    lines.push(install.unwrap_or_else(|| "nothing to install".to_string()));
    for Row { part: name, .. } in recommended_experiments {
        lines.push(format!(
            "{name} is an experiment: grh install {name} to try it"
        ));
    }
    let printed = print_lines(&lines);
    // A closed stderr is no reason to fail or panic: the rows are written.
    let stderr = std::io::stderr();
    let mut err = stderr.lock();
    for w in &warnings {
        if writeln!(err, "warning: {w}").is_err() {
            break;
        }
    }
    printed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalog() -> Catalog {
        Catalog::load().expect("the embedded catalog loads")
    }

    fn git_ok(dir: &Path, args: &[&str]) {
        let status = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .stdin(Stdio::null())
            .status()
            .expect("git runs");
        assert!(status.success(), "git {args:?} failed");
    }

    #[test]
    fn assess_reads_a_git_repository_without_printing() {
        let dir = tempfile::tempdir().expect("temp dir");
        git_ok(dir.path(), &["init", "-q"]);
        std::fs::write(dir.path().join("main.rs"), "fn main() {}\n").expect("write");
        let cat = catalog();
        let assessment = assess(&cat, Some(dir.path()))
            .expect("no failure")
            .expect("inside a work tree");
        assert_eq!(assessment.rows.len(), cat.parts.len());
        assert_eq!(assessment.languages.get("rust"), Some(&1));
        let recommended: Vec<&str> = assessment
            .rows
            .iter()
            .filter(|r| {
                r.verdict == Verdict::Recommend
                    && !cat.part(r.part).is_some_and(|p| p.is_experimental())
            })
            .map(|r| r.part)
            .collect();
        assert_eq!(assessment.install, recommended);
    }

    #[test]
    fn assess_outside_a_work_tree_is_none() {
        let dir = tempfile::tempdir().expect("temp dir");
        let cat = catalog();
        assert!(assess(&cat, Some(dir.path()))
            .expect("no failure")
            .is_none());
    }
}
