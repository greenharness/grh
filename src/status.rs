//! `grh status [--json]`: each part, installed or not, its version, whether
//! grh placed or adopted its binary, and what its owner says. Reads only.

use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::{json, Value};

use crate::catalog::{Catalog, Integration, Part};
use crate::dispatch;
use crate::exit::Exit;
use crate::ops::{bin_for, git_root, mark, recorded_bin, same_scope, use_color, Mark};
use crate::owner::{self, Answer};
use crate::receipt::{self, Kind, Level};

/// First line of `<bin> --version`.
fn version_of(bin: &Path) -> Option<String> {
    let out = Command::new(bin)
        .arg("--version")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .next()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
}

fn state_of(
    integ: Option<&Integration>,
    bin: Option<&PathBuf>,
    repo: Option<&Path>,
) -> Option<Answer> {
    let integ = integ?;
    Some(match bin {
        Some(b) => owner::query(&integ.query, b, repo),
        None => Answer::Unknown,
    })
}

fn part_json(part: &Part, receipt: &receipt::Receipt, repo: Option<&Path>) -> Value {
    let recorded = recorded_bin(receipt, &part.name);
    let bin = bin_for(receipt, part);
    let source = match (&recorded, &bin) {
        (Some((_, true)), Some(_)) => Some("placed"),
        (Some((_, false)), Some(_)) => Some("adopted"),
        (_, Some(_)) => Some("on-path"),
        _ => None,
    };
    let held = receipt.entries.iter().any(|e| e.part == part.name);
    let installed = if part.bin.is_some() {
        bin.is_some()
    } else {
        held
    };
    let repos: Vec<&str> = receipt
        .entries
        .iter()
        .filter(|e| e.part == part.name && e.level == Level::Repo)
        .filter_map(|e| e.repo.as_deref())
        .collect();
    let owner_of = |level: Level, scope: Option<&Path>| {
        receipt
            .entries
            .iter()
            .filter(|e| same_scope(e, &part.name, level, scope))
            .find_map(|e| match e.kind {
                Kind::ToolInstall { .. } => Some("grh"),
                Kind::ToolInstallAdopted => Some("adopted"),
                _ => None,
            })
    };
    let machine = state_of(part.machine.as_ref(), bin.as_ref(), None)
        .map(|s| json!({ "state": s.as_str(), "owner": owner_of(Level::Machine, None) }));
    let in_repo = repo.and_then(|r| {
        state_of(part.repo_level.as_ref(), bin.as_ref(), Some(r)).map(|s| {
            json!({ "path": r.to_string_lossy(), "state": s.as_str(), "owner": owner_of(Level::Repo, Some(r)) })
        })
    });
    json!({
        "name": part.name,
        "summary": part.summary,
        "installed": installed,
        "version": bin.as_deref().and_then(version_of),
        "binary": bin.as_ref().map(|b| b.to_string_lossy().into_owned()),
        "source": source,
        "machine": machine,
        "repo": in_repo,
        "armed_repos": repos,
        "extension": part.extension,
    })
}

/// How `status` prints.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Text,
    Json,
}

pub fn run(cat: &Catalog, format: Format) -> Result<(), Exit> {
    let receipt = receipt::read_in(&receipt::state_dir()?)?;
    let repo = git_root().ok();
    let parts: Vec<Value> = cat
        .parts
        .iter()
        .map(|p| part_json(p, &receipt, repo.as_deref()))
        .collect();
    if format == Format::Json {
        let doc = json!({ "parts": parts, "bin_dir": dispatch::bin_dir() });
        println!(
            "{}",
            serde_json::to_string_pretty(&doc)
                .map_err(|e| Exit::Failed(format!("cannot encode the status: {e}")))?
        );
        return Ok(());
    }
    let color = use_color(std::io::stdout().is_terminal());
    for p in &parts {
        let installed = p["installed"].as_bool().unwrap_or(false);
        let (m, word) = if installed {
            (mark(Mark::Ok, color), "installed")
        } else {
            (mark(Mark::Fail, color), "not installed")
        };
        let mut extra = Vec::new();
        if let Some(v) = p["version"].as_str() {
            extra.push(v.to_string());
        }
        if let Some(s) = p["source"].as_str() {
            extra.push(s.to_string());
        }
        for level in ["machine", "repo"] {
            if let Some(s) = p[level]["state"].as_str() {
                let flag = if s == "present" { Mark::Ok } else { Mark::Warn };
                extra.push(format!("{level} {} {s}", mark(flag, color)));
            }
        }
        let tail = if extra.is_empty() {
            String::new()
        } else {
            format!("  ({})", extra.join(", "))
        };
        println!("{m} {:<11} {word}{tail}", p["name"].as_str().unwrap_or(""));
    }
    Ok(())
}
