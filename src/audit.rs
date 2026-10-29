//! `grh audit [dir] [--json]`: read a repository and recommend parts and a
//! profile. Reads only: it runs tools' own read-only queries and never reads
//! a tool's files to learn what it installed.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::{json, Value};

use crate::catalog::Catalog;
use crate::exit::Exit;
use crate::owner::{self, Answer};

const RECOMMEND: &str = "recommend";
const HERE: &str = "already here";
const NOT_USEFUL: &str = "not useful";

/// The profiles, smallest first; each is a superset of the one before.
const PROFILES: [(&str, &[&str]); 3] = [
    ("solo", &["gate", "guardrails"]),
    ("team", &["gate", "guardrails", "decisions", "conform"]),
    (
        "full",
        &["gate", "guardrails", "decisions", "conform", "router"],
    ),
];
const CORE: [&str; 5] = ["gate", "guardrails", "decisions", "conform", "router"];

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

struct Row {
    part: &'static str,
    verdict: &'static str,
    reason: String,
}

fn row(part: &'static str, verdict: &'static str, reason: impl Into<String>) -> Row {
    Row {
        part,
        verdict,
        reason: reason.into(),
    }
}

fn has_duro(root: &Path) -> bool {
    let Ok(text) = std::fs::read_to_string(root.join("package.json")) else {
        return false;
    };
    let Ok(v) = serde_json::from_str::<Value>(&text) else {
        return false;
    };
    ["dependencies", "devDependencies", "peerDependencies"]
        .iter()
        .filter_map(|k| v.get(*k).and_then(Value::as_object))
        .any(|deps| deps.keys().any(|n| n.starts_with("@duro-app/")))
}

