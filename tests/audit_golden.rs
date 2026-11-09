//! `grh audit --json` against golden outputs recorded from grh 0.2.0.
//!
//! Each fixture is a small repository plus the answers its stub tools give.
//! The goldens in `tests/fixtures/audit/` were recorded by the released 0.2.0
//! binary under a scratch HOME and a PATH of stub tools
//! (`tests/fixtures/audit/bin`), so they do not depend on what is installed
//! on the machine that runs this test, CI included.
//!
//! To record again from a binary:
//! `GRH_AUDIT_RECORD=/path/to/grh cargo test --test audit_golden`.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

struct Fixture {
    name: &'static str,
    files: &'static [(&'static str, &'static str)],
    hooks_path: Option<&'static str>,
    env: &'static [(&'static str, &'static str)],
}

const FIXTURES: &[Fixture] = &[
    Fixture {
        name: "empty",
        files: &[],
        hooks_path: None,
        env: &[],
    },
    Fixture {
        name: "rust_husky",
        files: &[
            ("src/main.rs", "fn main() {}\n"),
            (".husky/pre-commit", "#!/bin/sh\n"),
            (".git/hooks/pre-commit", "#!/bin/sh\n"),
        ],
        hooks_path: None,
        env: &[],
    },
    Fixture {
        name: "managers_all",
        files: &[
            ("lefthook.yml", "pre-commit: {}\n"),
            (".pre-commit-config.yaml", "repos: []\n"),
            ("app.py", "print(1)\n"),
        ],
        hooks_path: None,
        env: &[],
    },
    Fixture {
        name: "hooks_path",
        files: &[
            (".githooks/pre-push", "#!/bin/sh\n"),
            ("main.go", "package main\n"),
        ],
        hooks_path: Some(".githooks"),
        env: &[],
    },
    Fixture {
        name: "ts_adrs",
        files: &[
            ("src/index.ts", "export {}\n"),
            ("docs/adr/0001-x.md", "# x\n"),
        ],
        hooks_path: None,
        env: &[],
    },
    Fixture {
        name: "adr_no_language",
        files: &[(".adr.yaml", "x: 1\n")],
        hooks_path: None,
        env: &[],
    },
    Fixture {
        name: "gate_installed",
        files: &[("src/main.rs", "fn main() {}\n")],
        hooks_path: None,
        env: &[("STUB_AMONT", "present")],
    },
    Fixture {
        name: "claude_setup",
        files: &[
            ("CLAUDE.md", "# c\n"),
            (".claude/settings.json", "{}\n"),
            ("lib.py", "x = 1\n"),
        ],
        hooks_path: None,
        env: &[],
    },
    Fixture {
        name: "claude_installed",
        files: &[("AGENTS.md", "# a\n")],
        hooks_path: None,
        env: &[("STUB_RELAIS", "present"), ("STUB_AGENT", "present")],
    },
    Fixture {
        name: "ci_present",
        files: &[(".github/workflows/ci.yml", "name: ci\n")],
        hooks_path: None,
        env: &[],
    },
    Fixture {
        name: "ci_attest",
        files: &[
            (".forgejo/workflows/attest.yml", "name: attest\n"),
            (".github/workflows/ci.yml", "name: ci\n"),
        ],
        hooks_path: None,
        env: &[],
    },
    Fixture {
        name: "duro",
        files: &[
            (
                "package.json",
                "{\"dependencies\":{\"@duro-app/ui\":\"^6\"}}\n",
            ),
            ("src/app.tsx", "export {}\n"),
        ],
        hooks_path: None,
        env: &[],
    },
    Fixture {
        name: "aval_mcp_registered",
        files: &[
            ("docs/decisions/0001.md", "# d\n"),
            ("src/main.rs", "fn main() {}\n"),
        ],
        hooks_path: None,
        env: &[("STUB_AVAL_MCP", "1")],
    },
];

/// The part names grh 0.2.0 printed in its audit rows, and what the catalog
/// calls those parts now.
const NAMES: [(&str, &str); 2] = [("gate", "greengate"), ("guardrails", "greenpiste")];

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/audit")
}

fn git(repo: &Path, args: &[&str]) {
    let ok = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .status()
        .expect("git runs")
        .success();
    assert!(ok, "git {args:?} failed");
}

