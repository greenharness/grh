//! End-to-end: the real `grh` binary against fake tools, a scratch HOME and
//! file:// release fixtures. Nothing here touches the real HOME or the network.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use grh::fetch::{sha256_hex, target};

/// A tool that logs its argv and keeps its integration state in files under
/// `$FAKE_STATE`: `<name>.machine` and `<name>.repo` exist when present.
/// A `claude` that logs its argv and keeps MCP servers as files under
/// `$FAKE_STATE`: `mcp.<name>` exists while the server is registered.
const FAKE_CLAUDE: &str = r#"#!/bin/sh
echo "claude $*" >> "$FAKE_LOG"
mkdir -p "$FAKE_STATE"
case "$1 $2" in
  "mcp add") [ -n "$FAKE_ADD_FAIL" ] && exit 1; touch "$FAKE_STATE/mcp.$5";;
  "mcp remove") [ -n "$FAKE_REMOVE_STICKY" ] || rm -f "$FAKE_STATE/mcp.$5";;
  "mcp get") [ -f "$FAKE_STATE/mcp.$3" ] && exit 0; exit 1;;
esac
exit 0
"#;

const FAKE: &str = r#"#!/bin/sh
name=$(basename "$0")
echo "$name $*" >> "$FAKE_LOG"
st="$FAKE_STATE/$name"
mkdir -p "$FAKE_STATE"
if [ "$FAKE_UNKNOWN" = "$name" ]; then
  case "$*" in doctor*|list*|*--check*) exit 7;; esac
fi
state() { if [ -f "$st.$1" ]; then echo present; elif [ -f "$st.partial" ]; then echo partial; else echo absent; fi; }
if [ "$FAKE_FAIL_UNINSTALL" = "$name" ]; then
  case "$*" in uninstall*) exit 3;; esac
fi
case "$name:$*" in
  *:--version) echo "$name 1.0.0";;
  aval:"mcp install --check") [ -f "$st.machine" ] && exit 0; exit 1;;
  aval:"hook uninstall --check") [ -f "$st.repo" ] && exit 1; exit 0;;
  aval:"mcp install --write") touch "$st.machine";;
  aval:"mcp uninstall --write") [ "$FAKE_STICKY" = "$name" ] || rm -f "$st.machine";;
  aval:"hook install") touch "$st.repo";;
  aval:"hook uninstall --write") rm -f "$st.repo";;
  amont:"list --json") echo "{\"installed\":{\"state\":\"$(state repo)\"},\"template_dir\":{\"state\":\"${FAKE_TEMPLATE:-absent}\",\"shims\":[{\"binary\":\"$FAKE_TEMPLATE_BIN\"}]}}";;
  amont:init) touch "$st.repo";;
  amont:"uninstall --repo") [ "$FAKE_STICKY" = "$name" ] || rm -f "$st.repo";;
  amont-agent:"doctor --json") echo "{\"integration\":{\"state\":\"$(state machine)\"}}";;
  amont-agent:"install --write") touch "$st.machine";;
  amont-agent:"uninstall --write") [ "$FAKE_STICKY" = "$name" ] || rm -f "$st.machine";;
esac
exit 0
"#;

struct Fx {
    _tmp: tempfile::TempDir,
    root: PathBuf,
}

fn tarball(files: &[(&str, &[u8])]) -> Vec<u8> {
    let gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    let mut b = tar::Builder::new(gz);
    for (name, data) in files {
        let mut h = tar::Header::new_gnu();
        h.set_size(data.len() as u64);
        h.set_mode(0o755);
        h.set_cksum();
        b.append_data(&mut h, name, *data).unwrap();
    }
    b.into_inner().unwrap().finish().unwrap()
}

fn put(path: &Path, bytes: &[u8]) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
}

