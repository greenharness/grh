//! The extension manifest, `greenharness.toml`: at the root of an extension
//! directory, and the type of every `[[part]]` of the catalog. Unknown keys
//! are refused, and so is a `harness` requirement grh does not satisfy. The
//! format is described in docs/extension-manifest.md.

use std::collections::BTreeMap;
use std::path::{Component, Path};

use serde::Deserialize;

use crate::exit::Exit;

/// The harness version this grh answers to in `[extension] harness`.
pub const HARNESS_VERSION: (u64, u64, u64) = (0, 2, 0);

/// The last harness version that knew only the v1 sections and keys.
const V1_HARNESS_VERSION: (u64, u64, u64) = (0, 1, 0);

pub const FILE_NAME: &str = "greenharness.toml";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub extension: Extension,
    pub tool: Option<Tool>,
    #[serde(default)]
    pub repo_file: Vec<RepoFile>,
    pub bundle: Option<Bundle>,
    #[serde(default)]
    pub skill: Vec<Entry>,
    #[serde(default)]
    pub agent: Vec<Entry>,
    #[serde(default)]
    pub mcp: Vec<Mcp>,
    #[serde(default)]
    pub session_context: Vec<Command>,
    #[serde(default)]
    pub doctor: Vec<Command>,
    pub conform: Option<Conform>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Extension {
    pub name: String,
    pub version: String,
    /// A version requirement on grh's harness: `>=0.2`, `^0.2`, `0.2`.
    pub harness: Option<String>,
    pub summary: Option<String>,
    #[serde(default)]
    pub needs: Vec<String>,
    #[serde(default)]
    pub enhances: Vec<String>,
    #[serde(default)]
    pub aliases: Vec<String>,
    /// Printed, never run: what to do in a repository by hand.
    pub hint: Option<String>,
    /// `false` until the part's manifest is released; absent means published.
    pub published: Option<bool>,
}

/// The binary a part installs, where its release lives, and the commands of
/// its own integration.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tool {
    pub bin: Option<String>,
    pub release: Option<Release>,
    pub machine: Option<ToolLevel>,
    pub repo_level: Option<ToolLevel>,
    pub keep_binary: Option<KeepBinary>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Release {
    /// `owner/name` on GitHub, where the release lives.
    pub repo: String,
    pub version: String,
    /// Target triple → sha256 of `<bin>-<version>-<target>.tar.gz`.
    #[serde(default)]
    pub sha256: BTreeMap<String, String>,
}

/// A tool's own commands for one level, and the query that reads it back.
/// `{bin}` in `install` and `uninstall` is the absolute path of the binary.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolLevel {
    pub install: Vec<String>,
    pub uninstall: Vec<String>,
    /// Host commands that must be on PATH for this level to be placed.
    #[serde(default)]
    pub requires: Vec<String>,
    pub status: Option<Status>,
}

/// Arguments to the part's binary, and how its answer reads.
#[derive(Debug, Deserialize)]
#[serde(try_from = "RawStatus")]
pub struct Status {
    pub args: Vec<String>,
    pub reading: Reading,
}

/// How a status query's answer reads.
#[derive(Debug, Clone)]
pub enum Reading {
    /// The exit code says present or absent.
    Exit(ExitPair),
    /// The state word sits at this path in the JSON the query prints.
    Json(Vec<String>),
}

/// A status as written: exactly one of `exit` and `json` becomes a `Reading`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawStatus {
    args: Vec<String>,
    exit: Option<ExitPair>,
    json: Option<Vec<String>>,
}

impl TryFrom<RawStatus> for Status {
    type Error = String;

    fn try_from(raw: RawStatus) -> Result<Status, String> {
        let reading = match (raw.exit, raw.json) {
            (Some(pair), None) => Reading::Exit(pair),
            (None, Some(path)) if !path.is_empty() => Reading::Json(path),
            _ => {
                return Err("[tool.*.status] needs exactly one of exit = { present, absent } or a non-empty json path".into())
            }
        };
        Ok(Status {
            args: raw.args,
            reading,
        })
    }
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExitPair {
    pub present: i32,
    pub absent: i32,
}

/// When a placed binary must stay: the tool's own answer about a place that
/// bakes its path. `args` run the binary; `state` is the JSON path of the
/// state word, `absent` the word that frees the binary, `shims` the path of
/// the array of baked copies and `binary` the key of each one's path.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KeepBinary {
    pub args: Vec<String>,
    pub state: Vec<String>,
    pub absent: String,
    pub shims: Vec<String>,
    pub binary: String,
    /// Printed, after the path, when the binary is removed.
    pub warning: String,
}