/// Run `grh audit --json` with `bin` on the fixture, under a scratch HOME and a
/// PATH of the stub tools, and return the document with `dir` replaced.
fn audit(bin: &Path, fx: &Fixture) -> Value {
    let scratch = tempfile::tempdir().expect("tempdir");
    let repo = scratch.path().join("repo");
    let home = scratch.path().join("home");
    fs::create_dir_all(&repo).unwrap();
    fs::create_dir_all(&home).unwrap();
    git(&repo, &["init", "-q", "."]);
    if let Some(p) = fx.hooks_path {
        git(&repo, &["config", "core.hooksPath", p]);
    }
    for (rel, text) in fx.files {
        let path = repo.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, text).unwrap();
    }
    let stubs = fixtures_dir().join("bin");
    let out = Command::new(bin)
        .args(["audit", "--json"])
        .current_dir(&repo)
        .env_clear()
        .env("HOME", &home)
        .env("XDG_STATE_HOME", home.join("state"))
        .env("PATH", format!("{}:/usr/bin:/bin", stubs.display()))
        .envs(fx.env.iter().copied())
        .output()
        .expect("grh runs");
    assert!(
        out.status.success(),
        "{}: grh audit failed: {}",
        fx.name,
        String::from_utf8_lossy(&out.stderr)
    );
    let mut doc: Value = serde_json::from_slice(&out.stdout)
        .unwrap_or_else(|e| panic!("{}: audit printed no JSON: {e}", fx.name));
    doc["dir"] = Value::String("<dir>".into());
    doc
}

/// Rename the 0.2.0 part names to the catalog's current ones.
fn rename_parts(doc: &mut Value) {
    if let Some(rows) = doc["parts"].as_array_mut() {
        for row in rows {
            let part = row["part"].as_str().unwrap_or_default().to_string();
            if let Some((_, new)) = NAMES.iter().find(|(old, _)| *old == part) {
                row["part"] = Value::String((*new).into());
            }
        }
    }
}

fn golden_path(fx: &Fixture) -> PathBuf {
    fixtures_dir().join(format!("{}.json", fx.name))
}

#[test]
fn audit_reproduces_the_goldens_recorded_from_0_2_0() {
    if let Some(recorder) = std::env::var_os("GRH_AUDIT_RECORD") {
        for fx in FIXTURES {
            let doc = audit(Path::new(&recorder), fx);
            let mut text = serde_json::to_string_pretty(&doc).unwrap();
            text.push('\n');
            fs::write(golden_path(fx), text).unwrap();
        }
        return;
    }
    let bin = Path::new(env!("CARGO_BIN_EXE_grh"));
    for fx in FIXTURES {
        let golden: Value = serde_json::from_str(
            &fs::read_to_string(golden_path(fx))
                .unwrap_or_else(|e| panic!("{}: no golden: {e}", fx.name)),
        )
        .unwrap();
        let mut want = golden;
        let mut got = audit(bin, fx);
        rename_parts(&mut want);
        rename_parts(&mut got);
        if got["schema"] != 2 {
            assert_eq!(got, want, "fixture {}", fx.name);
            continue;
        }
        for key in ["parts", "warnings", "ci", "adr_folder", "languages", "dir"] {
            assert_eq!(got[key], want[key], "fixture {}: {key}", fx.name);
        }
        let new = &expected_new()[fx.name];
        assert_eq!(
            got["install"], new["install"],
            "fixture {}: install",
            fx.name
        );
        assert_eq!(got["hooks"], new["hooks"], "fixture {}: hooks", fx.name);
    }
}

/// What the new audit must say where it differs from the 0.2.0 goldens: the
/// `install` line and the `hooks` list under the catalog's part names.
fn expected_new() -> Value {
    let path = fixtures_dir().join("expected-new.json");
    serde_json::from_str(&fs::read_to_string(path).expect("expected-new.json")).unwrap()
}

#[test]
fn every_fixture_has_a_golden() {
    let expected = expected_new();
    for fx in FIXTURES {
        assert!(golden_path(fx).is_file(), "no golden for {}", fx.name);
        assert!(
            expected.get(fx.name).is_some(),
            "no expected for {}",
            fx.name
        );
    }
}

#[test]
fn a_status_query_without_an_answer_warns_that_its_row_reads_it_absent() {
    let fx = Fixture {
        name: "status_unknown",
        files: &[("src/main.rs", "fn main() {}\n")],
        hooks_path: None,
        env: &[("STUB_AMONT", "garbled")],
    };
    let doc = audit(Path::new(env!("CARGO_BIN_EXE_grh")), &fx);
    let warnings: Vec<&str> = doc["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert!(
        warnings
            .iter()
            .any(|w| w.starts_with("greengate: the repo status query gave no answer")),
        "{warnings:?}"
    );
    let row = doc["parts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["part"] == "greengate")
        .unwrap();
    assert_eq!(row["verdict"], "recommend");
}
