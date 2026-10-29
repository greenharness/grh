//! The extension manifest v1: `greenharness.toml` at the root of an extension
//! directory. Unknown keys are refused, and so is a `harness` requirement grh
//! does not satisfy. The format is described in docs/extension-manifest.md.

use std::path::{Component, Path};

use serde::Deserialize;

use crate::exit::Exit;

/// The harness version this grh answers to in `[extension] harness`.
pub const HARNESS_VERSION: (u64, u64, u64) = (0, 1, 0);

pub const FILE_NAME: &str = "greenharness.toml";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub extension: Extension,
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
    /// A version requirement on grh's harness: `>=0.1`, `^0.1`, `0.1`.
    pub harness: Option<String>,
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

    fn check(&self) -> Result<(), Exit> {
        let name = &self.extension.name;
        if name.is_empty()
            || !name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
        {
            return Err(bad(format!("extension name {name:?} is not a plain name")));
        }
        if let Some(req) = &self.extension.harness {
            if !satisfies(req)? {
                return Err(bad(format!(
                    "this extension needs harness {req}; grh is {}.{}",
                    HARNESS_VERSION.0, HARNESS_VERSION.1
                )));
            }
        }
        for e in self.skill.iter().chain(&self.agent) {
            let p = Path::new(&e.path);
            if e.path.is_empty()
                || p.components()
                    .any(|c| !matches!(c, Component::Normal(_) | Component::CurDir))
            {
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

/// Does grh's harness version satisfy `req`? Clauses are comma-separated;
/// operators are `>=`, `>`, `<=`, `<`, `=`, `^`, `~`; a bare version means `^`.
fn satisfies(req: &str) -> Result<bool, Exit> {
    let have = HARNESS_VERSION;
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
    }

    #[test]
    fn harness_requirements() {
        assert!(satisfies(">=0.1").unwrap());
        assert!(satisfies("^0.1").unwrap());
        assert!(satisfies("0.1").unwrap());
        assert!(satisfies(">=0.1, <1").unwrap());
        assert!(!satisfies(">=9").unwrap());
        assert!(!satisfies("^0.2").unwrap());
        assert!(satisfies("nope").is_err());
    }

    #[test]
    fn escaping_paths_are_refused() {
        assert!(Manifest::parse(&format!("{BASE}[[skill]]\npath = \"../x\"\n")).is_err());
        assert!(Manifest::parse(&format!("{BASE}[[agent]]\npath = \"/x\"\n")).is_err());
    }
}