/// A file a part places in a repository, in the first of `dirs` that exists.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepoFile {
    pub dirs: Vec<String>,
    pub path: String,
    pub content: String,
}

/// Names taken from grh's pinned skills bundle.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Bundle {
    #[serde(default)]
    pub skills: Vec<String>,
    #[serde(default)]
    pub agents: Vec<String>,
}

/// A skill directory or an agent file, relative to the manifest directory.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub path: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Mcp {
    pub name: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Command {
    pub command: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Conform {
    pub eslint: Option<String>,
}

fn bad(msg: impl std::fmt::Display) -> Exit {
    Exit::Usage(format!("invalid extension manifest: {msg}"))
}

fn is_plain_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
}

/// A relative path that stays inside its root: normal components only.
fn stays_inside(path: &str) -> bool {
    !path.is_empty()
        && Path::new(path)
            .components()
            .all(|c| matches!(c, Component::Normal(_) | Component::CurDir))
}

impl Manifest {
    /// Read `<dir>/greenharness.toml`.
    pub fn load(dir: &Path) -> Result<Manifest, Exit> {
        let file = dir.join(FILE_NAME);
        let text = std::fs::read_to_string(&file)
            .map_err(|e| Exit::Usage(format!("cannot read {}: {e}", file.display())))?;
        Manifest::parse(&text)
    }

    pub fn parse(text: &str) -> Result<Manifest, Exit> {
        let m: Manifest = toml::from_str(text).map_err(bad)?;
        m.check()?;
        Ok(m)
    }

    /// A manifest for skills and agents grh already holds, not read from a file.
    pub fn bundled(
        name: String,
        summary: String,
        skills: Vec<String>,
        agents: Vec<String>,
    ) -> Manifest {
        Manifest {
            extension: Extension {
                name,
                version: String::new(),
                harness: None,
                summary: Some(summary),
                needs: Vec::new(),
                enhances: Vec::new(),
                aliases: Vec::new(),
                hint: None,
                published: None,
            },
            tool: None,
            repo_file: Vec::new(),
            bundle: Some(Bundle { skills, agents }),
            skill: Vec::new(),
            agent: Vec::new(),
            mcp: Vec::new(),
            session_context: Vec::new(),
            doctor: Vec::new(),
            conform: None,
        }
    }

    pub fn name(&self) -> &str {
        &self.extension.name
    }

    pub fn summary(&self) -> &str {
        self.extension.summary.as_deref().unwrap_or("")
    }

    pub fn needs(&self) -> &[String] {
        &self.extension.needs
    }

    pub fn enhances(&self) -> &[String] {
        &self.extension.enhances
    }

    pub fn aliases(&self) -> &[String] {
        &self.extension.aliases
    }

    /// Does a receipt entry recorded under `recorded` belong to this part:
    /// its own name, or an alias it was once written under?
    pub fn answers_to(&self, recorded: &str) -> bool {
        self.name() == recorded || self.extension.aliases.iter().any(|a| a == recorded)
    }

    pub fn hint(&self) -> Option<&str> {
        self.extension.hint.as_deref()
    }

    pub fn is_published(&self) -> bool {
        self.extension.published.unwrap_or(true)
    }

    pub fn bin(&self) -> Option<&str> {
        self.tool.as_ref()?.bin.as_deref()
    }

    pub fn release(&self) -> Option<&Release> {
        self.tool.as_ref()?.release.as_ref()
    }

    pub fn machine(&self) -> Option<&ToolLevel> {
        self.tool.as_ref()?.machine.as_ref()
    }

    pub fn repo_level(&self) -> Option<&ToolLevel> {
        self.tool.as_ref()?.repo_level.as_ref()
    }

    pub fn keep_binary(&self) -> Option<&KeepBinary> {
        self.tool.as_ref()?.keep_binary.as_ref()
    }