// holds-until: v0.1.1 folds `json_out` into status::Format, as status does.
pub fn run(cat: &Catalog, dir: Option<&Path>, json_out: bool) -> Result<(), Exit> {
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
    let supported = languages.keys().any(|l| *l != "other");

    let mut warnings: Vec<String> = Vec::new();

    // Hooks.
    let mut hooks: Vec<String> = Vec::new();
    let amont_state = candidates("amont")
        .first()
        .map_or(Answer::Unknown, |b| owner::amont_list(b, Some(&root)));
    if amont_state == Answer::Present {
        hooks.push("amont".into());
    }
    let hooks_path = git(&root, &["config", "--get", "core.hooksPath"]).filter(|p| !p.is_empty());
    if let Some(p) = &hooks_path {
        hooks.push(format!("core.hooksPath={p}"));
        warnings.push(format!(
            "core.hooksPath redirects git hooks to {p}; amont refuses a redirected hooks path: unset it before grh install gate"
        ));
    }
    let mut managers: Vec<&str> = Vec::new();
    if root.join(".husky").is_dir() {
        managers.push("husky (.husky/)");
    }
    for f in ["lefthook.yml", ".lefthook.yml"] {
        if root.join(f).is_file() {
            managers.push("lefthook");
            break;
        }
    }
    if root.join(".pre-commit-config.yaml").is_file() {
        managers.push("pre-commit (.pre-commit-config.yaml)");
    }
    for m in &managers {
        hooks.push((*m).to_string());
        warnings.push(format!(
            "{m} already manages git hooks here; the gate part (amont) installs its own hooks next to it: check they do not clash"
        ));
    }
    if amont_state != Answer::Present {
        if let Some(h) = git(&root, &["rev-parse", "--git-path", "hooks"]) {
            let hooks_dir = if Path::new(&h).is_absolute() {
                PathBuf::from(h)
            } else {
                root.join(h)
            };
            let mut other: Vec<String> = std::fs::read_dir(&hooks_dir)
                .into_iter()
                .flatten()
                .flatten()
                .filter(|e| e.path().is_file())
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .filter(|n| !n.ends_with(".sample"))
                .collect();
            other.sort();
            for n in other {
                hooks.push(format!("hook {n}"));
            }
        }
    }

    // ADR folder.
    let adr = [".adr.yaml", "docs/adr", "docs/decisions", "doc/adr"]
        .iter()
        .find(|p| root.join(p).exists())
        .map(|p| (*p).to_string());

    // Claude Code setup.
    let claude_files: Vec<&str> = [".claude", "CLAUDE.md", "AGENTS.md"]
        .into_iter()
        .filter(|p| root.join(p).exists())
        .collect();
    let claude_set_up = !claude_files.is_empty();
    let state_of = |bin: &str, q: fn(&Path) -> Answer| {
        candidates(bin).first().map_or(Answer::Unknown, |b| q(b))
    };
    let guardrails_state = state_of("amont-agent", owner::amont_agent_doctor);
    let router_state = state_of("relais", owner::relais_doctor);
    let aval_mcp_registered = candidates("claude").first().is_some_and(|c| {
        Command::new(c)
            .args(["mcp", "get", "aval"])
            .current_dir(&root)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    });

    // CI.
    let ci: Vec<&str> = [".github/workflows", ".forgejo/workflows"]
        .into_iter()
        .filter(|p| root.join(p).is_dir())
        .collect();
    let attest_here = ci.iter().any(|d| root.join(d).join("attest.yml").is_file());

    let duro = has_duro(&root);

    // Shadowed binaries.
    for part in &cat.parts {
        if let Some(bin) = &part.bin {
            let found = candidates(bin);
            if found.len() > 1 {
                let list: Vec<String> = found.iter().map(|p| p.display().to_string()).collect();
                warnings.push(format!(
                    "{bin} ({}) is shadowed on PATH: {} candidates, {} wins: {}",
                    part.name,
                    found.len(),
                    found[0].display(),
                    list.join(", ")
                ));
            }
        }
    }

    // Rows.
    let mut rows: Vec<Row> = Vec::new();
    rows.push(match amont_state {
        Answer::Present => row("gate", HERE, "amont's git hooks are installed here"),
        _ => row(
            "gate",
            RECOMMEND,
            "a git repository: gate its commits and pushes",
        ),
    });
    rows.push(match guardrails_state {
        Answer::Present => row("guardrails", HERE, "amont-agent's integration is installed"),
        _ => row(
            "guardrails",
            RECOMMEND,
            "a git repository: keep agents inside the rules",
        ),
    });
    rows.push(match &adr {
        Some(_) if aval_mcp_registered => {
            row("decisions", HERE, "aval is registered with Claude Code")
        }
        Some(a) => row("decisions", RECOMMEND, format!("decisions live in {a}")),
        None => row("decisions", NOT_USEFUL, "no ADR folder"),
    });
    rows.push(match (&adr, supported) {
        (Some(a), true) => row(
            "conform",
            RECOMMEND,
            format!("{a} plus a supported language"),
        ),
        (Some(_), false) => row(
            "conform",
            NOT_USEFUL,
            "no supported language (rust, typescript/javascript, python, go)",
        ),
        (None, _) => row("conform", NOT_USEFUL, "no ADR folder to conform to"),
    });
    rows.push(match (claude_set_up, router_state) {
        (true, Answer::Present) => row("router", HERE, "relais's integration is installed"),
        (true, _) => row(
            "router",
            RECOMMEND,
            format!("Claude Code is set up ({})", claude_files.join(", ")),
        ),
        (false, _) => row("router", NOT_USEFUL, "Claude Code is not set up here"),
    });
    rows.push(if attest_here {
        row("attest", HERE, "an attest workflow exists")
    } else if !ci.is_empty() {
        row("attest", RECOMMEND, format!("CI in {}", ci.join(", ")))
    } else {
        row("attest", NOT_USEFUL, "no CI workflows")
    });
    rows.push(if claude_set_up {
        row(
            "workflow",
            RECOMMEND,
            "Claude Code is set up: release and merge skills",
        )
    } else {
        row("workflow", NOT_USEFUL, "Claude Code is not set up here")
    });
    rows.push(if duro {
        row("design", RECOMMEND, "package.json depends on @duro-app/")
    } else {
        row("design", NOT_USEFUL, "no @duro-app/ dependency")
    });

    // Profile: the smallest covering the recommended core parts.
    let rec = |p: &str| rows.iter().any(|r| r.part == p && r.verdict == RECOMMEND);
    let core_rec: Vec<&str> = CORE.iter().copied().filter(|p| rec(p)).collect();
    let (profile, members) = PROFILES
        .iter()
        .find(|(_, m)| core_rec.iter().all(|p| m.contains(p)))
        .copied()
        .unwrap_or(PROFILES[2]);
    let extras: Vec<&str> = ["attest", "workflow", "design"]
        .into_iter()
        .filter(|p| rec(p))
        .collect();
    let mut install = format!("grh install {profile}");
    for e in &extras {
        install.push(' ');
        install.push_str(e);
    }

    if json_out {
        let doc = json!({
            "dir": root.to_string_lossy(),
            "languages": languages,
            "hooks": hooks,
            "adr_folder": adr,
            "claude": {
                "files": claude_files,
                "amont_agent": guardrails_state.as_str(),
                "relais": router_state.as_str(),
                "aval_mcp": aval_mcp_registered,
            },
            "ci": ci,
            "duro": duro,
            "parts": rows.iter().map(|r| json!({
                "part": r.part, "verdict": r.verdict, "reason": r.reason
            })).collect::<Vec<_>>(),
            "profile": profile,
            "profile_parts": members,
            "extras": extras,
            "install": install,
            "warnings": warnings,
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&doc)
                .map_err(|e| Exit::Failed(format!("cannot encode the audit: {e}")))?
        );
        return Ok(());
    }

    for r in &rows {
        println!("{:<11} {:<13} {}", r.part, r.verdict, r.reason);
    }
    println!("{install}");
    for w in &warnings {
        eprintln!("warning: {w}");
    }
    Ok(())
}
