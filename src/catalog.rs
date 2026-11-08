//! The parts grh knows, read from `catalog.toml` compiled into the binary.
//! Each `[[part]]` is a greenharness.toml v2 manifest.

use std::collections::{BTreeMap, BTreeSet};

use serde::Deserialize;

use crate::exit::Exit;
use crate::manifest::Manifest;
use crate::ops::text;

const CATALOG: &str = include_str!("../catalog.toml");

#[derive(Debug, Deserialize)]
pub struct Catalog {
    pub skills_repo: String,
    pub skills_version: String,
    pub skills_sha256: String,
    pub profiles: BTreeMap<String, Vec<String>>,
    /// ADR-0064's list of the parts: every edge points to an earlier entry.
    pub order: Vec<String>,
    #[serde(rename = "part")]
    pub parts: Vec<Part>,
}

/// A catalog entry is a manifest like any extension's.
pub type Part = Manifest;

/// What a name on the command line asked for.
#[derive(Debug, PartialEq, Eq)]
pub enum Unknown {
    Name(String),
}

fn invalid(msg: impl std::fmt::Display) -> Exit {
    Exit::Failed(format!("the catalog is invalid: {msg}"))
}

impl Catalog {
    /// The compiled-in catalog, or the file `GRH_CATALOG` names (a test hook
    /// so the integration tests can pin their own fixture tarballs). A bad
    /// file is a failure, never a panic.
    pub fn load() -> Result<Catalog, Exit> {
        if let Some(path) = std::env::var_os("GRH_CATALOG").filter(|p| !p.is_empty()) {
            let source = std::fs::read_to_string(&path).map_err(|e| {
                Exit::Failed(format!(
                    "GRH_CATALOG {} is not readable: {e}",
                    path.to_string_lossy()
                ))
            })?;
            return Catalog::parse(&source).map_err(|e| {
                Exit::Failed(format!(
                    "GRH_CATALOG {} is not a valid catalog: {}",
                    path.to_string_lossy(),
                    text(&e)
                ))
            });
        }
        Catalog::parse(CATALOG)
            .map_err(|e| Exit::Failed(format!("catalog.toml is invalid: {}", text(&e))))
    }

    /// Parse a catalog and refuse one `resolve` could not walk.
    pub fn parse(source: &str) -> Result<Catalog, Exit> {
        let catalog: Catalog = toml::from_str(source).map_err(|e| Exit::Failed(e.to_string()))?;
        catalog.checked()
    }

    /// Every part is a valid manifest; names and aliases are unique; every
    /// `needs`, `enhances` and profile member names a part; no cycle runs
    /// through `needs` and `enhances` together; every part is in `order` and
    /// every edge points to an earlier entry. So `resolve` can walk the
    /// graph without a lookup that could fail.
    fn checked(self) -> Result<Catalog, Exit> {
        for part in &self.parts {
            part.check()
                .map_err(|e| invalid(format!("part {}: {}", part.name(), text(&e))))?;
        }
        let mut taken: BTreeMap<&str, &str> = BTreeMap::new();
        for part in &self.parts {
            if let Some(owner) = taken.insert(part.name(), part.name()) {
                return Err(invalid(format!(
                    "the name {} is taken twice ({owner})",
                    part.name()
                )));
            }
        }
        for part in &self.parts {
            for alias in part.aliases() {
                if let Some(owner) = taken.insert(alias, part.name()) {
                    return Err(invalid(format!(
                        "the alias {alias} of {} collides with {owner}",
                        part.name()
                    )));
                }
            }
        }
        for part in &self.parts {
            for (kind, edges) in [("needs", part.needs()), ("enhances", part.enhances())] {
                if let Some(edge) = edges.iter().find(|n| self.part(n).is_none()) {
                    return Err(invalid(format!(
                        "{} {kind} unknown part {edge}",
                        part.name()
                    )));
                }
            }
        }
        for (profile, members) in &self.profiles {
            if let Some(member) = members.iter().find(|m| self.part(m).is_none()) {
                return Err(invalid(format!(
                    "profile {profile} names unknown part {member}"
                )));
            }
        }
        self.check_order()?;
        self.check_acyclic()?;
        self.check_edges_point_down()?;
        Ok(self)
    }