    pub fn bundle_skills(&self) -> &[String] {
        self.bundle.as_ref().map_or(&[], |b| &b.skills)
    }

    pub fn bundle_agents(&self) -> &[String] {
        self.bundle.as_ref().map_or(&[], |b| &b.agents)
    }

    /// Does the manifest use a section or key that v1 did not have?
    fn uses_v2(&self) -> bool {
        let e = &self.extension;
        e.summary.is_some()
            || !e.needs.is_empty()
            || !e.enhances.is_empty()
            || !e.aliases.is_empty()
            || e.hint.is_some()
            || e.published.is_some()
            || self.tool.is_some()
            || !self.repo_file.is_empty()
            || self.bundle.is_some()
    }

    /// The checks a parsed manifest must pass; the catalog runs them on every
    /// part too.
    pub fn check(&self) -> Result<(), Exit> {
        let name = &self.extension.name;
        if !is_plain_name(name) {
            return Err(bad(format!("extension name {name:?} is not a plain name")));
        }
        if let Some(req) = &self.extension.harness {
            if !satisfies(req, HARNESS_VERSION)? {
                return Err(bad(format!(
                    "this extension needs harness {req}; grh is {}.{}",
                    HARNESS_VERSION.0, HARNESS_VERSION.1
                )));
            }
        }
        if self.uses_v2() {
            let Some(req) = &self.extension.harness else {
                return Err(bad(
                    "this manifest uses v2 sections or keys, so it must declare harness = \">=0.2\"",
                ));
            };
            if satisfies(req, V1_HARNESS_VERSION)? {
                return Err(bad(format!(
                    "harness {req} is satisfied by 0.1, which does not know the v2 sections and keys used here; declare harness = \">=0.2\""
                )));
            }
        }
        for related in [
            &self.extension.needs,
            &self.extension.enhances,
            &self.extension.aliases,
        ] {
            if let Some(n) = related.iter().find(|n| !is_plain_name(n)) {
                return Err(bad(format!("{n:?} is not a plain part name")));
            }
        }
        for e in self.skill.iter().chain(&self.agent) {
            if !stays_inside(&e.path) {
                return Err(bad(format!(
                    "path {:?} must be relative and stay inside the extension",
                    e.path
                )));
            }
        }
        for m in &self.mcp {
            if m.name.is_empty() || m.command.is_empty() {
                return Err(bad("an [[mcp]] entry needs a name and a command"));
            }
        }
        if let Some(tool) = &self.tool {
            tool.check()?;
        }
        for f in &self.repo_file {
            if f.dirs.is_empty() || f.dirs.iter().any(|d| !stays_inside(d)) {
                return Err(bad(format!(
                    "a [[repo_file]] for {:?} needs relative directories that stay inside the repository",
                    f.path
                )));
            }
            if !stays_inside(&f.path) {
                return Err(bad(format!(
                    "repo_file path {:?} must be relative and stay inside the repository",
                    f.path
                )));
            }
        }
        Ok(())
    }
}

impl Tool {
    fn check(&self) -> Result<(), Exit> {
        if self.bin.as_deref().is_some_and(|b| !is_plain_name(b)) {
            return Err(bad("[tool] bin is not a plain file name"));
        }
        if self.bin.is_none()
            && (self.release.is_some()
                || self.machine.is_some()
                || self.repo_level.is_some()
                || self.keep_binary.is_some())
        {
            return Err(bad(
                "[tool] needs a bin for its release, levels and keep_binary",
            ));
        }
        if let Some(r) = &self.release {
            if r.repo.is_empty() || r.version.is_empty() {
                return Err(bad("[tool.release] needs a repo and a version"));
            }
        }
        for (label, level) in [("machine", &self.machine), ("repo_level", &self.repo_level)] {
            let Some(level) = level else { continue };
            if level.install.is_empty() || level.uninstall.is_empty() {
                return Err(bad(format!(
                    "[tool.{label}] needs a non-empty install and uninstall"
                )));
            }
            if level.requires.iter().any(|c| !is_plain_name(c)) {
                return Err(bad(format!("[tool.{label}] requires names plain commands")));
            }
        }
        if let Some(k) = &self.keep_binary {
            if k.args.is_empty()
                || k.state.is_empty()
                || k.shims.is_empty()
                || k.absent.is_empty()
                || k.binary.is_empty()
            {
                return Err(bad(
                    "[tool.keep_binary] needs args, state, absent, shims and binary",
                ));
            }
        }
        Ok(())
    }
}

