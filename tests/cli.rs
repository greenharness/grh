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
  amont:"list --json") tpl="{\"state\":\"${FAKE_TEMPLATE:-absent}\",\"shims\":[{\"binary\":\"$FAKE_TEMPLATE_BIN\"}]}"; [ -n "$FAKE_TEMPLATE_RAW" ] && tpl=$FAKE_TEMPLATE_RAW; echo "{\"installed\":{\"state\":\"$(state repo)\"},\"template_dir\":$tpl}";;
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
            "skills_repo = \"greenharness/skills\"\nskills_version = \"0.1.0\"\nskills_sha256 = \"{}\"\nforeign_markers = [\"relais:begin\"]\n\
             order = [\"decisions\", \"gate\", \"guardrails\", \"attest\", \"design\", \"lab\"]\n\
             [retired_names]\nsolo = [\"gate\", \"guardrails\"]\n",
            sha256_hex(&skills)
        ));
        let level = |key: &str, install: &str, uninstall: &str, requires: &str, status: &str| {
            format!(
                "[part.tool.{key}]\ninstall = {install}\nuninstall = {uninstall}\n{requires}\
                 [part.tool.{key}.status]\n{status}\n"
            )
        };
        let claude = "requires = [\"claude\"]\n";
        for (part, bin, extra) in [
            (
                "decisions",
                "aval",
                level(
                    "machine",
                    "[\"{bin}\", \"mcp\", \"install\", \"--write\"]",
                    "[\"{bin}\", \"mcp\", \"uninstall\", \"--write\"]",
                    claude,
                    "args = [\"mcp\", \"install\", \"--check\"]\nexit = { present = 0, absent = 1 }",
                ) + &level(
                    "repo_level",
                    "[\"{bin}\", \"hook\", \"install\"]",
                    "[\"{bin}\", \"hook\", \"uninstall\", \"--write\"]",
                    "",
                    "args = [\"hook\", \"uninstall\", \"--check\"]\nexit = { present = 1, absent = 0 }",
                ),
            ),
            (
                "gate",
                "amont",
                level(
                    "repo_level",
                    "[\"{bin}\", \"init\"]",
                    "[\"{bin}\", \"uninstall\", \"--repo\"]",
                    "",
                    "args = [\"list\", \"--json\"]\njson = [\"installed\", \"state\"]",
                ) + "[part.tool.keep_binary]\nargs = [\"list\", \"--json\"]\n\
                     state = [\"template_dir\", \"state\"]\nabsent = \"absent\"\n\
                     shims = [\"template_dir\", \"shims\"]\nbinary = \"binary\"\n\
                     warning = \"repositories armed by hand with it fail their next commit until `amont init` re-arms them\"\n",
            ),
            (
                "guardrails",
                "amont-agent",
                level(
                    "machine",
                    "[\"{bin}\", \"install\", \"--write\"]",
                    "[\"{bin}\", \"uninstall\", \"--write\"]",
                    claude,
                    "args = [\"doctor\", \"--json\"]\njson = [\"integration\", \"state\"]",
                ),
            ),
        ] {
            let tgz = tarball(&[(&format!("{bin}-1.0.0/{bin}"), FAKE.as_bytes())]);
            put(
                &fx.root.join(format!(
                    "rel/fredericrous/{bin}/releases/download/v1.0.0/{bin}-1.0.0-{t}.tar.gz"
                )),
                &tgz,
            );
            let bundle = if part == "gate" {
                "[part.bundle]\nskills = [\"s1\"]\n"
            } else {
                ""
            };
            catalog.push_str(&format!(
                "[[part]]\n[part.extension]\nname = \"{part}\"\nversion = \"1.0.0\"\nharness = \">=0.2\"\n\
                 summary = \"x\"\nneeds = []\n[part.tool]\nbin = \"{bin}\"\n\
                 [part.tool.release]\nrepo = \"fredericrous/{bin}\"\nversion = \"1.0.0\"\n\
                 [part.tool.release.sha256]\n{t} = \"{}\"\n{extra}{bundle}",
                sha256_hex(&tgz)
            ));
        }
        catalog.push_str(
            "[[part]]\n[part.extension]\nname = \"attest\"\nversion = \"1.0.0\"\nharness = \">=0.2\"\n\
             summary = \"x\"\nneeds = [\"gate\"]\n\
             [[part.repo_file]]\ndirs = [\".forgejo\", \".github\"]\npath = \"workflows/attest.yml\"\n\
             content = \"name: attest\\non:\\n  pull_request:\\njobs:\\n  attest:\\n    runs-on: ubuntu-latest\\n    steps:\\n      - uses: actions/checkout@v4\\n      - uses: fredericrous/attest@v1\\n\"\n\
             [[part]]\n[part.extension]\nname = \"design\"\nversion = \"1.0.0\"\nharness = \">=0.2\"\n\
             summary = \"x\"\nneeds = []\npublished = false\n\
             [[part]]\n[part.extension]\nname = \"lab\"\nversion = \"1.0.0\"\nharness = \">=0.4\"\n\
             summary = \"x\"\nneeds = []\nexperimental = true\n",
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

fn code(output: &Output) -> i32 {
    output.status.code().unwrap()
}

fn err(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn install_solo_places_binaries_and_runs_the_tools_installs() {
    let fx = Fx::new();
    let output = fx.grh("work", &[], &["install", "gate", "guardrails", "--yes"]);
    assert_eq!(code(&output), 0, "{}", err(&output));
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
    assert_eq!(
        code(&fx.grh("work", &[], &["install", "gate", "guardrails", "--yes"])),
        0
    );
    let entries = fx.entries();
    let installs = fx.log().matches("install --write").count();
    let output = fx.grh("work", &[], &["install", "gate", "guardrails", "--yes"]);
    assert_eq!(code(&output), 0, "{}", err(&output));
    assert_eq!(fx.entries(), entries);
    assert_eq!(fx.log().matches("install --write").count(), installs);
    assert!(!fx.receipt().contains("tool-install-adopted"));
    assert!(!fx.receipt().contains("binary-adopted"));
}

#[test]
fn a_present_integration_is_adopted_and_never_undone() {
    let fx = Fx::new();
    fs::write(fx.p("fstate/amont-agent.machine"), "").unwrap();
    assert_eq!(
        code(&fx.grh("work", &[], &["install", "gate", "guardrails", "--yes"])),
        0
    );
    assert!(fx.receipt().contains("tool-install-adopted"));
    assert!(!fx.log().contains("amont-agent install --write"));
    let output = fx.grh("work", &[], &["uninstall", "--yes"]);
    assert_eq!(code(&output), 0, "{}", err(&output));
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
    let output = fx.grh(
        "work",
        &[("FAKE_UNKNOWN", "amont-agent")],
        &["install", "guardrails", "--yes"],
    );
    assert_eq!(code(&output), 0, "{}", err(&output));
    assert!(!fx.log().contains("install --write"));
    assert!(!fx.receipt().contains("tool-install"));
    assert!(!fx.p("fstate/amont-agent.machine").exists());
    assert!(err(&output).contains("next:"), "{}", err(&output));
}

#[test]
fn a_missing_claude_is_a_reported_gap() {
    let fx = Fx::new();
    fs::remove_file(fx.p("fakes/claude")).unwrap();
    let output = fx.grh("work", &[], &["install", "guardrails", "--yes"]);
    assert_eq!(code(&output), 0, "{}", err(&output));
    assert!(!fx.log().contains("install --write"));
    assert!(err(&output).contains("claude"));
}

#[test]
fn bare_uninstall_removes_everything_it_placed() {
    let fx = Fx::new();
    assert_eq!(
        code(&fx.grh("work", &[], &["install", "gate", "guardrails", "--yes"])),
        0
    );
    let output = fx.grh("work", &[], &["uninstall", "--yes"]);
    assert_eq!(code(&output), 0, "{}", err(&output));
    assert!(fx.files("bin").is_empty());
    assert!(fx.files("claude").is_empty());
    assert!(fx.files("home").is_empty());
    assert!(!fx.p("fstate/amont-agent.machine").exists());
    assert!(fx.log().contains("amont-agent uninstall --write"));
}

#[test]
fn a_binary_whose_integration_survives_the_undo_is_kept() {
    let fx = Fx::new();
    assert_eq!(
        code(&fx.grh("work", &[], &["install", "gate", "guardrails", "--yes"])),
        0
    );
    let output = fx.grh(
        "work",
        &[("FAKE_STICKY", "amont-agent")],
        &["uninstall", "--yes"],
    );
    assert_eq!(code(&output), 1, "{}", err(&output));
    assert_eq!(fx.files("bin"), ["amont-agent"]);
    assert!(
        err(&output).contains("grh uninstall guardrails"),
        "{}",
        err(&output)
    );
    assert!(fx.receipt().contains("tool-install"));
    // The retry works once the tool lets go.
    let output = fx.grh("work", &[], &["uninstall", "guardrails", "--yes"]);
    assert_eq!(code(&output), 0, "{}", err(&output));
    assert!(fx.files("bin").is_empty());
}

#[test]
fn without_yes_on_a_non_tty_nothing_changes() {
    let fx = Fx::new();
    let output = fx.grh("work", &[], &["install", "gate", "guardrails"]);
    assert_eq!(code(&output), 2, "{}", err(&output));
    assert!(fx.files("bin").is_empty());
    assert!(!fx.p("state/grh").exists());
    assert!(fx.log().is_empty());
}

#[test]
fn the_all_failure_and_the_listing_heading_fit_in_80_columns() {
    let fx = Fx::new();
    let output = fx.grh("work", &[], &["install", "--all", "gate", "--yes"]);
    assert_eq!(code(&output), 2);
    let message = err(&output);
    assert!(message.contains("name no parts with it"), "{message}");
    for line in message.lines() {
        assert!(line.chars().count() <= 80, "too wide: {line}");
    }
    let output = fx.grh("work", &[], &["install"]);
    let listing = err(&output);
    let heading: Vec<&str> = listing.lines().take_while(|l| *l != "parts:").collect();
    assert!(heading.len() >= 2, "{listing}");
    assert!(heading.iter().any(|l| l.contains("grh install --all")));
    for line in heading {
        assert!(line.chars().count() <= 80, "too wide: {line}");
    }
}

#[test]
fn a_bare_install_lists_parts_and_exits_2() {
    let fx = Fx::new();
    let output = fx.grh("work", &[], &["install"]);
    assert_eq!(code(&output), 2);
    assert!(err(&output).contains("gate") && err(&output).contains("--all"));
    assert!(
        !err(&output).contains("solo") && !err(&output).contains("profile"),
        "{}",
        err(&output)
    );
    assert!(output.stdout.is_empty());
}

#[test]
fn a_bare_install_with_yes_still_lists_parts_and_exits_2() {
    let fx = Fx::new();
    let output = fx.grh("work", &[], &["install", "--yes"]);
    assert_eq!(code(&output), 2);
    assert!(err(&output).contains("--all"), "{}", err(&output));
    assert!(output.stdout.is_empty());
}

#[test]
fn a_bare_install_with_stdout_in_a_file_leaves_the_file_empty() {
    let fx = Fx::new();
    let file = fs::File::create(fx.p("stdout.txt")).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_grh"))
        .arg("install")
        .current_dir(fx.p("work"))
        .env_clear()
        .env("HOME", fx.p("home"))
        .env("XDG_STATE_HOME", fx.p("state"))
        .env("GRH_CATALOG", fx.p("catalog.toml"))
        .stdin(Stdio::null())
        .stdout(file)
        .output()
        .unwrap();
    assert_eq!(code(&output), 2);
    assert!(err(&output).contains("--all"), "{}", err(&output));
    assert_eq!(fs::read_to_string(fx.p("stdout.txt")).unwrap(), "");
}

fn out(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn a_retired_name_exits_2_with_the_hint_and_changes_nothing() {
    let fx = Fx::new();
    assert_eq!(code(&fx.grh("work", &[], &["install", "gate", "--yes"])), 0);
    let receipt = fx.receipt();
    let log = fx.log();
    let files = fx.files("bin");
    let hint = |verb: &str, what: &str, members: &str| {
        format!(
            "profiles were removed in 0.3.0.\n  to {what} what solo installed: grh {verb} {members}"
        )
    };
    for (args, expected) in [
        (
            &["install", "solo", "--yes"][..],
            hint("install", "install", "gate guardrails")
                + "\n  or every part but the experiments: grh install --all",
        ),
        (
            &["install", "solo", "gate", "--yes"],
            hint("install", "install", "gate guardrails")
                + "\n  or every part but the experiments: grh install --all",
        ),
        (
            &["uninstall", "solo", "--yes"],
            hint("uninstall", "remove", "gate guardrails"),
        ),
    ] {
        let output = fx.grh("work", &[], args);
        assert_eq!(code(&output), 2, "{args:?}: {}", err(&output));
        assert!(
            err(&output).contains(&expected),
            "{args:?}: {}",
            err(&output)
        );
        assert!(out(&output).is_empty(), "{args:?}: {}", out(&output));
        assert_eq!(fx.receipt(), receipt);
        assert_eq!(fx.log(), log);
        assert_eq!(fx.files("bin"), files);
    }
}

#[test]
fn all_with_yes_installs_every_published_part() {
    let fx = Fx::new();
    let output = fx.grh("work", &[], &["install", "--all", "--yes"]);
    assert_eq!(code(&output), 0, "{}", err(&output));
    assert_eq!(fx.files("bin"), ["amont", "amont-agent", "aval"]);
    let receipt = fx.receipt();
    for part in ["decisions", "gate", "guardrails"] {
        assert!(
            receipt.contains(&format!("\"part\": \"{part}\"")),
            "{part}: {receipt}"
        );
    }
    assert!(!receipt.contains("design"), "{receipt}");
    assert!(!receipt.contains("lab"), "{receipt}");
    assert!(
        err(&output).contains("left out: lab (an experiment: install it by name)"),
        "{}",
        err(&output)
    );
    let named = fx.grh("work", &[], &["install", "lab", "--yes"]);
    assert_eq!(code(&named), 0, "{}", err(&named));
    assert!(err(&named).contains("lab: x"), "{}", err(&named));
    let entries = fx.entries();
    assert_eq!(
        code(&fx.grh("work", &[], &["install", "--all", "--yes"])),
        0
    );
    assert_eq!(fx.entries(), entries);
}

#[test]
fn all_without_yes_off_a_terminal_exits_2_naming_yes() {
    let fx = Fx::new();
    let output = fx.grh("work", &[], &["install", "--all"]);
    assert_eq!(code(&output), 2, "{}", err(&output));
    assert!(
        err(&output).contains("grh install --all --yes"),
        "{}",
        err(&output)
    );
    assert!(fx.files("bin").is_empty());
    assert!(!fx.p("state/grh").exists());
}

#[test]
fn all_with_names_or_an_extension_exits_2() {
    let fx = Fx::new();
    for args in [
        &["install", "--all", "gate", "--yes"][..],
        &["install", "--all", "--extension", "somewhere", "--yes"],
    ] {
        let output = fx.grh("work", &[], args);
        assert_eq!(code(&output), 2, "{args:?}: {}", err(&output));
        assert!(fx.files("bin").is_empty());
        assert!(!fx.p("state/grh").exists());
    }
}

#[test]
fn all_skips_a_part_whose_need_failed_and_exits_1_naming_both() {
    let fx = Fx::new();
    let t = target().unwrap();
    fs::remove_file(fx.p(&format!(
        "rel/fredericrous/amont/releases/download/v1.0.0/amont-1.0.0-{t}.tar.gz"
    )))
    .unwrap();
    git_init(&fx);
    let output = fx.grh("work", &[], &["install", "--all", "--repo", "--yes"]);
    assert_eq!(code(&output), 1, "{}", err(&output));
    assert!(
        err(&output).contains("gate did not install; skipped because of it: attest; the other parts are installed. Fix the errors above, then run grh install --all --repo --yes"),
        "{}",
        err(&output)
    );
    let receipt = fx.receipt();
    assert!(!receipt.contains("\"part\": \"attest\""), "{receipt}");
    assert!(receipt.contains("\"part\": \"guardrails\""), "{receipt}");
    assert!(!receipt.contains("\"part\": \"gate\""), "{receipt}");
    assert!(!fx.p("work/.github/workflows/attest.yml").exists());
}

#[test]
fn a_named_install_whose_binary_cannot_be_fetched_places_no_skills() {
    let fx = Fx::new();
    let t = target().unwrap();
    fs::remove_file(fx.p(&format!(
        "rel/fredericrous/amont/releases/download/v1.0.0/amont-1.0.0-{t}.tar.gz"
    )))
    .unwrap();
    let output = fx.grh("work", &[], &["install", "gate", "--yes"]);
    assert_ne!(code(&output), 0, "{}", err(&output));
    // The skill would point at a tool that is not there, and the receipt must
    // not record a part that did not install.
    assert!(!fx.p("claude/skills/s1/SKILL.md").exists());
    assert!(
        !fx.receipt().contains("\"part\": \"gate\""),
        "{}",
        fx.receipt()
    );
}

#[test]
fn the_consent_hint_for_all_keeps_the_flags_given() {
    let fx = Fx::new();
    git_init(&fx);
    let output = fx.grh("work", &[], &["install", "--all", "--repo"]);
    assert_eq!(code(&output), 2, "{}", err(&output));
    assert!(
        err(&output).contains("grh install --all --repo --yes"),
        "{}",
        err(&output)
    );
    assert!(output.stdout.is_empty());
}

#[test]
fn all_skips_a_part_whose_need_installed_its_binary_but_failed_its_integration() {
    let fx = Fx::new();
    git_init(&fx);
    let catalog = fs::read_to_string(fx.p("catalog.toml"))
        .unwrap()
        .replace("[\"{bin}\", \"init\"]", "[\"/usr/bin/false\"]");
    put(&fx.p("catalog.toml"), catalog.as_bytes());
    let output = fx.grh("work", &[], &["install", "--all", "--repo", "--yes"]);
    assert_eq!(code(&output), 1, "{}", err(&output));
    assert!(
        err(&output).contains("gate did not install; skipped because of it: attest; the other parts are installed. Fix the errors above, then run grh install --all --repo --yes"),
        "{}",
        err(&output)
    );
    let receipt = fx.receipt();
    assert!(!receipt.contains("\"part\": \"attest\""), "{receipt}");
    assert!(receipt.contains("\"part\": \"guardrails\""), "{receipt}");
    assert!(!fx.p("work/.github/workflows/attest.yml").exists());

    // A rerun fails the same way and leaves the parts that installed as they
    // were.
    let entries = fx.entries();
    let output = fx.grh("work", &[], &["install", "--all", "--repo", "--yes"]);
    assert_eq!(code(&output), 1, "{}", err(&output));
    assert_eq!(fx.entries(), entries, "the rerun changed the receipt");
}

#[test]
fn help_wins_over_an_unknown_flag_and_the_flag_alone_exits_2() {
    let fx = Fx::new();
    let out = |output: &Output| String::from_utf8_lossy(&output.stdout).into_owned();
    for args in [&["--bogus", "--help"][..], &["--bogus", "-h"], &["--help"]] {
        let output = fx.grh("work", &[], args);
        assert_eq!(code(&output), 0, "{args:?}: {}", err(&output));
        assert!(
            out(&output).contains("Usage: grh"),
            "{args:?}: {}",
            out(&output)
        );
    }
    let output = fx.grh("work", &[], &["install", "--bogus", "--help"]);
    assert_eq!(code(&output), 0, "{}", err(&output));
    assert!(
        out(&output).contains("Usage: grh install"),
        "{}",
        out(&output)
    );
    let output = fx.grh("work", &[], &["--bogus-flag-for-probe"]);
    assert_eq!(code(&output), 2);
    assert!(
        out(&output).is_empty(),
        "the usage error goes to stderr: {}",
        out(&output)
    );
}

#[test]
fn dry_run_changes_nothing() {
    let fx = Fx::new();
    let output = fx.grh("work", &[], &["install", "gate", "guardrails", "--dry-run"]);
    assert_eq!(code(&output), 0, "{}", err(&output));
    assert!(fx.files("bin").is_empty());
    assert!(!fx.p("state/grh").exists());
}

#[test]
fn installing_design_exits_3_after_installing_the_rest() {
    let fx = Fx::new();
    let output = fx.grh(
        "work",
        &[],
        &["install", "gate", "guardrails", "design", "--yes"],
    );
    assert_eq!(code(&output), 3, "{}", err(&output));
    assert_eq!(fx.files("bin"), ["amont", "amont-agent"]);
}

#[test]
fn uninstall_warns_about_a_dependent_and_keep_deps_refuses() {
    let fx = Fx::new();
    git_init(&fx);
    let output = fx.grh("work", &[], &["install", "attest", "--repo", "--yes"]);
    assert_eq!(code(&output), 0, "{}", err(&output));
    assert!(fx.p("work/.github/workflows/attest.yml").exists());
    assert!(fx.log().contains("amont init"));
    let output = fx.grh("work", &[], &["uninstall", "gate", "--keep-deps", "--yes"]);
    assert_eq!(code(&output), 1);
    assert!(
        err(&output).contains("grh uninstall attest"),
        "{}",
        err(&output)
    );
    let output = fx.grh("work", &[], &["uninstall", "gate", "--yes"]);
    assert_eq!(code(&output), 0);
    assert!(
        err(&output).contains("grh uninstall attest"),
        "{}",
        err(&output)
    );
    // --repo narrows to this repository: the workflow goes, nothing else.
    let output = fx.grh("work", &[], &["uninstall", "--repo", "--yes"]);
    assert_eq!(code(&output), 0, "{}", err(&output));
    assert!(!fx.p("work/.github").exists());
}

#[test]
fn status_json_is_one_document_on_stdout() {
    let fx = Fx::new();
    assert_eq!(
        code(&fx.grh("work", &[], &["install", "gate", "guardrails", "--yes"])),
        0
    );
    let output = fx.grh("work", &[], &["status", "--json"]);
    assert_eq!(code(&output), 0, "{}", err(&output));
    let v: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
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
    let output = fx.grh("work", &[], &["install", "guardrails", "--yes"]);
    assert_eq!(code(&output), 0, "{}", err(&output));
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
    let output = fx.grh("work", &[], &["install", "gate", "--yes"]);
    assert_eq!(code(&output), 0, "{}", err(&output));
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
    let output = fx.grh("work", &[], &["uninstall", "--yes"]);
    assert_eq!(code(&output), 0, "{}", err(&output));
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
    let output = fx.grh("work", &[], &["status", "--json"]);
    assert_eq!(code(&output), 0, "{}", err(&output));
    let v: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
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
    let output = fx.grh("work", &[], &["install", "guardrails", "--yes"]);
    assert_eq!(code(&output), 0, "{}", err(&output));
    assert!(!fx.log().contains("install --write"), "{}", fx.log());
    assert!(!fx.receipt().contains("tool-install"), "{}", fx.receipt());
    assert!(
        err(&output).contains("partly installed"),
        "{}",
        err(&output)
    );
}

#[test]
fn an_existing_mcp_server_is_skipped_and_survives_uninstall() {
    let fx = Fx::new();
    fs::write(fx.p("fstate/mcp.srv"), "").unwrap();
    let dir = extension_dir(&fx);
    let output = fx.grh("work", &[], &["install", "--extension", &dir, "--yes"]);
    assert_eq!(code(&output), 0, "{}", err(&output));
    assert!(!fx.log().contains("mcp add"), "{}", fx.log());
    assert!(!fx.receipt().contains("tool-install"), "{}", fx.receipt());
    let output = fx.grh("work", &[], &["uninstall", "--yes"]);
    assert_eq!(code(&output), 0, "{}", err(&output));
    assert!(!fx.log().contains("mcp remove"), "{}", fx.log());
    assert!(fx.p("fstate/mcp.srv").exists());
}

#[test]
fn a_failed_mcp_add_leaves_no_entry() {
    let fx = Fx::new();
    let dir = extension_dir(&fx);
    let output = fx.grh(
        "work",
        &[("FAKE_ADD_FAIL", "1")],
        &["install", "--extension", &dir, "--yes"],
    );
    assert_eq!(code(&output), 1, "{}", err(&output));
    assert!(!fx.receipt().contains("tool-install"), "{}", fx.receipt());
    let output = fx.grh("work", &[], &["uninstall", "--yes"]);
    assert_eq!(code(&output), 0, "{}", err(&output));
    assert!(!fx.log().contains("mcp remove"), "{}", fx.log());
}

#[test]
fn an_mcp_server_grh_added_is_removed_only_when_claude_reports_it_gone() {
    let fx = Fx::new();
    let dir = extension_dir(&fx);
    let output = fx.grh("work", &[], &["install", "--extension", &dir, "--yes"]);
    assert_eq!(code(&output), 0, "{}", err(&output));
    assert!(fx.p("fstate/mcp.srv").exists());
    let output = fx.grh(
        "work",
        &[("FAKE_REMOVE_STICKY", "1")],
        &["uninstall", "--yes"],
    );
    assert_eq!(code(&output), 1, "{}", err(&output));
    assert!(fx.receipt().contains("tool-install"), "{}", fx.receipt());
    let output = fx.grh("work", &[], &["uninstall", "--yes"]);
    assert_eq!(code(&output), 0, "{}", err(&output));
    assert!(!fx.p("fstate/mcp.srv").exists());
    assert!(!fx.receipt().contains("tool-install"), "{}", fx.receipt());
}

#[test]
fn the_gate_binary_is_kept_while_the_template_dir_bakes_it_and_removed_when_absent() {
    let fx = Fx::new();
    assert_eq!(code(&fx.grh("work", &[], &["install", "gate", "--yes"])), 0);
    let bin = fx.p("bin/amont").to_string_lossy().into_owned();
    let output = fx.grh(
        "work",
        &[("FAKE_TEMPLATE", "present"), ("FAKE_TEMPLATE_BIN", &bin)],
        &["uninstall", "gate", "--yes"],
    );
    assert_eq!(code(&output), 0, "{}", err(&output));
    assert_eq!(fx.files("bin"), ["amont"]);
    assert!(err(&output).contains("kept"), "{}", err(&output));
    assert!(fx.receipt().contains("binary-placed"));
    // A present answer whose shims cannot be read is unknown: kept.
    for raw in [
        r#"{"state":"present"}"#,
        r#"{"state":"present","shims":[]}"#,
        r#"{"state":"present","shims":[{"binary":null}]}"#,
    ] {
        let output = fx.grh(
            "work",
            &[("FAKE_TEMPLATE_RAW", raw)],
            &["uninstall", "gate", "--yes"],
        );
        assert_eq!(code(&output), 0, "{}", err(&output));
        assert_eq!(fx.files("bin"), ["amont"], "{raw}: {}", err(&output));
    }
    // A failed query keeps it too.
    let output = fx.grh(
        "work",
        &[("FAKE_UNKNOWN", "amont")],
        &["uninstall", "gate", "--yes"],
    );
    assert_eq!(code(&output), 0, "{}", err(&output));
    assert_eq!(fx.files("bin"), ["amont"]);
    // Absent: removed, with the warning.
    let output = fx.grh("work", &[], &["uninstall", "gate", "--yes"]);
    assert_eq!(code(&output), 0, "{}", err(&output));
    assert!(fx.files("bin").is_empty());
    assert!(err(&output).contains("amont init"), "{}", err(&output));
}

#[test]
fn an_existing_attest_workflow_with_identical_bytes_is_not_ours_to_remove() {
    let fx = Fx::new();
    git_init(&fx);
    put(
        &fx.p("work/.github/workflows/attest.yml"),
        ATTEST_BYTES.as_bytes(),
    );
    let output = fx.grh("work", &[], &["install", "attest", "--repo", "--yes"]);
    assert_eq!(code(&output), 0, "{}", err(&output));
    assert!(!fx.receipt().contains("attest.yml"), "{}", fx.receipt());
    let output = fx.grh("work", &[], &["uninstall", "--yes"]);
    assert_eq!(code(&output), 0, "{}", err(&output));
    assert!(fx.p("work/.github/workflows/attest.yml").exists());
}

#[test]
fn a_failing_tool_uninstall_keeps_the_entry_and_exits_1() {
    let fx = Fx::new();
    assert_eq!(
        code(&fx.grh("work", &[], &["install", "guardrails", "--yes"])),
        0
    );
    let output = fx.grh(
        "work",
        &[("FAKE_FAIL_UNINSTALL", "amont-agent")],
        &["uninstall", "--yes"],
    );
    assert_eq!(code(&output), 1, "{}", err(&output));
    assert!(fx.receipt().contains("tool-install"), "{}", fx.receipt());
    assert_eq!(fx.files("bin"), ["amont-agent"]);
}

#[test]
fn an_unreachable_armed_repo_keeps_its_entry_and_the_binary_and_exits_1() {
    let fx = Fx::new();
    git_init(&fx);
    let output = fx.grh("work", &[], &["install", "gate", "--repo", "--yes"]);
    assert_eq!(code(&output), 0, "{}", err(&output));
    fs::remove_dir_all(fx.p("work")).unwrap();
    let output = fx.grh("home", &[], &["uninstall", "--yes"]);
    assert_eq!(code(&output), 1, "{}", err(&output));
    assert!(fx.receipt().contains("tool-install"), "{}", fx.receipt());
    assert_eq!(fx.files("bin"), ["amont"]);
}

/// Replace the fixture catalog with the shipped one, whose parts carry the
/// audit rules.
fn use_shipped_catalog(fx: &Fx) {
    put(
        &fx.p("catalog.toml"),
        include_str!("../catalog.toml").as_bytes(),
    );
}

fn audit_install(fx: &Fx) -> (Option<String>, Vec<String>) {
    git_init(fx);
    let output = fx.grh("work", &[], &["audit", "--json"]);
    assert_eq!(code(&output), 0, "{}", err(&output));
    let v: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(v["schema"], 2);
    let warnings = v["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| w.as_str().unwrap().to_string())
        .collect();
    (v["install"].as_str().map(str::to_string), warnings)
}

#[test]
fn audit_of_a_rust_repo_with_husky_recommends_the_gates_and_warns() {
    let fx = Fx::new();
    use_shipped_catalog(&fx);
    put(&fx.p("work/src/main.rs"), b"fn main() {}\n");
    put(&fx.p("work/.husky/pre-commit"), b"#!/bin/sh\n");
    let (install, warnings) = audit_install(&fx);
    assert_eq!(install.as_deref(), Some("grh install greengate greenpiste"));
    assert!(warnings.iter().any(|w| w.contains("husky")), "{warnings:?}");
}

#[test]
fn audit_of_a_typescript_repo_with_adrs_recommends_decisions_and_conform() {
    let fx = Fx::new();
    use_shipped_catalog(&fx);
    put(&fx.p("work/src/a.ts"), b"export {};\n");
    put(&fx.p("work/docs/adr/0001-x.md"), b"# 1\n");
    let (install, _) = audit_install(&fx);
    assert_eq!(
        install.as_deref(),
        Some("grh install greengate greenpiste decisions conform")
    );
}

#[test]
fn audit_of_an_empty_repo_with_no_rules_has_nothing_to_install() {
    let fx = Fx::new();
    let (install, warnings) = audit_install(&fx);
    assert_eq!(install, None);
    assert!(warnings.is_empty(), "{warnings:?}");
    let output = fx.grh("work", &[], &["audit"]);
    assert_eq!(code(&output), 0, "{}", err(&output));
    let text = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(text.trim_end().ends_with("nothing to install"), "{text}");
}

#[test]
fn audit_ending_on_a_closed_pipe_exits_0_without_a_panic() {
    let fx = Fx::new();
    use_shipped_catalog(&fx);
    git_init(&fx);
    let mut child = Command::new(env!("CARGO_BIN_EXE_grh"))
        .arg("audit")
        .current_dir(fx.p("work"))
        .env_clear()
        .env("HOME", fx.p("home"))
        .env("XDG_STATE_HOME", fx.p("state"))
        .env("GRH_CATALOG", fx.p("catalog.toml"))
        .env("PATH", "/usr/bin:/bin")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    drop(child.stdout.take());
    let output = child.wait_with_output().unwrap();
    assert_eq!(code(&output), 0, "{}", err(&output));
    assert!(!err(&output).contains("panicked"), "{}", err(&output));
}

#[test]
fn a_receipt_written_under_an_alias_is_read_by_status_and_migrated_by_a_locked_command() {
    let fx = Fx::new();
    let catalog = fs::read_to_string(fx.p("catalog.toml"))
        .unwrap()
        .replace("\"gate\"", "\"greengate\"")
        .replace(
            "name = \"greengate\"",
            "name = \"greengate\"\naliases = [\"gate\"]",
        );
    put(&fx.p("catalog.toml"), catalog.as_bytes());
    let bin = fx.p("bin/amont");
    executable(&bin, FAKE);
    // A skill placed while the part was still called `gate`: uninstall must
    // remove it too, through the alias.
    let skill = fx.p("claude/skills/legacy-gate-skill/SKILL.md");
    let skill_bytes = b"# a skill placed under the old name\n";
    put(&skill, skill_bytes);
    seed_receipt(
        &fx,
        &serde_json::json!([
            {
                "part": "gate", "level": "machine", "state": "done",
                "kind": { "binary-adopted": { "path": bin.to_string_lossy() } }
            },
            {
                "part": "gate", "level": "machine", "state": "done",
                "kind": { "file": {
                    "path": skill.to_string_lossy(),
                    "sha256": sha256_hex(skill_bytes),
                } }
            }
        ]),
    );
    let parts_named = |fx: &Fx, name: &str| {
        let v: serde_json::Value = serde_json::from_str(&fx.receipt()).unwrap();
        v["entries"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|e| e["part"] == name)
            .count()
    };

    let output = fx.grh("work", &[], &["status", "--json"]);
    assert_eq!(code(&output), 0, "{}", err(&output));
    let status: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let gate = status["parts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == "greengate")
        .unwrap();
    assert_eq!(gate["installed"], true);
    assert_eq!(gate["binary"], bin.to_string_lossy().as_ref());
    assert_eq!(parts_named(&fx, "gate"), 2);

    let output = fx.grh("work", &[], &["install", "greengate", "--yes"]);
    assert_eq!(code(&output), 0, "{}", err(&output));
    assert_eq!(parts_named(&fx, "gate"), 0, "{}", fx.receipt());
    assert_eq!(parts_named(&fx, "greengate"), 3, "{}", fx.receipt());
    assert!(fx.receipt().contains("binary-adopted"), "{}", fx.receipt());

    let output = fx.grh("work", &[], &["uninstall", "greengate", "--yes"]);
    assert_eq!(code(&output), 0, "{}", err(&output));
    assert_eq!(parts_named(&fx, "gate"), 0, "{}", fx.receipt());
    assert_eq!(parts_named(&fx, "greengate"), 0, "{}", fx.receipt());
    assert!(
        !skill.exists(),
        "the skill recorded under `gate` was left behind"
    );
}

// --- a manifest's own data drives the install path ---

fn stripped_of_recorded_specs(fx: &Fx) {
    let mut doc: serde_json::Value = serde_json::from_str(&fx.receipt()).unwrap();
    for entry in doc["entries"].as_array_mut().unwrap() {
        if let Some(tool) = entry["kind"]
            .get_mut("tool-install")
            .and_then(serde_json::Value::as_object_mut)
        {
            tool.remove("status");
            tool.remove("keep_binary");
        }
    }
    put(&fx.p("state/grh/receipt.json"), doc.to_string().as_bytes());
}

#[test]
fn uninstall_of_an_entry_written_by_0_1_x_takes_its_spec_from_the_catalog() {
    let fx = Fx::new();
    assert_eq!(
        code(&fx.grh("work", &[], &["install", "guardrails", "--yes"])),
        0
    );
    assert!(fx.receipt().contains("\"status\""), "{}", fx.receipt());
    stripped_of_recorded_specs(&fx);
    assert!(!fx.receipt().contains("\"status\""));
    let output = fx.grh("work", &[], &["uninstall", "guardrails", "--yes"]);
    assert_eq!(code(&output), 0, "{}", err(&output));
    assert!(!fx.receipt().contains("tool-install"), "{}", fx.receipt());
    assert!(!fx.p("fstate/amont-agent.machine").exists());
    assert!(fx.files("bin").is_empty());
}

#[test]
fn a_missing_requires_leaves_a_gap_naming_the_command_and_the_rerun() {
    let fx = Fx::new();
    fs::remove_file(fx.p("fakes/claude")).unwrap();
    let output = fx.grh("work", &[], &["install", "guardrails", "--yes"]);
    assert_eq!(code(&output), 0, "{}", err(&output));
    let text = err(&output);
    assert!(text.contains("claude is not on PATH"), "{text}");
    assert!(text.contains("grh install guardrails"), "{text}");
}

#[test]
fn attest_workflow_is_placed_from_the_manifest_with_backup_rules() {
    let fx = Fx::new();
    git_init(&fx);
    let output = fx.grh("work", &[], &["install", "attest", "--repo", "--yes"]);
    assert_eq!(code(&output), 0, "{}", err(&output));
    let file = fx.p("work/.forgejo/workflows/attest.yml");
    assert!(
        !file.exists() || fs::read_to_string(&file).unwrap() == ATTEST_BYTES,
        "unexpected content"
    );
    let placed = if file.exists() {
        file
    } else {
        fx.p("work/.github/workflows/attest.yml")
    };
    assert_eq!(fs::read_to_string(&placed).unwrap(), ATTEST_BYTES);
    // Someone edits it: grh leaves it, then backs it up under --replace.
    fs::write(&placed, "edited\n").unwrap();
    let output = fx.grh("work", &[], &["install", "attest", "--repo", "--yes"]);
    assert_eq!(code(&output), 0, "{}", err(&output));
    assert_eq!(fs::read_to_string(&placed).unwrap(), "edited\n");
    assert!(
        err(&output).contains("grh install attest --repo --replace"),
        "{}",
        err(&output)
    );
    let output = fx.grh(
        "work",
        &[],
        &["install", "attest", "--repo", "--replace", "--yes"],
    );
    assert_eq!(code(&output), 0, "{}", err(&output));
    assert_eq!(fs::read_to_string(&placed).unwrap(), ATTEST_BYTES);
    assert_eq!(fx.files("state/grh/backup").len(), 1);
}

#[test]
fn a_repo_file_through_a_link_out_of_the_repository_is_refused() {
    let fx = Fx::new();
    git_init(&fx);
    fs::create_dir_all(fx.p("outside")).unwrap();
    std::os::unix::fs::symlink(fx.p("outside"), fx.p("work/.forgejo")).unwrap();
    let output = fx.grh("work", &[], &["install", "attest", "--repo", "--yes"]);
    assert_eq!(code(&output), 1, "{}", err(&output));
    assert!(
        err(&output).contains("outside the repository"),
        "{}",
        err(&output)
    );
    assert!(fx.files("outside").is_empty());
    assert!(!fx.receipt().contains("attest.yml"));
}

#[test]
fn a_repo_file_path_with_dotdot_or_absolute_is_refused() {
    for path in ["../escape.yml", "/tmp/escape.yml"] {
        let fx = Fx::new();
        git_init(&fx);
        put(
            &fx.p("ext/greenharness.toml"),
            format!(
                "[extension]\nname = \"x\"\nversion = \"1.0.0\"\nharness = \">=0.2\"\n\n\
                 [[repo_file]]\ndirs = [\".github\"]\npath = \"{path}\"\ncontent = \"x\"\n"
            )
            .as_bytes(),
        );
        let dir = fx.p("ext").to_string_lossy().into_owned();
        let output = fx.grh(
            "work",
            &[],
            &["install", "--extension", &dir, "--repo", "--yes"],
        );
        assert_ne!(code(&output), 0, "{path}: {}", err(&output));
        assert!(!fx.p("work/escape.yml").exists());
        assert!(!fx.receipt().contains("escape.yml"));
    }
}

const TOOL_EXT: &str = r#"#!/bin/sh
echo "exttool $*" >> "$FAKE_LOG"
mkdir -p "$FAKE_STATE"
case "$1" in
  install) touch "$FAKE_STATE/exttool.on";;
  uninstall) rm -f "$FAKE_STATE/exttool.on";;
  status) [ -f "$FAKE_STATE/exttool.on" ] && exit 0; exit 1;;
esac
exit 0
"#;

fn tool_extension(fx: &Fx, extra: &str) -> String {
    executable(&fx.p("fakes/exttool"), TOOL_EXT);
    put(
        &fx.p("ext/greenharness.toml"),
        format!(
            "[extension]\nname = \"withtool\"\nversion = \"1.0.0\"\nharness = \">=0.2\"\n\n\
             [tool]\nbin = \"exttool\"\n[tool.machine]\n\
             install = [\"{{bin}}\", \"install\", \"a b\"]\nuninstall = [\"{{bin}}\", \"uninstall\"]\n\
             {extra}[tool.machine.status]\nargs = [\"status\"]\nexit = {{ present = 0, absent = 1 }}\n"
        )
        .as_bytes(),
    );
    fx.p("ext").to_string_lossy().into_owned()
}

#[test]
fn an_extension_with_a_tool_installs_and_uninstalls_without_its_directory() {
    let fx = Fx::new();
    let dir = tool_extension(&fx, "");
    let output = fx.grh("work", &[], &["install", "--extension", &dir, "--yes"]);
    assert_eq!(code(&output), 0, "{}", err(&output));
    assert!(fx.p("fstate/exttool.on").exists());
    assert!(fx.receipt().contains("\"status\""), "{}", fx.receipt());
    fs::remove_dir_all(fx.p("ext")).unwrap();
    let output = fx.grh("work", &[], &["uninstall", "--yes"]);
    assert_eq!(code(&output), 0, "{}", err(&output));
    assert!(fx.log().contains("exttool uninstall"), "{}", fx.log());
    assert!(!fx.p("fstate/exttool.on").exists());
    assert!(!fx.receipt().contains("tool-install"), "{}", fx.receipt());
}

#[test]
fn an_extension_whose_tool_was_present_leaves_no_entry_after_uninstall() {
    let fx = Fx::new();
    let dir = tool_extension(&fx, "");
    put(&fx.p("fstate/exttool.on"), b"");
    let output = fx.grh("work", &[], &["install", "--extension", &dir, "--yes"]);
    assert_eq!(code(&output), 0, "{}", err(&output));
    assert!(fx.receipt().contains("ext:"), "{}", fx.receipt());
    let output = fx.grh("work", &[], &["uninstall", "--yes"]);
    assert_eq!(code(&output), 0, "{}", err(&output));
    assert!(!fx.receipt().contains("ext:"), "{}", fx.receipt());
    assert!(fx.p("fstate/exttool.on").exists());
}

#[test]
fn an_unpinned_manifest_lists_what_it_runs_on_stderr_and_exits_2_without_consent() {
    let fx = Fx::new();
    let dir = tool_extension(&fx, "requires = [\"somecli\"]\n");
    put(
        &fx.p("ext/greenharness.toml"),
        format!(
            "{}\n[[repo_file]]\ndirs = [\".github\"]\npath = \"workflows/x.yml\"\ncontent = \"SECRETBODY\"\n",
            fs::read_to_string(fx.p("ext/greenharness.toml")).unwrap()
        )
        .replace("SECRETBODY\"\n", "SECRETBODY\"\n\n[[doctor]]\ncommand = \"exttool check\"\n")
        .replace(
            "[[doctor]]",
            "[tool.release]\nrepo = \"someone/exttool\"\nversion = \"9.9.9\"\n\
             [tool.release.sha256]\n\
             aarch64-apple-darwin = \"0000000000000000000000000000000000000000000000000000000000000000\"\n\
             x86_64-apple-darwin = \"0000000000000000000000000000000000000000000000000000000000000000\"\n\
             aarch64-unknown-linux-gnu = \"0000000000000000000000000000000000000000000000000000000000000000\"\n\
             x86_64-unknown-linux-gnu = \"0000000000000000000000000000000000000000000000000000000000000000\"\n\n[[doctor]]",
        )
        .as_bytes(),
    );
    let output = fx.grh("work", &[], &["install", "--extension", &dir, "--quiet"]);
    assert_eq!(code(&output), 2, "{}", err(&output));
    let text = err(&output);
    assert!(text.contains("exttool install 'a b'"), "{text}");
    assert!(text.contains("exttool uninstall"), "{text}");
    assert!(text.contains("exttool status"), "{text}");
    assert!(text.contains(".github/workflows/x.yml"), "{text}");
    assert!(text.contains("somecli"), "{text}");
    assert!(text.contains("doctor:"), "{text}");
    assert!(text.contains("exttool check"), "{text}");
    assert!(
        text.contains("download: exttool 9.9.9 from github.com/someone/exttool"),
        "{text}"
    );
    assert!(!text.contains("SECRETBODY"), "{text}");
    assert!(!fx.p("fstate/exttool.on").exists());
    assert!(fx.receipt().is_empty());
}

// --- aliases and the receipt migration ---

/// The fixture catalog with `gate` renamed `greengate`, answering to `gate`.
fn alias_fx() -> Fx {
    let fx = Fx::new();
    let catalog = fs::read_to_string(fx.p("catalog.toml"))
        .unwrap()
        .replace("\"gate\"", "\"greengate\"")
        .replace(
            "name = \"greengate\"",
            "name = \"greengate\"\naliases = [\"gate\"]",
        );
    put(&fx.p("catalog.toml"), catalog.as_bytes());
    fx
}

fn seed_gate_receipt(fx: &Fx) -> String {
    let bin = fx.p("bin/amont");
    executable(&bin, FAKE);
    seed_receipt(
        fx,
        &serde_json::json!([{
            "part": "gate", "level": "machine", "state": "done",
            "kind": { "binary-adopted": { "path": bin.to_string_lossy() } }
        }]),
    );
    fx.receipt()
}

#[test]
fn the_first_locked_command_migrates_the_receipt_once_and_keeps_the_original() {
    let fx = alias_fx();
    let original = seed_gate_receipt(&fx);
    let backup = fx.p("state/grh/receipt.json.pre-0.2");

    let output = fx.grh("work", &[], &["install", "greengate", "--yes", "--quiet"]);
    assert_eq!(code(&output), 0, "{}", err(&output));
    let notice = err(&output);
    assert!(notice.contains("receipt.json.pre-0.2"), "{notice}");
    assert!(notice.contains("gate -> greengate"), "{notice}");
    let doc: serde_json::Value = serde_json::from_str(&fx.receipt()).unwrap();
    assert_eq!(doc["version"], 2);
    assert_eq!(doc["entries"][0]["part"], "greengate");
    assert_eq!(fs::read_to_string(&backup).unwrap(), original);

    // The second locked command does not migrate again: no notice, the same
    // receipt, and the backup is never written again (bytes and mtime). The
    // receipt itself may be written (an install records pending steps), so
    // its mtime is not the measure.
    let migrated = fx.receipt();
    let backup_mtime = fs::metadata(&backup).unwrap().modified().unwrap();
    std::thread::sleep(std::time::Duration::from_millis(20));
    let output = fx.grh("work", &[], &["install", "greengate", "--yes", "--quiet"]);
    assert_eq!(code(&output), 0, "{}", err(&output));
    assert!(
        !err(&output).contains("receipt.json.pre-0.2"),
        "{}",
        err(&output)
    );
    assert_eq!(fx.receipt(), migrated);
    assert_eq!(fs::read_to_string(&backup).unwrap(), original);
    assert_eq!(
        fs::metadata(&backup).unwrap().modified().unwrap(),
        backup_mtime
    );
}

#[test]
fn a_version_1_receipt_with_no_alias_is_still_raised_to_version_2() {
    let fx = alias_fx();
    let bin = fx.p("bin/aval");
    executable(&bin, FAKE);
    seed_receipt(
        &fx,
        &serde_json::json!([{
            "part": "decisions", "level": "machine", "state": "done",
            "kind": { "binary-adopted": { "path": bin.to_string_lossy() } }
        }]),
    );
    let original = fx.receipt();
    let output = fx.grh("work", &[], &["install", "greengate", "--yes"]);
    assert_eq!(code(&output), 0, "{}", err(&output));
    let doc: serde_json::Value = serde_json::from_str(&fx.receipt()).unwrap();
    assert_eq!(doc["version"], 2, "{}", fx.receipt());
    let backup = fx.p("state/grh/receipt.json.pre-0.2");
    assert_eq!(fs::read_to_string(&backup).unwrap(), original);
    assert!(err(&output).contains("no part renamed"), "{}", err(&output));
}

#[test]
fn upgrade_downgrade_upgrade_leaves_the_second_version_1_receipt_as_the_backup() {
    let fx = alias_fx();
    let first = seed_gate_receipt(&fx);
    let backup = fx.p("state/grh/receipt.json.pre-0.2");
    let output = fx.grh("work", &[], &["install", "greengate", "--yes"]);
    assert_eq!(code(&output), 0, "{}", err(&output));
    assert_eq!(fs::read_to_string(&backup).unwrap(), first);

    // A downgrade restores a different version-1 receipt, B.
    let second = {
        let doc = serde_json::json!({ "version": 1, "entries": [] });
        format!("{doc}\n")
    };
    put(&fx.p("state/grh/receipt.json"), second.as_bytes());
    let output = fx.grh("work", &[], &["install", "greengate", "--yes"]);
    assert_eq!(code(&output), 0, "{}", err(&output));
    assert_eq!(fs::read_to_string(&backup).unwrap(), second);
}

#[test]
fn uninstalling_by_an_alias_names_the_uninstall_command() {
    let fx = alias_fx();
    let output = fx.grh("work", &[], &["uninstall", "gate", "--dry-run"]);
    assert_eq!(code(&output), 0, "{}", err(&output));
    let notice = "gate is now greengate: run grh uninstall greengate";
    assert_eq!(err(&output).matches(notice).count(), 1, "{}", err(&output));
}

#[test]
fn the_install_plan_shows_the_binary_name_never_the_placeholder() {
    let fx = Fx::new();
    let output = fx.grh("work", &[], &["install", "gate", "guardrails", "--dry-run"]);
    assert_eq!(code(&output), 0, "{}", err(&output));
    let plan = String::from_utf8_lossy(&output.stdout).into_owned() + &err(&output);
    assert!(!plan.contains("{bin}"), "{plan}");
    assert!(
        plan.contains("machine: amont-agent install --write"),
        "{plan}"
    );
}

#[test]
fn status_under_a_held_lock_exits_0_and_writes_nothing() {
    let fx = alias_fx();
    let original = seed_gate_receipt(&fx);
    fs::create_dir_all(fx.p("state/grh")).unwrap();
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(fx.p("state/grh/receipt.lock"))
        .unwrap();
    lock.lock().unwrap();
    let output = fx.grh("work", &[], &["status", "--json"]);
    assert_eq!(code(&output), 0, "{}", err(&output));
    serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap();
    assert_eq!(fx.receipt(), original);
    assert!(!fx.p("state/grh/receipt.json.pre-0.2").exists());
}

#[test]
fn status_json_on_a_version_1_receipt_is_json_and_changes_nothing() {
    let fx = alias_fx();
    let original = seed_gate_receipt(&fx);
    let output = fx.grh("work", &[], &["status", "--json"]);
    assert_eq!(code(&output), 0, "{}", err(&output));
    serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap();
    assert_eq!(fx.receipt(), original);
}

#[test]
fn installing_by_an_alias_installs_the_part_and_warns_once() {
    let fx = alias_fx();
    let output = fx.grh("work", &[], &["install", "gate", "--yes"]);
    assert_eq!(code(&output), 0, "{}", err(&output));
    let notice = "gate is now greengate: run grh install greengate";
    assert_eq!(err(&output).matches(notice).count(), 1, "{}", err(&output));
    assert!(fx.p("bin/amont").exists());
    assert!(fx.receipt().contains("greengate"));
}

#[test]
fn a_part_run_through_its_alias_warns_before_the_exec_and_keeps_stdout() {
    let fx = alias_fx();
    executable(&fx.p("bin/amont"), FAKE);
    let by_alias = fx.grh("work", &[], &["gate", "--version"]);
    let by_name = fx.grh("work", &[], &["greengate", "--version"]);
    assert_eq!(code(&by_alias), 0, "{}", err(&by_alias));
    assert!(!by_alias.stdout.is_empty());
    assert_eq!(by_alias.stdout, by_name.stdout);
    let notice = "gate is now greengate: run grh greengate";
    assert_eq!(err(&by_alias).matches(notice).count(), 1);
    assert!(!err(&by_name).contains(notice));
}

#[test]
fn audit_rows_with_the_gate_installed_name_the_parts_by_the_catalog() {
    let fx = Fx::new();
    use_shipped_catalog(&fx);
    git_init(&fx);
    executable(&fx.p("bin/amont"), FAKE);
    put(&fx.p("fstate/amont.repo"), b"");
    let path = format!("{}:/usr/bin:/bin", fx.p("bin").display());
    let output = fx.grh("work", &[("PATH", &path)], &["audit", "--json"]);
    assert_eq!(code(&output), 0, "{}", err(&output));
    let v: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let rows: Vec<(String, String)> = v["parts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| {
            (
                r["part"].as_str().unwrap().to_string(),
                r["verdict"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    let names: Vec<&str> = rows.iter().map(|r| r.0.as_str()).collect();
    assert_eq!(
        names,
        [
            "greengate",
            "greenpiste",
            "decisions",
            "conform",
            "router",
            "attest",
            "workflow",
            "design"
        ]
    );
    assert_eq!(rows[0].1, "already here", "{rows:?}");
    assert_eq!(v["install"], "grh install greenpiste");
}

#[test]
fn audit_reads_the_gate_state_from_the_catalog_query() {
    // The same installed fake amont, but the catalog's repo-level status now
    // reads a JSON path the fake never prints: audit must follow the
    // catalog, so the gate is no longer "already here".
    let fx = Fx::new();
    let catalog = include_str!("../catalog.toml").replacen(
        "[\"installed\", \"state\"]",
        "[\"nowhere\", \"state\"]",
        1,
    );
    put(&fx.p("catalog.toml"), catalog.as_bytes());
    git_init(&fx);
    executable(&fx.p("bin/amont"), FAKE);
    put(&fx.p("fstate/amont.repo"), b"");
    let path = format!("{}:/usr/bin:/bin", fx.p("bin").display());
    let output = fx.grh("work", &[("PATH", &path)], &["audit", "--json"]);
    assert_eq!(code(&output), 0, "{}", err(&output));
    let v: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_ne!(v["parts"][0]["verdict"], "already here", "{v}");
}

#[test]
fn a_pinned_part_installs_from_a_fixture_release_beside_gh_catalog() {
    let fx = Fx::new();
    git_init(&fx);
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/pinned");
    let pinned_copy = fs::read(fixture.join("manifests/pinned-demo.toml")).unwrap();
    put(
        &fx.p("pin/catalog.toml"),
        &fs::read(fixture.join("catalog.toml")).unwrap(),
    );
    put(&fx.p("pin/manifests/pinned-demo.toml"), &pinned_copy);
    put(
        &fx.p("rel/fixture/pinned-demo/releases/download/v1.0.0/greenharness.toml"),
        &pinned_copy,
    );
    let catalog = fx.p("pin/catalog.toml").to_string_lossy().into_owned();
    let envs = [("GRH_CATALOG", catalog.as_str())];
    let output = fx.grh(
        "work",
        &envs,
        &["install", "pinned-demo", "--repo", "--yes"],
    );
    assert_eq!(code(&output), 0, "{}", err(&output));
    assert_eq!(
        fs::read_to_string(fx.p("work/.github/workflows/pinned-demo.yml")).unwrap(),
        "name: pinned-demo\n"
    );

    fs::write(fx.p("pin/manifests/pinned-demo.toml"), b"[extension]\n").unwrap();
    let output = fx.grh("work", &envs, &["status"]);
    assert_eq!(code(&output), 1, "{}", err(&output));
    assert!(err(&output).contains("pinned-demo"), "{}", err(&output));
}