    fn check_order(&self) -> Result<(), Exit> {
        let mut seen = BTreeSet::new();
        for name in &self.order {
            if self.parts.iter().all(|p| p.name() != name) {
                return Err(invalid(format!("order lists {name}, which is no part")));
            }
            if !seen.insert(name.as_str()) {
                return Err(invalid(format!("order lists {name} twice")));
            }
        }
        if let Some(part) = self.parts.iter().find(|p| !seen.contains(p.name())) {
            return Err(invalid(format!("{} is missing from order", part.name())));
        }
        Ok(())
    }

    /// The parts `name` needs or enhances, by catalog name.
    fn edges<'a>(&'a self, name: &str) -> Vec<&'a str> {
        self.part(name)
            .into_iter()
            .flat_map(|p| p.needs().iter().chain(p.enhances()))
            .filter_map(|n| self.part(n))
            .map(|p| p.name())
            .collect()
    }

    fn check_acyclic(&self) -> Result<(), Exit> {
        fn walk<'a>(
            cat: &'a Catalog,
            name: &'a str,
            path: &mut Vec<&'a str>,
            done: &mut BTreeSet<&'a str>,
        ) -> Result<(), Exit> {
            if let Some(at) = path.iter().position(|n| *n == name) {
                let mut cycle: Vec<&str> = path[at..].to_vec();
                cycle.push(name);
                return Err(invalid(format!(
                    "a cycle of needs and enhances: {}",
                    cycle.join(" -> ")
                )));
            }
            if !done.insert(name) {
                return Ok(());
            }
            path.push(name);
            for next in cat.edges(name) {
                walk(cat, next, path, done)?;
            }
            path.pop();
            Ok(())
        }
        let mut done = BTreeSet::new();
        for part in &self.parts {
            walk(self, part.name(), &mut Vec::new(), &mut done)?;
        }
        Ok(())
    }

    fn check_edges_point_down(&self) -> Result<(), Exit> {
        let rank = |name: &str| self.order.iter().position(|n| n == name);
        for part in &self.parts {
            for edge in self.edges(part.name()) {
                if rank(edge) >= rank(part.name()) {
                    return Err(invalid(format!(
                        "{} points up order to {edge}: an edge names an earlier part",
                        part.name()
                    )));
                }
            }
        }
        Ok(())
    }

    /// The part a name or an alias asks for.
    pub fn part(&self, name: &str) -> Option<&Part> {
        self.parts.iter().find(|p| p.name() == name).or_else(|| {
            self.parts
                .iter()
                .find(|p| p.aliases().iter().any(|a| a == name))
        })
    }

    /// The part's own name when `name` is one of its aliases.
    pub fn name_of_alias(&self, name: &str) -> Option<String> {
        self.parts
            .iter()
            .find(|p| p.aliases().iter().any(|a| a == name))
            .map(|p| p.name().to_string())
    }

    /// Parts and profiles named on the command line, expanded, with every
    /// dependency added, in install order (a part after what it needs).
    /// `enhances` are never added.
    pub fn resolve(&self, names: &[String]) -> Result<Vec<&Part>, Unknown> {
        let mut wanted = BTreeSet::new();
        for name in names {
            if let Some(profile) = self.profiles.get(name) {
                wanted.extend(
                    profile
                        .iter()
                        .filter_map(|m| self.part(m))
                        .map(|p| p.name()),
                );
            } else if let Some(part) = self.part(name) {
                wanted.insert(part.name());
            } else {
                return Err(Unknown::Name(name.clone()));
            }
        }
        let mut order = Vec::new();
        let mut seen = BTreeSet::new();
        for name in wanted {
            self.visit(name, &mut seen, &mut order);
        }
        Ok(order)
    }

    fn visit<'a>(&'a self, name: &str, seen: &mut BTreeSet<String>, order: &mut Vec<&'a Part>) {
        let part = self.part(name).expect("needs names a known part");
        if !seen.insert(part.name().to_string()) {
            return;
        }
        for need in part.needs() {
            self.visit(need, seen, order);
        }
        order.push(part);
    }

    /// The installed parts that need `name`.
    pub fn dependents<'a>(&'a self, name: &str, installed: &BTreeSet<String>) -> Vec<&'a str> {
        let Some(target) = self.part(name).map(Part::name) else {
            return Vec::new();
        };
        self.parts
            .iter()
            .filter(|p| {
                installed.contains(p.name())
                    && p.needs()
                        .iter()
                        .any(|n| self.part(n).is_some_and(|x| x.name() == target))
            })
            .map(|p| p.name())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(parts: &[&Part]) -> Vec<String> {
        parts.iter().map(|p| p.name().to_string()).collect()
    }

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    /// `(name, needs, enhances, aliases)` of one bare manifest.
    type Bare<'a> = (&'a str, &'a [&'a str], &'a [&'a str], &'a [&'a str]);

    /// A catalog of bare manifests.
    fn fixture(order: &[&str], parts: &[Bare]) -> String {
        let list = |v: &[&str]| {
            format!(
                "[{}]",
                v.iter()
                    .map(|s| format!("\"{s}\""))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };
        let mut out = format!(
            "skills_repo = \"o/s\"\nskills_version = \"1\"\nskills_sha256 = \"x\"\norder = {}\n[profiles]\n",
            list(order)
        );
        for (name, needs, enhances, aliases) in parts {
            out.push_str(&format!(
                "[[part]]\n[part.extension]\nname = \"{name}\"\nversion = \"1\"\nharness = \">=0.2\"\n\
                 needs = {}\nenhances = {}\naliases = {}\n",
                list(needs),
                list(enhances),
                list(aliases)
            ));
        }
        out
    }

    fn refused(source: &str, mentions: &[&str]) {
        let message = match Catalog::parse(source) {
            Ok(_) => panic!("the catalog was accepted:\n{source}"),
            Err(e) => text(&e),
        };
        for word in mentions {
            assert!(message.contains(word), "{message:?} does not name {word}");
        }
    }

    #[test]
    fn the_shipped_catalog_passes_checked_and_every_edge_points_down_order() {
        let c = Catalog::load().unwrap();
        let rank = |n: &str| c.order.iter().position(|o| o == n).unwrap();
        for p in &c.parts {
            for edge in p.needs().iter().chain(p.enhances()) {
                let target = c.part(edge).unwrap();
                assert!(
                    rank(target.name()) < rank(p.name()),
                    "{} -> {edge}",
                    p.name()
                );
            }
        }
        assert_eq!(c.part("gate").unwrap().name(), "greengate");
        assert_eq!(c.part("guardrails").unwrap().name(), "greenpiste");
    }

    #[test]
    fn every_need_names_a_part_and_no_profile_collides_with_one() {
        let c = Catalog::load().unwrap();
        for p in &c.parts {
            for n in p.needs() {
                assert!(c.part(n).is_some(), "{} needs unknown {n}", p.name());
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
        for p in c.parts.iter().filter(|p| p.bin().is_some()) {
            let release = p
                .release()
                .unwrap_or_else(|| panic!("{} has no release", p.name()));
            for t in [
                "aarch64-apple-darwin",
                "x86_64-apple-darwin",
                "aarch64-unknown-linux-gnu",
                "x86_64-unknown-linux-gnu",
            ] {
                let sha = release.sha256.get(t).map(String::as_str).unwrap_or("");
                assert_eq!(sha.len(), 64, "{} lacks a sha256 for {t}", p.name());
            }
        }
    }

    #[test]
    fn profiles_expand_to_what_the_site_lists() {
        let c = Catalog::load().unwrap();
        let solo = names(&c.resolve(&args(&["solo"])).unwrap());
        assert_eq!(solo, ["greengate", "greenpiste"]);
        let team = names(&c.resolve(&args(&["team"])).unwrap());
        assert_eq!(team, ["decisions", "conform", "greengate", "greenpiste"]);
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
            ["greengate", "attest"]
        );
        assert_eq!(
            names(&c.resolve(&args(&["workflow"])).unwrap()),
            ["greengate", "greenpiste", "workflow"]
        );
    }

    #[test]
    fn resolving_a_part_returns_it_and_its_needs_only() {
        let c = Catalog::load().unwrap();
        assert_eq!(
            names(&c.resolve(&args(&["greengate"])).unwrap()),
            ["greengate"]
        );
        assert_eq!(names(&c.resolve(&args(&["gate"])).unwrap()), ["greengate"]);
    }

    #[test]
    fn enhances_are_never_installed_for_a_part() {
        let source = fixture(
            &["a", "b"],
            &[("a", &[], &[], &[]), ("b", &[], &["a"], &[])],
        );
        let c = Catalog::parse(&source).unwrap();
        assert_eq!(names(&c.resolve(&args(&["b"])).unwrap()), ["b"]);
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
        let installed: BTreeSet<String> = ["decisions", "conform", "greengate"]
            .map(String::from)
            .into();
        assert_eq!(c.dependents("decisions", &installed), ["conform"]);
        assert!(c.dependents("greengate", &installed).is_empty());
        assert!(c.dependents("gate", &installed).is_empty());
    }

    #[test]
    fn an_unknown_need_or_enhance_is_refused_naming_it() {
        refused(
            &fixture(&["a"], &[("a", &["ghost"], &[], &[])]),
            &["a", "needs", "ghost"],
        );
        refused(
            &fixture(&["a"], &[("a", &[], &["ghost"], &[])]),
            &["a", "enhances", "ghost"],
        );
    }

    #[test]
    fn a_needs_cycle_is_refused_naming_it() {
        refused(
            &fixture(
                &["a", "b"],
                &[("a", &["b"], &[], &[]), ("b", &["a"], &[], &[])],
            ),
            &["cycle", "a", "b"],
        );
    }

    #[test]
    fn an_enhances_cycle_is_refused_naming_it() {
        refused(
            &fixture(
                &["a", "b"],
                &[("a", &[], &["b"], &[]), ("b", &[], &["a"], &[])],
            ),
            &["cycle", "a", "b"],
        );
    }

    #[test]
    fn a_cycle_across_needs_and_enhances_is_refused() {
        refused(
            &fixture(
                &["a", "b"],
                &[("a", &["b"], &[], &[]), ("b", &[], &["a"], &[])],
            ),
            &["cycle"],
        );
    }

    #[test]
    fn an_upward_edge_is_refused_naming_it() {
        refused(
            &fixture(
                &["b", "a"],
                &[("a", &[], &[], &[]), ("b", &["a"], &[], &[])],
            ),
            &["b", "up", "a"],
        );
    }

    #[test]
    fn a_part_missing_from_order_is_refused_naming_it() {
        refused(
            &fixture(&["a"], &[("a", &[], &[], &[]), ("b", &[], &[], &[])]),
            &["b", "missing from order"],
        );
    }

    #[test]
    fn an_alias_colliding_with_a_name_or_an_alias_is_refused_naming_it() {
        refused(
            &fixture(
                &["a", "b"],
                &[("a", &[], &[], &[]), ("b", &[], &[], &["a"])],
            ),
            &["alias a", "b"],
        );
        refused(
            &fixture(
                &["a", "b"],
                &[("a", &[], &[], &["x"]), ("b", &[], &[], &["x"])],
            ),
            &["alias x"],
        );
    }

    #[test]
    fn a_part_with_a_bad_manifest_is_refused_naming_it() {
        let source = fixture(&["a"], &[("a", &[], &[], &[])])
            .replace(">=0.2", ">=0.1")
            .replace("needs = [", "summary = \"s\"\nneeds = [");
        refused(&source, &["part a"]);
    }
}