fn parse_version(s: &str) -> Result<(u64, u64, u64), Exit> {
    let mut parts = s.trim().split('.');
    let mut next = |required: bool| -> Result<u64, Exit> {
        match parts.next() {
            Some(p) => p
                .parse()
                .map_err(|_| bad(format!("{s:?} is not a version"))),
            None if required => Err(bad(format!("{s:?} is not a version"))),
            None => Ok(0),
        }
    };
    let v = (next(true)?, next(false)?, next(false)?);
    if parts.next().is_some() {
        return Err(bad(format!("{s:?} is not a version")));
    }
    Ok(v)
}

/// Does harness version `have` satisfy `req`? Clauses are comma-separated;
/// operators are `>=`, `>`, `<=`, `<`, `=`, `^`, `~`; a bare version means `^`.
fn satisfies(req: &str, have: (u64, u64, u64)) -> Result<bool, Exit> {
    for clause in req.split(',') {
        let clause = clause.trim();
        let (op, rest) = ["<=", ">=", "<", ">", "=", "^", "~"]
            .iter()
            .find_map(|op| clause.strip_prefix(op).map(|r| (*op, r)))
            .unwrap_or(("^", clause));
        let want = parse_version(rest)?;
        let ok = match op {
            ">=" => have >= want,
            ">" => have > want,
            "<=" => have <= want,
            "<" => have < want,
            "=" => have == want,
            "~" => have >= want && (have.0, have.1) == (want.0, want.1),
            _ => {
                have >= want
                    && if want.0 > 0 {
                        have.0 == want.0
                    } else {
                        (have.0, have.1) == (want.0, want.1)
                    }
            }
        };
        if !ok {
            return Ok(false);
        }
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: &str = "[extension]\nname = \"duro\"\nversion = \"1.0.0\"\n";
    const DURO_V1: &str = include_str!("../tests/fixtures/duro-greenharness.toml");

    const V2: &str = "[extension]\nname = \"tool\"\nversion = \"1.0.0\"\nharness = \">=0.2\"\n\
         summary = \"a tool\"\nneeds = [\"other\"]\naliases = [\"old\"]\n\
         [tool]\nbin = \"tool\"\n\
         [tool.release]\nrepo = \"o/tool\"\nversion = \"1.0.0\"\n\
         [tool.release.sha256]\nx86_64-unknown-linux-gnu = \"ab\"\n\
         [tool.repo_level]\ninstall = [\"{bin}\", \"init\"]\nuninstall = [\"{bin}\", \"uninstall\"]\nrequires = [\"claude\"]\n\
         [tool.repo_level.status]\nargs = [\"list\", \"--json\"]\njson = [\"installed\", \"state\"]\n\
         [[repo_file]]\ndirs = [\".forgejo\", \".github\"]\npath = \"workflows/a.yml\"\ncontent = \"x\\n\"\n";

    #[test]
    fn duros_v1_manifest_still_loads() {
        let m = Manifest::parse(DURO_V1).unwrap();
        assert_eq!(m.name(), "design");
        assert!(m.tool.is_none() && !m.mcp.is_empty());
    }

    #[test]
    fn a_v2_manifest_with_release_and_repo_level_loads() {
        let m = Manifest::parse(V2).unwrap();
        assert_eq!(m.bin(), Some("tool"));
        assert_eq!(m.release().unwrap().version, "1.0.0");
        let level = m.repo_level().unwrap();
        assert_eq!(level.requires, ["claude"]);
        let Reading::Json(path) = &level.status.as_ref().unwrap().reading else {
            panic!("the status reads JSON");
        };
        assert_eq!(path, &["installed", "state"]);
        assert_eq!(m.repo_file[0].dirs, [".forgejo", ".github"]);
    }

    #[test]
    fn a_v2_section_needs_a_harness_that_0_1_does_not_satisfy() {
        let v2_body = V2.replace("harness = \">=0.2\"\n", "");
        assert!(Manifest::parse(&v2_body).is_err());
        let lax = V2.replace(">=0.2", ">=0.1");
        assert!(Manifest::parse(&lax).is_err());
        let caret = V2.replace(">=0.2", "^0.1");
        assert!(Manifest::parse(&caret).is_err());
        assert!(Manifest::parse(&V2.replace(">=0.2", "^0.2")).is_ok());
    }

    #[test]
    fn a_full_manifest_parses() {
        let m = Manifest::parse(&format!(
            "{BASE}harness = \">=0.1\"\n[[skill]]\npath = \"skills/a\"\n[[agent]]\npath = \"agents/b.md\"\n\
             [[mcp]]\nname = \"duro\"\ncommand = \"duro\"\nargs = [\"mcp\"]\n\
             [[session_context]]\ncommand = \"duro ctx\"\n[[doctor]]\ncommand = \"duro doctor\"\n\
             [conform]\neslint = \"duro lint\"\n"
        ))
        .unwrap();
        assert_eq!(m.mcp[0].args, ["mcp"]);
        assert_eq!(m.conform.unwrap().eslint.as_deref(), Some("duro lint"));
    }

    #[test]
    fn unknown_keys_are_refused() {
        assert!(Manifest::parse(&format!("{BASE}colour = 1\n")).is_err());
        assert!(Manifest::parse(&format!("{BASE}[[skill]]\npath = \"a\"\nx = 1\n")).is_err());
        assert!(Manifest::parse(&format!("{V2}[tool.keep]\nx = 1\n")).is_err());
    }

    #[test]
    fn harness_requirements() {
        let now = HARNESS_VERSION;
        assert!(satisfies(">=0.1", now).unwrap());
        assert!(!satisfies("^0.1", now).unwrap());
        assert!(!satisfies("0.1", now).unwrap());
        assert!(satisfies("^0.2", now).unwrap());
        assert!(satisfies(">=0.1, <1", now).unwrap());
        assert!(!satisfies(">=9", now).unwrap());
        assert!(satisfies("nope", now).is_err());
    }

    #[test]
    fn v1_manifests_with_a_harness_grh_no_longer_matches_are_refused() {
        assert!(Manifest::parse(&format!("{BASE}harness = \"^0.1\"\n")).is_err());
        assert!(Manifest::parse(&format!("{BASE}harness = \">=0.1\"\n")).is_ok());
    }

    #[test]
    fn escaping_paths_are_refused() {
        assert!(Manifest::parse(&format!("{BASE}[[skill]]\npath = \"../x\"\n")).is_err());
        assert!(Manifest::parse(&format!("{BASE}[[agent]]\npath = \"/x\"\n")).is_err());
        assert!(Manifest::parse(&V2.replace("workflows/a.yml", "../a.yml")).is_err());
        assert!(Manifest::parse(&V2.replace("workflows/a.yml", "/a.yml")).is_err());
        assert!(Manifest::parse(&V2.replace("\".forgejo\"", "\"..\"")).is_err());
    }

    #[test]
    fn a_status_needs_exactly_one_reading() {
        let both = V2.replace(
            "json = [\"installed\", \"state\"]",
            "json = [\"installed\", \"state\"]\nexit = { present = 0, absent = 1 }",
        );
        assert!(Manifest::parse(&both).is_err());
        let neither = V2.replace("json = [\"installed\", \"state\"]", "");
        assert!(Manifest::parse(&neither).is_err());
        let exit = V2.replace(
            "json = [\"installed\", \"state\"]",
            "exit = { present = 0, absent = 1 }",
        );
        assert!(Manifest::parse(&exit).is_ok());
    }

    #[test]
    fn requires_and_keep_binary_are_validated() {
        assert!(Manifest::parse(&V2.replace("[\"claude\"]", "[\"a b\"]")).is_err());
        let keep = format!(
            "{V2}[tool.keep_binary]\nargs = [\"list\"]\nstate = [\"t\", \"state\"]\nabsent = \"absent\"\n\
             shims = [\"t\", \"shims\"]\nbinary = \"binary\"\nwarning = \"w\"\n"
        );
        assert!(Manifest::parse(&keep).is_ok());
        assert!(Manifest::parse(&keep.replace("args = [\"list\"]", "args = []")).is_err());
    }
}
