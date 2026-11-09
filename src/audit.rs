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
use crate::manifest::{render, Signal, ToolLevel, Verdict};
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

struct Row<'a> {
    part: &'a str,
    verdict: Verdict,
    reason: String,
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
/// first candidate binary found; unknown when the level, its query or the
/// binary is missing.
fn level_answer(part: &Part, level: Option<&ToolLevel>, repo: Option<&Path>) -> Answer {
    let (Some(level), Some(bin)) = (level, part.bin()) else {
        return Answer::Unknown;
    };
    candidates(bin)
        .first()
        .map_or(Answer::Unknown, |b| owner::level_state(level, b, repo))
}

fn part_facts(part: &Part, root: &Path, ci: &[String]) -> PartFacts {
    let repo_file_here = part
        .repo_file
        .first()
        .and_then(|f| Path::new(&f.path).file_name())
        .is_some_and(|name| ci.iter().any(|dir| root.join(dir).join(name).is_file()));
    PartFacts {
        machine_present: level_answer(part, part.machine(), None) == Answer::Present,
        repo_present: level_answer(part, part.repo_level(), Some(root)) == Answer::Present,
        repo_file_here,
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

pub fn run(cat: &Catalog, dir: Option<&Path>, format: crate::status::Format) -> Result<(), Exit> {
    let given = dir.unwrap_or_else(|| Path::new("."));
    let root = git(given, &["rev-parse", "--show-toplevel"])
        .filter(|_| git(given, &["rev-parse", "--is-inside-work-tree"]).as_deref() == Some("true"))
        .map(PathBuf::from)
        .ok_or_else(|| Exit::Usage(format!("{} is not a git work tree", given.display())))?;

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
            if let Some(key) = &conflict.git_config {
                let Some(value) = git(&root, &["config", "--get", key]).filter(|v| !v.is_empty())
                else {
                    continue;
                };
                let vals = values(part, &value, "");
                hooks.extend(conflict.label.iter().map(|l| fill(l, &vals)));
                warnings.extend(conflict.warning.iter().map(|w| fill(w, &vals)));
            } else if conflict.foreign_hooks {
                if !part_facts.repo_present {
                    hooks.extend(
                        foreign_hook_names(&root)
                            .into_iter()
                            .map(|n| format!("hook {n}")),
                    );
                }
            } else if conflict.paths.iter().any(|p| root.join(p).exists()) {
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
    let recommended: Vec<&str> = rows
        .iter()
        .filter(|r| r.verdict == Verdict::Recommend)
        .map(|r| r.part)
        .collect();
    let install =
        (!recommended.is_empty()).then(|| format!("grh install {}", recommended.join(" ")));

    if format == crate::status::Format::Json {
        let mut signals: BTreeMap<String, bool> = BTreeMap::new();
        let no_part = PartFacts {
            machine_present: false,
            repo_present: false,
            repo_file_here: false,
        };
        for (name, signal) in Signal::PLAIN {
            let holds = match signal {
                Signal::MachinePresent => per_part.iter().any(|f| f.machine_present),
                Signal::RepoPresent => per_part.iter().any(|f| f.repo_present),
                Signal::RepoFileHere => per_part.iter().any(|f| f.repo_file_here),
                Signal::AdrFolder
                | Signal::ClaudeSetUp
                | Signal::CiPresent
                | Signal::LanguageSupported
                | Signal::Dep(_) => signal_holds(&signal, &facts, &no_part),
            };
            signals.insert(name.to_string(), holds);
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
        let doc = json!({
            "schema": 2,
            "dir": root.to_string_lossy(),
            "languages": languages,
            "hooks": hooks,
            "adr_folder": facts.adr_folder,
            "ci": facts.ci,
            "signals": signals,
            "parts": rows.iter().map(|r| json!({
                "part": r.part, "verdict": r.verdict.as_str(), "reason": r.reason
            })).collect::<Vec<_>>(),
            "install": install,
            "warnings": warnings,
        });
        let text = serde_json::to_string_pretty(&doc)
            .map_err(|e| Exit::Failed(format!("cannot encode the audit: {e}")))?;
        return print_lines(&[text]);
    }

    let width = rows.iter().map(|r| r.part.len()).max().unwrap_or(0);
    let mut lines: Vec<String> = rows
        .iter()
        .map(|r| format!("{:<width$} {:<13} {}", r.part, r.verdict.as_str(), r.reason))
        .collect();
    lines.push(install.unwrap_or_else(|| "nothing to install".to_string()));
    let printed = print_lines(&lines);
    for w in &warnings {
        eprintln!("warning: {w}");
    }
    printed
}