fn executable(path: &Path, text: &str) {
    put(path, text.as_bytes());
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

impl Fx {
    fn new() -> Fx {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        let fx = Fx { _tmp: tmp, root };
        for d in ["home", "state", "claude", "bin", "fakes", "fstate", "work"] {
            fs::create_dir_all(fx.root.join(d)).unwrap();
        }
        executable(&fx.root.join("fakes/claude"), FAKE_CLAUDE);
        // Release fixtures and a catalog that pins them.
        let t = target().unwrap();
        let mut catalog = String::new();
        let skills = tarball(&[("skills/s1/SKILL.md", b"skill one\n")]);
        put(
            &fx.root.join(
                "rel/greenharness/skills/releases/download/v0.1.0/greenharness-skills-0.1.0.tar.gz",
            ),
            &skills,
        );
        catalog.push_str(&format!(
            "skills_repo = \"greenharness/skills\"\nskills_version = \"0.1.0\"\nskills_sha256 = \"{}\"\n\
             [profiles]\nsolo = [\"gate\", \"guardrails\"]\n",
            sha256_hex(&skills)
        ));
        for (part, bin, extra) in [
            (
                "decisions",
                "aval",
                "[part.machine]\ninstall = [\"{bin}\", \"mcp\", \"install\", \"--write\"]\n\
                 uninstall = [\"{bin}\", \"mcp\", \"uninstall\", \"--write\"]\nquery = \"aval-mcp\"\n\
                 [part.repo_level]\ninstall = [\"{bin}\", \"hook\", \"install\"]\n\
                 uninstall = [\"{bin}\", \"hook\", \"uninstall\", \"--write\"]\nquery = \"aval-hook\"\n",
            ),
            (
                "gate",
                "amont",
                "[part.repo_level]\ninstall = [\"{bin}\", \"init\"]\n\
                 uninstall = [\"{bin}\", \"uninstall\", \"--repo\"]\nquery = \"amont-list\"\n",
            ),
            (
                "guardrails",
                "amont-agent",
                "[part.machine]\ninstall = [\"{bin}\", \"install\", \"--write\"]\n\
                 uninstall = [\"{bin}\", \"uninstall\", \"--write\"]\nquery = \"amont-agent-doctor\"\n",
            ),
        ] {
            let tgz = tarball(&[(&format!("{bin}-1.0.0/{bin}"), FAKE.as_bytes())]);
            put(
                &fx.root.join(format!(
                    "rel/fredericrous/{bin}/releases/download/v1.0.0/{bin}-1.0.0-{t}.tar.gz"
                )),
                &tgz,
            );
            let skills_line = if part == "gate" { "[\"s1\"]" } else { "[]" };
            catalog.push_str(&format!(
                "[[part]]\nname = \"{part}\"\nsummary = \"x\"\nneeds = []\nbin = \"{bin}\"\n\
                 repo = \"fredericrous/{bin}\"\nversion = \"1.0.0\"\nskills = {skills_line}\n\
                 {extra}[part.sha256]\n{t} = \"{}\"\n",
                sha256_hex(&tgz)
            ));
        }
        catalog.push_str(
            "[[part]]\nname = \"attest\"\nsummary = \"x\"\nneeds = [\"gate\"]\nskills = []\nworkflow = \"attest\"\n\
             [[part]]\nname = \"design\"\nsummary = \"x\"\nneeds = []\nskills = []\nextension = \"unpublished\"\n",
        );
        put(&fx.root.join("catalog.toml"), catalog.as_bytes());
        fx
    }

    fn p(&self, rel: &str) -> PathBuf {
        self.root.join(rel)
    }

    fn grh(&self, cwd: &str, envs: &[(&str, &str)], args: &[&str]) -> Output {
        let r = |s: &str| self.p(s).to_string_lossy().into_owned();
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_grh"));
        cmd.args(args)
            .current_dir(self.p(cwd))
            .env_clear()
            .env("HOME", r("home"))
            .env("XDG_STATE_HOME", r("state"))
            .env("CLAUDE_CONFIG_DIR", r("claude"))
            .env("GRH_BIN_DIR", r("bin"))
            .env("GRH_RELEASE_BASE", format!("file://{}", r("rel")))
            .env("GRH_CATALOG", r("catalog.toml"))
            .env("FAKE_LOG", r("log.txt"))
            .env("FAKE_STATE", r("fstate"))
            .env("PATH", format!("{}:/usr/bin:/bin", r("fakes")))
            .env("NO_COLOR", "1")
            .stdin(Stdio::null());
        for (k, v) in envs {
            cmd.env(k, v);
        }
        cmd.output().unwrap()
    }

    fn log(&self) -> String {
        fs::read_to_string(self.p("log.txt")).unwrap_or_default()
    }

    fn receipt(&self) -> String {
        fs::read_to_string(self.p("state/grh/receipt.json")).unwrap_or_default()
    }

    /// The receipt's entries as sorted strings (re-recording a file moves it).
    fn entries(&self) -> Vec<String> {
        let v: serde_json::Value = serde_json::from_str(&self.receipt()).unwrap();
        let mut e: Vec<String> = v["entries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| x.to_string())
            .collect();
        e.sort();
        e
    }

    /// Every file under `dir`, relative.
    fn files(&self, dir: &str) -> Vec<String> {
        fn walk(d: &Path, root: &Path, out: &mut Vec<String>) {
            for e in fs::read_dir(d).into_iter().flatten().flatten() {
                let p = e.path();
                if p.is_dir() {
                    walk(&p, root, out);
                } else {
                    out.push(p.strip_prefix(root).unwrap().to_string_lossy().into_owned());
                }
            }
        }
        let root = self.p(dir);
        let mut out = Vec::new();
        walk(&root, &root, &mut out);
        out.sort();
        out
    }
}

fn git_init(fx: &Fx) {
    let out = Command::new("git")
        .args(["init", "-q"])
        .current_dir(fx.p("work"))
        .env_clear()
        .env("HOME", fx.p("home"))
        .env("PATH", "/usr/bin:/bin:/usr/local/bin:/opt/homebrew/bin")
        .output()
        .unwrap();
    assert!(out.status.success());
}

fn seed_receipt(fx: &Fx, entries: &serde_json::Value) {
    let doc = serde_json::json!({ "version": 1, "entries": entries });
    put(&fx.p("state/grh/receipt.json"), doc.to_string().as_bytes());
}

fn code(o: &Output) -> i32 {
    o.status.code().unwrap()
}

fn err(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

#[test]
fn install_solo_places_binaries_and_runs_the_tools_installs() {
    let fx = Fx::new();
    let o = fx.grh("work", &[], &["install", "solo", "--yes"]);
    assert_eq!(code(&o), 0, "{}", err(&o));
    assert_eq!(fx.files("bin"), ["amont", "amont-agent"]);
    assert!(fx.log().contains("amont-agent install --write"));
    assert!(fx.p("claude/skills/s1/SKILL.md").exists());
    assert!(fx.receipt().contains("binary-placed"));
    assert!(fx.receipt().contains("tool-install"));
    assert!(fx.p("fstate/amont-agent.machine").exists());
}

#[test]
fn a_second_install_changes_nothing_and_keeps_ownership() {
    let fx = Fx::new();
    assert_eq!(code(&fx.grh("work", &[], &["install", "solo", "--yes"])), 0);
    let entries = fx.entries();
    let installs = fx.log().matches("install --write").count();
    let o = fx.grh("work", &[], &["install", "solo", "--yes"]);
    assert_eq!(code(&o), 0, "{}", err(&o));
    assert_eq!(fx.entries(), entries);
    assert_eq!(fx.log().matches("install --write").count(), installs);
    assert!(!fx.receipt().contains("tool-install-adopted"));
    assert!(!fx.receipt().contains("binary-adopted"));
}

#[test]
fn a_present_integration_is_adopted_and_never_undone() {
    let fx = Fx::new();
    fs::write(fx.p("fstate/amont-agent.machine"), "").unwrap();
    assert_eq!(code(&fx.grh("work", &[], &["install", "solo", "--yes"])), 0);
    assert!(fx.receipt().contains("tool-install-adopted"));
    assert!(!fx.log().contains("amont-agent install --write"));
    let o = fx.grh("work", &[], &["uninstall", "--yes"]);
    assert_eq!(code(&o), 0, "{}", err(&o));
    assert!(!fx.log().contains("uninstall --write"));
    assert!(fx.p("fstate/amont-agent.machine").exists());
    assert!(fx.files("bin").is_empty());
}

#[test]
fn an_adopted_binary_on_path_is_left_alone() {
    let fx = Fx::new();
    executable(&fx.p("fakes/amont"), FAKE);
    assert_eq!(code(&fx.grh("work", &[], &["install", "gate", "--yes"])), 0);
    assert!(fx.receipt().contains("binary-adopted"));
    assert!(fx.files("bin").is_empty());
    assert_eq!(code(&fx.grh("work", &[], &["uninstall", "--yes"])), 0);
    assert!(fx.p("fakes/amont").exists());
}

#[test]
fn an_unknown_query_changes_nothing() {
    let fx = Fx::new();
    let o = fx.grh(
        "work",
        &[("FAKE_UNKNOWN", "amont-agent")],
        &["install", "guardrails", "--yes"],
    );
    assert_eq!(code(&o), 0, "{}", err(&o));
    assert!(!fx.log().contains("install --write"));
    assert!(!fx.receipt().contains("tool-install"));
    assert!(!fx.p("fstate/amont-agent.machine").exists());
    assert!(err(&o).contains("next:"), "{}", err(&o));
}

#[test]
fn a_missing_claude_is_a_reported_gap() {
    let fx = Fx::new();
    fs::remove_file(fx.p("fakes/claude")).unwrap();
    let o = fx.grh("work", &[], &["install", "guardrails", "--yes"]);
    assert_eq!(code(&o), 0, "{}", err(&o));
    assert!(!fx.log().contains("install --write"));
    assert!(err(&o).contains("claude"));
}

#[test]
fn bare_uninstall_removes_everything_it_placed() {
    let fx = Fx::new();
    assert_eq!(code(&fx.grh("work", &[], &["install", "solo", "--yes"])), 0);
    let o = fx.grh("work", &[], &["uninstall", "--yes"]);
    assert_eq!(code(&o), 0, "{}", err(&o));
    assert!(fx.files("bin").is_empty());
    assert!(fx.files("claude").is_empty());
    assert!(fx.files("home").is_empty());
    assert!(!fx.p("fstate/amont-agent.machine").exists());
    assert!(fx.log().contains("amont-agent uninstall --write"));
}

#[test]
fn a_binary_whose_integration_survives_the_undo_is_kept() {
    let fx = Fx::new();
    assert_eq!(code(&fx.grh("work", &[], &["install", "solo", "--yes"])), 0);
    let o = fx.grh(
        "work",
        &[("FAKE_STICKY", "amont-agent")],
        &["uninstall", "--yes"],
    );
    assert_eq!(code(&o), 1, "{}", err(&o));
    assert_eq!(fx.files("bin"), ["amont-agent"]);
    assert!(err(&o).contains("grh uninstall guardrails"), "{}", err(&o));
    assert!(fx.receipt().contains("tool-install"));
    // The retry works once the tool lets go.
    let o = fx.grh("work", &[], &["uninstall", "guardrails", "--yes"]);
    assert_eq!(code(&o), 0, "{}", err(&o));
    assert!(fx.files("bin").is_empty());
}

#[test]
fn without_yes_on_a_non_tty_nothing_changes() {
    let fx = Fx::new();
    let o = fx.grh("work", &[], &["install", "solo"]);
    assert_eq!(code(&o), 2, "{}", err(&o));
    assert!(fx.files("bin").is_empty());
    assert!(!fx.p("state/grh").exists());
    assert!(fx.log().is_empty());
}

#[test]
fn a_bare_install_lists_parts_and_exits_2() {
    let fx = Fx::new();
    let o = fx.grh("work", &[], &["install"]);
    assert_eq!(code(&o), 2);
    assert!(err(&o).contains("gate") && err(&o).contains("solo"));
}

#[test]
fn dry_run_changes_nothing() {
    let fx = Fx::new();
    let o = fx.grh("work", &[], &["install", "solo", "--dry-run"]);
    assert_eq!(code(&o), 0, "{}", err(&o));
    assert!(fx.files("bin").is_empty());
    assert!(!fx.p("state/grh").exists());
}

#[test]
fn installing_design_exits_3_after_installing_the_rest() {
    let fx = Fx::new();
    let o = fx.grh("work", &[], &["install", "solo", "design", "--yes"]);
    assert_eq!(code(&o), 3, "{}", err(&o));
    assert_eq!(fx.files("bin"), ["amont", "amont-agent"]);
}

#[test]
fn uninstall_warns_about_a_dependent_and_keep_deps_refuses() {
    let fx = Fx::new();
    git_init(&fx);
    let o = fx.grh("work", &[], &["install", "attest", "--repo", "--yes"]);
    assert_eq!(code(&o), 0, "{}", err(&o));
    assert!(fx.p("work/.github/workflows/attest.yml").exists());
    assert!(fx.log().contains("amont init"));
    let o = fx.grh("work", &[], &["uninstall", "gate", "--keep-deps", "--yes"]);
    assert_eq!(code(&o), 1);
    assert!(err(&o).contains("grh uninstall attest"), "{}", err(&o));
    let o = fx.grh("work", &[], &["uninstall", "gate", "--yes"]);
    assert_eq!(code(&o), 0);
    assert!(err(&o).contains("grh uninstall attest"), "{}", err(&o));
    // --repo narrows to this repository: the workflow goes, nothing else.
    let o = fx.grh("work", &[], &["uninstall", "--repo", "--yes"]);
    assert_eq!(code(&o), 0, "{}", err(&o));
    assert!(!fx.p("work/.github").exists());
}

#[test]
fn status_json_is_one_document_on_stdout() {
    let fx = Fx::new();
    assert_eq!(code(&fx.grh("work", &[], &["install", "solo", "--yes"])), 0);
    let o = fx.grh("work", &[], &["status", "--json"]);
    assert_eq!(code(&o), 0, "{}", err(&o));
    let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    let parts = v["parts"].as_array().unwrap();
    let gate = parts.iter().find(|p| p["name"] == "gate").unwrap();
    assert_eq!(gate["installed"], true);
    assert_eq!(gate["source"], "placed");
    assert_eq!(gate["version"], "amont 1.0.0");
    let guard = parts.iter().find(|p| p["name"] == "guardrails").unwrap();
    assert_eq!(guard["machine"]["state"], "present");
    assert_eq!(guard["machine"]["owner"], "grh");
    let design = parts.iter().find(|p| p["name"] == "design").unwrap();
    assert_eq!(design["installed"], false);
    let text = fx.grh("work", &[], &["status"]);
    assert!(String::from_utf8_lossy(&text.stdout).contains("gate"));
}

#[test]
fn a_pending_tool_install_row_is_promoted_not_duplicated() {
    let fx = Fx::new();
    executable(&fx.p("fakes/amont-agent"), FAKE);
    fs::write(fx.p("fstate/amont-agent.machine"), "").unwrap();
    seed_receipt(
        &fx,
        &serde_json::json!([{
            "part": "guardrails", "level": "machine", "state": "pending",
            "kind": { "tool-install": {
                "argv": ["x", "install", "--write"],
                "undo_argv": ["x", "uninstall", "--write"]
            } }
        }]),
    );
    let o = fx.grh("work", &[], &["install", "guardrails", "--yes"]);
    assert_eq!(code(&o), 0, "{}", err(&o));
    let v: serde_json::Value = serde_json::from_str(&fx.receipt()).unwrap();
    let rows: Vec<&serde_json::Value> = v["entries"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["kind"].get("tool-install").is_some() || e["kind"] == "tool-install-adopted")
        .collect();
    assert_eq!(rows.len(), 1, "{}", fx.receipt());
    assert_eq!(rows[0]["state"], "done");
    assert!(rows[0]["kind"].get("tool-install").is_some());
}

#[test]
fn a_pending_placed_binary_in_the_bin_dir_is_promoted_then_uninstalled() {
    let fx = Fx::new();
    let bin = fx.p("bin/amont");
    executable(&bin, FAKE);
    seed_receipt(
        &fx,
        &serde_json::json!([{
            "part": "gate", "level": "machine", "state": "pending",
            "kind": { "binary-placed": { "path": bin.to_string_lossy(), "sha256": "" } }
        }]),
    );
    let o = fx.grh("work", &[], &["install", "gate", "--yes"]);
    assert_eq!(code(&o), 0, "{}", err(&o));
    let v: serde_json::Value = serde_json::from_str(&fx.receipt()).unwrap();
    let placed: Vec<&serde_json::Value> = v["entries"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["kind"].get("binary-placed").is_some())
        .collect();
    assert_eq!(placed.len(), 1, "{}", fx.receipt());
    assert_eq!(placed[0]["state"], "done");
    assert_eq!(
        placed[0]["kind"]["binary-placed"]["sha256"],
        sha256_hex(FAKE.as_bytes())
    );
    assert!(!fx.receipt().contains("binary-adopted"));
    let o = fx.grh("work", &[], &["uninstall", "--yes"]);
    assert_eq!(code(&o), 0, "{}", err(&o));
    assert!(fx.files("bin").is_empty());
}

#[test]
fn status_reports_the_current_repos_owner_when_two_repos_are_armed() {
    let fx = Fx::new();
    git_init(&fx);
    let work = fx.p("work");
    seed_receipt(
        &fx,
        &serde_json::json!([
            {
                "part": "gate", "level": "repo", "state": "done",
                "repo": fx.p("elsewhere").to_string_lossy(),
                "kind": "tool-install-adopted"
            },
            {
                "part": "gate", "level": "repo", "state": "done",
                "repo": work.to_string_lossy(),
                "kind": { "tool-install": { "argv": ["x"], "undo_argv": ["y"] } }
            }
        ]),
    );
    let o = fx.grh("work", &[], &["status", "--json"]);
    assert_eq!(code(&o), 0, "{}", err(&o));
    let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    let gate = v["parts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == "gate")
        .unwrap();
    assert_eq!(gate["repo"]["owner"], "grh", "{gate}");
}

#[test]
fn a_bad_catalog_override_fails_with_exit_1_not_a_panic() {
    let fx = Fx::new();
    put(&fx.p("bad.toml"), b"this is not [a catalog");
    let bad = fx.p("bad.toml").to_string_lossy().into_owned();
    let output = fx.grh("work", &[("GRH_CATALOG", &bad)], &["status"]);
    assert_eq!(code(&output), 1, "{}", err(&output));
    assert!(err(&output).contains("GRH_CATALOG"), "{}", err(&output));
    assert!(!err(&output).contains("panicked"), "{}", err(&output));
}

const ATTEST_BYTES: &str = "\
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

fn extension_dir(fx: &Fx) -> String {
    put(
        &fx.p("ext/greenharness.toml"),
        b"[extension]\nname = \"x\"\nversion = \"1.0.0\"\n\n[[mcp]]\nname = \"srv\"\ncommand = \"srvbin\"\n",
    );
    fx.p("ext").to_string_lossy().into_owned()
}

#[test]
fn a_partial_query_with_no_grh_entry_records_nothing_and_runs_no_install() {
    let fx = Fx::new();
    fs::write(fx.p("fstate/amont-agent.partial"), "").unwrap();
    let o = fx.grh("work", &[], &["install", "guardrails", "--yes"]);
    assert_eq!(code(&o), 0, "{}", err(&o));
    assert!(!fx.log().contains("install --write"), "{}", fx.log());
    assert!(!fx.receipt().contains("tool-install"), "{}", fx.receipt());
    assert!(err(&o).contains("partly installed"), "{}", err(&o));
}

#[test]
fn an_existing_mcp_server_is_skipped_and_survives_uninstall() {
    let fx = Fx::new();
    fs::write(fx.p("fstate/mcp.srv"), "").unwrap();
    let dir = extension_dir(&fx);
    let o = fx.grh("work", &[], &["install", "--extension", &dir, "--yes"]);
    assert_eq!(code(&o), 0, "{}", err(&o));
    assert!(!fx.log().contains("mcp add"), "{}", fx.log());
    assert!(!fx.receipt().contains("tool-install"), "{}", fx.receipt());
    let o = fx.grh("work", &[], &["uninstall", "--yes"]);
    assert_eq!(code(&o), 0, "{}", err(&o));
    assert!(!fx.log().contains("mcp remove"), "{}", fx.log());
    assert!(fx.p("fstate/mcp.srv").exists());
}

#[test]
fn a_failed_mcp_add_leaves_no_entry() {
    let fx = Fx::new();
    let dir = extension_dir(&fx);
    let o = fx.grh(
        "work",
        &[("FAKE_ADD_FAIL", "1")],
        &["install", "--extension", &dir, "--yes"],
    );
    assert_eq!(code(&o), 1, "{}", err(&o));
    assert!(!fx.receipt().contains("tool-install"), "{}", fx.receipt());
    let o = fx.grh("work", &[], &["uninstall", "--yes"]);
    assert_eq!(code(&o), 0, "{}", err(&o));
    assert!(!fx.log().contains("mcp remove"), "{}", fx.log());
}

#[test]
fn an_mcp_server_grh_added_is_removed_only_when_claude_reports_it_gone() {
    let fx = Fx::new();
    let dir = extension_dir(&fx);
    let o = fx.grh("work", &[], &["install", "--extension", &dir, "--yes"]);
    assert_eq!(code(&o), 0, "{}", err(&o));
    assert!(fx.p("fstate/mcp.srv").exists());
    let o = fx.grh(
        "work",
        &[("FAKE_REMOVE_STICKY", "1")],
        &["uninstall", "--yes"],
    );
    assert_eq!(code(&o), 1, "{}", err(&o));
    assert!(fx.receipt().contains("tool-install"), "{}", fx.receipt());
    let o = fx.grh("work", &[], &["uninstall", "--yes"]);
    assert_eq!(code(&o), 0, "{}", err(&o));
    assert!(!fx.p("fstate/mcp.srv").exists());
    assert!(!fx.receipt().contains("tool-install"), "{}", fx.receipt());
}

#[test]
fn the_gate_binary_is_kept_while_the_template_dir_bakes_it_and_removed_when_absent() {
    let fx = Fx::new();
    assert_eq!(code(&fx.grh("work", &[], &["install", "gate", "--yes"])), 0);
    let bin = fx.p("bin/amont").to_string_lossy().into_owned();
    let o = fx.grh(
        "work",
        &[("FAKE_TEMPLATE", "present"), ("FAKE_TEMPLATE_BIN", &bin)],
        &["uninstall", "gate", "--yes"],
    );
    assert_eq!(code(&o), 0, "{}", err(&o));
    assert_eq!(fx.files("bin"), ["amont"]);
    assert!(err(&o).contains("kept"), "{}", err(&o));
    assert!(fx.receipt().contains("binary-placed"));
    // A failed query keeps it too.
    let o = fx.grh(
        "work",
        &[("FAKE_UNKNOWN", "amont")],
        &["uninstall", "gate", "--yes"],
    );
    assert_eq!(code(&o), 0, "{}", err(&o));
    assert_eq!(fx.files("bin"), ["amont"]);
    // Absent: removed, with the warning.
    let o = fx.grh("work", &[], &["uninstall", "gate", "--yes"]);
    assert_eq!(code(&o), 0, "{}", err(&o));
    assert!(fx.files("bin").is_empty());
    assert!(err(&o).contains("amont init"), "{}", err(&o));
}

#[test]
fn an_existing_attest_workflow_with_identical_bytes_is_not_ours_to_remove() {
    let fx = Fx::new();
    git_init(&fx);
    put(
        &fx.p("work/.github/workflows/attest.yml"),
        ATTEST_BYTES.as_bytes(),
    );
    let o = fx.grh("work", &[], &["install", "attest", "--repo", "--yes"]);
    assert_eq!(code(&o), 0, "{}", err(&o));
    assert!(!fx.receipt().contains("attest.yml"), "{}", fx.receipt());
    let o = fx.grh("work", &[], &["uninstall", "--yes"]);
    assert_eq!(code(&o), 0, "{}", err(&o));
    assert!(fx.p("work/.github/workflows/attest.yml").exists());
}

#[test]
fn a_failing_tool_uninstall_keeps_the_entry_and_exits_1() {
    let fx = Fx::new();
    assert_eq!(
        code(&fx.grh("work", &[], &["install", "guardrails", "--yes"])),
        0
    );
    let o = fx.grh(
        "work",
        &[("FAKE_FAIL_UNINSTALL", "amont-agent")],
        &["uninstall", "--yes"],
    );
    assert_eq!(code(&o), 1, "{}", err(&o));
    assert!(fx.receipt().contains("tool-install"), "{}", fx.receipt());
    assert_eq!(fx.files("bin"), ["amont-agent"]);
}

#[test]
fn an_unreachable_armed_repo_keeps_its_entry_and_the_binary_and_exits_1() {
    let fx = Fx::new();
    git_init(&fx);
    let o = fx.grh("work", &[], &["install", "gate", "--repo", "--yes"]);
    assert_eq!(code(&o), 0, "{}", err(&o));
    fs::remove_dir_all(fx.p("work")).unwrap();
    let o = fx.grh("home", &[], &["uninstall", "--yes"]);
    assert_eq!(code(&o), 1, "{}", err(&o));
    assert!(fx.receipt().contains("tool-install"), "{}", fx.receipt());
    assert_eq!(fx.files("bin"), ["amont"]);
}

fn audit_install(fx: &Fx) -> (String, Vec<String>) {
    git_init(fx);
    let o = fx.grh("work", &[], &["audit", "--json"]);
    assert_eq!(code(&o), 0, "{}", err(&o));
    let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    let warnings = v["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| w.as_str().unwrap().to_string())
        .collect();
    (v["install"].as_str().unwrap().to_string(), warnings)
}

#[test]
fn audit_of_a_rust_repo_with_husky_recommends_solo_and_warns() {
    let fx = Fx::new();
    put(&fx.p("work/src/main.rs"), b"fn main() {}\n");
    put(&fx.p("work/.husky/pre-commit"), b"#!/bin/sh\n");
    let (install, warnings) = audit_install(&fx);
    assert_eq!(install, "grh install solo");
    assert!(warnings.iter().any(|w| w.contains("husky")), "{warnings:?}");
}

#[test]
fn audit_of_a_typescript_repo_with_adrs_recommends_team() {
    let fx = Fx::new();
    put(&fx.p("work/src/a.ts"), b"export {};\n");
    put(&fx.p("work/docs/adr/0001-x.md"), b"# 1\n");
    let (install, _) = audit_install(&fx);
    assert_eq!(install, "grh install team");
}

#[test]
fn audit_of_an_empty_repo_recommends_solo() {
    let fx = Fx::new();
    let (install, warnings) = audit_install(&fx);
    assert_eq!(install, "grh install solo");
    assert!(warnings.is_empty(), "{warnings:?}");
}
