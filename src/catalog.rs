//! The parts grh knows, read from `catalog.toml` compiled into the binary.

use std::collections::{BTreeMap, BTreeSet};

use serde::Deserialize;

use crate::exit::Exit;

const CATALOG: &str = include_str!("../catalog.toml");

#[derive(Debug, Deserialize)]
pub struct Catalog {
    pub skills_repo: String,
    pub skills_version: String,
    pub skills_sha256: String,
    pub profiles: BTreeMap<String, Vec<String>>,
    #[serde(rename = "part")]
    pub parts: Vec<Part>,
}

#[derive(Debug, Deserialize)]
pub struct Part {
    pub name: String,
    pub summary: String,
    pub needs: Vec<String>,
    /// The binary the part installs, if it has one.
    pub bin: Option<String>,
    /// `owner/name` on GitHub, where the release lives.
    pub repo: Option<String>,
    pub version: Option<String>,
    /// Target triple → sha256 of `<bin>-<version>-<target>.tar.gz`.
    #[serde(default)]
    pub sha256: BTreeMap<String, String>,
    pub machine: Option<Integration>,
    pub repo_level: Option<Integration>,
    #[serde(default)]
    pub skills: Vec<String>,
    #[serde(default)]
    pub agents: Vec<String>,
    /// Printed, never run: what to do in a repository by hand.
    pub hint: Option<String>,
    /// A CI workflow grh writes at repo level (attest).
    pub workflow: Option<String>,
    /// Set for an extension; `unpublished` until its manifest is released.
    pub extension: Option<String>,
}

/// A tool's own commands for one level, and the query that reads it back.
#[derive(Debug, Deserialize)]
pub struct Integration {
    pub install: Vec<String>,
    pub uninstall: Vec<String>,
    pub query: String,
}

/// What a name on the command line asked for.
#[derive(Debug, PartialEq, Eq)]
pub enum Unknown {
    Name(String),
}

impl Catalog {
    /// The compiled-in catalog, or the file `GRH_CATALOG` names (a test hook
    /// so the integration tests can pin their own fixture tarballs). A bad
    /// file is a failure, never a panic.
    pub fn load() -> Result<Catalog, Exit> {
        if let Some(path) = std::env::var_os("GRH_CATALOG").filter(|p| !p.is_empty()) {
            let text = std::fs::read_to_string(&path).map_err(|e| {
                Exit::Failed(format!(
                    "GRH_CATALOG {} is not readable: {e}",
                    path.to_string_lossy()
                ))
            })?;
            return toml::from_str(&text).map_err(|e| {
                Exit::Failed(format!(
                    "GRH_CATALOG {} is not a valid catalog: {e}",
                    path.to_string_lossy()
                ))
            });
        }
        // Compiled in and checked by the tests below.
        toml::from_str(CATALOG).map_err(|e| Exit::Failed(format!("catalog.toml is invalid: {e}")))
    }

    pub fn part(&self, name: &str) -> Option<&Part> {
        self.parts.iter().find(|p| p.name == name)
    }

    /// Parts and profiles named on the command line, expanded, with every
    /// dependency added, in install order (a part after what it needs).
    pub fn resolve(&self, names: &[String]) -> Result<Vec<&Part>, Unknown> {
        let mut wanted = BTreeSet::new();
        for name in names {
            if let Some(profile) = self.profiles.get(name) {
                wanted.extend(profile.iter().cloned());
            } else if self.part(name).is_some() {
                wanted.insert(name.clone());
            } else {
                return Err(Unknown::Name(name.clone()));
            }
        }
        let mut order = Vec::new();
        let mut seen = BTreeSet::new();
        for name in &wanted {
            self.visit(name, &mut seen, &mut order);
        }
        Ok(order)
    }

    fn visit<'a>(&'a self, name: &str, seen: &mut BTreeSet<String>, order: &mut Vec<&'a Part>) {
        if !seen.insert(name.to_string()) {
            return;
        }
        let part = self.part(name).expect("needs names a known part");
        for need in &part.needs {
            self.visit(need, seen, order);
        }
        order.push(part);
    }

    /// The installed parts that need `name`.
    pub fn dependents<'a>(&'a self, name: &str, installed: &BTreeSet<String>) -> Vec<&'a str> {
        self.parts
            .iter()
            .filter(|p| installed.contains(&p.name) && p.needs.iter().any(|n| n == name))
            .map(|p| p.name.as_str())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(parts: &[&Part]) -> Vec<String> {
        parts.iter().map(|p| p.name.clone()).collect()
    }

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn every_need_names_a_part_and_no_profile_collides_with_one() {
        let c = Catalog::load().unwrap();
        for p in &c.parts {
            for n in &p.needs {
                assert!(c.part(n).is_some(), "{} needs unknown {n}", p.name);
            }
        }
        for (name, members) in &c.profiles {
            assert!(c.part(name).is_none(), "profile {name} shadows a part");
            for m in members {
                assert!(c.part(m).is_some(), "profile {name} names unknown {m}");
            }
        }
    }

    #[test]
    fn a_binary_part_pins_every_unix_target() {
        let c = Catalog::load().unwrap();
        for p in c.parts.iter().filter(|p| p.bin.is_some()) {
            for t in [
                "aarch64-apple-darwin",
                "x86_64-apple-darwin",
                "aarch64-unknown-linux-gnu",
                "x86_64-unknown-linux-gnu",
            ] {
                let sha = p.sha256.get(t).map(String::as_str).unwrap_or("");
                assert_eq!(sha.len(), 64, "{} lacks a sha256 for {t}", p.name);
            }
            assert!(p.repo.is_some() && p.version.is_some(), "{}", p.name);
        }
    }

    #[test]
    fn profiles_expand_to_what_the_site_lists() {
        let c = Catalog::load().unwrap();
        let solo = names(&c.resolve(&args(&["solo"])).unwrap());
        assert_eq!(solo, ["gate", "guardrails"]);
        let team = names(&c.resolve(&args(&["team"])).unwrap());
        assert_eq!(team, ["decisions", "conform", "gate", "guardrails"]);
        let full = names(&c.resolve(&args(&["full"])).unwrap());
        assert_eq!(full.len(), 5);
        assert!(full.contains(&"router".to_string()));
    }

    #[test]
    fn dependencies_are_added_before_what_needs_them() {
        let c = Catalog::load().unwrap();
        assert_eq!(
            names(&c.resolve(&args(&["conform"])).unwrap()),
            ["decisions", "conform"]
        );
        assert_eq!(
            names(&c.resolve(&args(&["attest"])).unwrap()),
            ["gate", "attest"]
        );
        assert_eq!(
            names(&c.resolve(&args(&["workflow"])).unwrap()),
            ["gate", "guardrails", "workflow"]
        );
    }

    #[test]
    fn an_unknown_name_is_refused() {
        let c = Catalog::load().unwrap();
        assert_eq!(
            c.resolve(&args(&["gate", "nope"])).unwrap_err(),
            Unknown::Name("nope".into())
        );
    }

    #[test]
    fn dependents_are_the_installed_parts_that_need_it() {
        let c = Catalog::load().unwrap();
        let installed: BTreeSet<String> = ["decisions", "conform", "gate"].map(String::from).into();
        assert_eq!(c.dependents("decisions", &installed), ["conform"]);
        assert!(c.dependents("gate", &installed).is_empty());
    }
}
