//! The parts grh knows, read from `catalog.toml` compiled into the binary.
//! Each `[[part]]` is a greenharness.toml v2 manifest.

use std::collections::{BTreeMap, BTreeSet};

use serde::Deserialize;

use crate::exit::Exit;
use crate::fetch::sha256_hex;
use crate::manifest::{AuditList, Manifest, Scope};
use crate::ops::{text, Ui};

const CATALOG: &str = include_str!("../catalog.toml");

/// Every path to a `Catalog` goes through `Catalog::parse_with`, which runs
/// `checked`.
#[derive(Debug)]
pub struct Catalog {
    pub skills_repo: String,
    pub skills_version: String,
    pub skills_sha256: String,
    /// Names that used to install several parts, with their former members.
    /// They are refused with a hint, never expanded.
    pub retired_names: BTreeMap<String, Vec<String>>,
    pub audit: AuditPaths,
    /// A line of a placed file containing one of these marks it as another
    /// tool's: grh leaves such a file alone.
    pub foreign_markers: Vec<String>,
    /// ADR-0064's list of the parts: every edge points to an earlier entry.
    pub order: Vec<String>,
    pub parts: Vec<Part>,
}

/// The catalog as the file spells it, before pins are resolved and
/// `Catalog::checked` runs.
#[derive(Deserialize)]
struct RawCatalog {
    skills_repo: String,
    skills_version: String,
    skills_sha256: String,
    #[serde(default)]
    retired_names: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    audit: AuditPaths,
    foreign_markers: Vec<String>,
    order: Vec<String>,
    #[serde(rename = "part", default)]
    parts: Vec<Part>,
    #[serde(default)]
    pinned: Vec<Pin>,
}

/// A part whose manifest lives in its own repository's release: the catalog
/// holds where, and the sha256 of the bytes grh built in as its copy.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Pin {
    pub(crate) name: String,
    /// `owner/name` on GitHub.
    pub(crate) repo: String,
    /// The release tag, which starts with `v`.
    pub(crate) tag: String,
    #[serde(default = "default_asset")]
    pub(crate) asset: String,
    pub(crate) sha256: String,
}

fn default_asset() -> String {
    crate::manifest::FILE_NAME.to_string()
}

/// Pinned manifest copies built into grh by `build.rs`: `(name, bytes)`.
mod embedded {
    include!(concat!(env!("OUT_DIR"), "/manifests.rs"));
}

impl Pin {
    /// The part this pin stands for, from its copy: refused unless the copy
    /// is the bytes pinned and a valid manifest for exactly this release.
    fn resolve(&self, copies: &[(&str, &[u8])]) -> Result<Part, Exit> {
        let refuse = |why: String| invalid(format!("pinned part {}: {why}", self.name));
        let Some((_, bytes)) = copies.iter().find(|(name, _)| *name == self.name) else {
            return Err(refuse(format!(
                "no copy manifests/{}.toml of {} from {} {}",
                self.name, self.asset, self.repo, self.tag
            )));
        };
        self.check_copy(bytes).map_err(refuse)
    }

    /// The one check of a copy's bytes against this pin, shared by the
    /// catalog load and `pins::pin`: the reason it is refused, else its part.
    pub(crate) fn check_copy(&self, bytes: &[u8]) -> Result<Part, String> {
        let Some(version) = self.tag.strip_prefix('v') else {
            return Err(format!("the tag {} does not start with v", self.tag));
        };
        let got = sha256_hex(bytes);
        if !got.eq_ignore_ascii_case(&self.sha256) {
            return Err(format!(
                "manifests/{}.toml has sha256 {got}, the pin says {}",
                self.name, self.sha256
            ));
        }
        let source =
            std::str::from_utf8(bytes).map_err(|e| format!("the copy is not UTF-8: {e}"))?;
        let part = Manifest::parse(source).map_err(|e| text(&e))?;
        if part.name() != self.name {
            return Err(format!("the copy names the part {}", part.name()));
        }
        if part.extension.version != version {
            return Err(format!(
                "the copy is version {}, the tag {} says {version}",
                part.extension.version, self.tag
            ));
        }
        if let Some(release) = part.release() {
            if release.version != version {
                return Err(format!(
                    "its [tool.release] version is {}, the tag {} says {version}",
                    release.version, self.tag
                ));
            }
            if release.repo != self.repo {
                return Err(format!(
                    "its [tool.release] repo is {}, the pin says {}",
                    release.repo, self.repo
                ));
            }
        }
        if !part.is_published() {
            return Err("the copy says published = false".to_string());
        }
        Ok(part)
    }
}

/// The repository paths the audit's signals look for, relative to the root.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuditPaths {
    #[serde(default)]
    pub adr_folder: Vec<String>,
    #[serde(default)]
    pub claude_set_up: Vec<String>,
    #[serde(default)]
    pub ci_present: Vec<String>,
}

/// The command a name was typed for, so a notice can name its replacement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verb {
    Install,
    Uninstall,
    Dispatch,
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

/// Every `*.toml` file of `dir` as `(file stem, bytes)`, sorted; a missing
/// directory holds no copies.
pub(crate) fn read_copies(dir: &std::path::Path) -> std::io::Result<Vec<(String, Vec<u8>)>> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let mut copies = Vec::new();
    for entry in entries {
        let path = entry?.path();
        let is_copy = path.extension().is_some_and(|x| x == "toml") && path.is_file();
        let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned());
        if let (true, Some(stem)) = (is_copy, stem) {
            copies.push((stem, std::fs::read(&path)?));
        }
    }
    copies.sort();
    Ok(copies)
}

/// Where the catalog grh loads lives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CatalogLocation {
    /// The file `GRH_CATALOG` names, and the `manifests/` beside it.
    Named {
        file: std::path::PathBuf,
        copies_dir: std::path::PathBuf,
    },
    /// This repository's own `catalog.toml` and `manifests/`, the ones built
    /// into grh.
    Repository {
        file: std::path::PathBuf,
        copies_dir: std::path::PathBuf,
    },
}

impl CatalogLocation {
    pub fn file(&self) -> &std::path::Path {
        match self {
            CatalogLocation::Named { file, .. } | CatalogLocation::Repository { file, .. } => file,
        }
    }

    pub fn copies_dir(&self) -> &std::path::Path {
        match self {
            CatalogLocation::Named { copies_dir, .. }
            | CatalogLocation::Repository { copies_dir, .. } => copies_dir,
        }
    }
}

fn copies_beside(file: &std::path::Path) -> std::path::PathBuf {
    file.parent()
        .unwrap_or(std::path::Path::new("."))
        .join("manifests")
}

/// The file `GRH_CATALOG` names and the `manifests/` beside it, else this
/// repository's own `catalog.toml` and `manifests/`.
pub fn catalog_location() -> CatalogLocation {
    match std::env::var_os("GRH_CATALOG").filter(|path| !path.is_empty()) {
        Some(named) => {
            let file = std::path::PathBuf::from(named);
            let copies_dir = copies_beside(&file);
            CatalogLocation::Named { file, copies_dir }
        }
        None => {
            let file = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("catalog.toml");
            let copies_dir = copies_beside(&file);
            CatalogLocation::Repository { file, copies_dir }
        }
    }
}

/// The inline parts and the resolved pins, in the order their `[[part]]` and
/// `[[pinned]]` tables appear in `source`. Parts are listed (the audit, the
/// install checklist) in catalog order, so a part keeps its place when it
/// moves from inline to pinned. TOML keeps each array's own order but not how
/// the two interleave, so the headers are counted from the text; when they
/// cannot be (a table written another way), the pins follow the inline parts.
fn in_declared_order(source: &str, inline: Vec<Part>, pinned: Vec<Part>) -> Vec<Part> {
    let mut sequence = Vec::with_capacity(inline.len() + pinned.len());
    for line in source.lines() {
        match line.split('#').next().unwrap_or_default().trim() {
            "[[part]]" => sequence.push(false),
            "[[pinned]]" => sequence.push(true),
            _ => {}
        }
    }
    let pins = sequence.iter().filter(|is_pin| **is_pin).count();
    if pins != pinned.len() || sequence.len() - pins != inline.len() {
        let mut parts = inline;
        parts.extend(pinned);
        return parts;
    }
    let (mut inline, mut pinned) = (inline.into_iter(), pinned.into_iter());
    sequence
        .into_iter()
        .filter_map(|is_pin| if is_pin { pinned.next() } else { inline.next() })
        .collect()
}

impl Catalog {
    /// The compiled-in catalog, or the file `GRH_CATALOG` names (a test hook
    /// so the integration tests can pin their own fixture tarballs). A bad
    /// file is a failure, never a panic.
    pub fn load() -> Result<Catalog, Exit> {
        let location = match catalog_location() {
            CatalogLocation::Repository { .. } => {
                return Catalog::parse_with(CATALOG, embedded::MANIFESTS)
                    .map_err(|e| Exit::Failed(format!("catalog.toml is invalid: {}", text(&e))));
            }
            named @ CatalogLocation::Named { .. } => named,
        };
        let shown = location.file().display();
        let source = std::fs::read_to_string(location.file())
            .map_err(|e| Exit::Failed(format!("GRH_CATALOG {shown} is not readable: {e}")))?;
        let owned = read_copies(location.copies_dir()).map_err(|e| {
            Exit::Failed(format!(
                "GRH_CATALOG {shown}: cannot read {}: {e}",
                location.copies_dir().display()
            ))
        })?;
        let copies: Vec<(&str, &[u8])> = owned
            .iter()
            .map(|(name, bytes)| (name.as_str(), bytes.as_slice()))
            .collect();
        Catalog::parse_with(&source, &copies).map_err(|e| {
            Exit::Failed(format!(
                "GRH_CATALOG {shown} is not a valid catalog: {}",
                text(&e)
            ))
        })
    }

    /// Parse a catalog with no pinned copies: a pin is refused for having none.
    pub fn parse(source: &str) -> Result<Catalog, Exit> {
        Catalog::parse_with(source, &[])
    }

    /// Parse a catalog, resolve every pin into a part from its copy
    /// (`(name, bytes)`), and only then run `checked`, so a pinned part is
    /// checked as an inline one is.
    pub fn parse_with(source: &str, copies: &[(&str, &[u8])]) -> Result<Catalog, Exit> {
        let raw: RawCatalog = toml::from_str(source).map_err(|e| Exit::Failed(e.to_string()))?;
        let mut pinned_names: BTreeSet<&str> = BTreeSet::new();
        let mut pinned = Vec::with_capacity(raw.pinned.len());
        for pin in &raw.pinned {
            if !pinned_names.insert(&pin.name) {
                return Err(invalid(format!("two pins name the part {}", pin.name)));
            }
            if raw.parts.iter().any(|p| p.name() == pin.name) {
                return Err(invalid(format!(
                    "the part {} is both a [[part]] and a [[pinned]]",
                    pin.name
                )));
            }
            pinned.push(pin.resolve(copies)?);
        }
        let parts = in_declared_order(source, raw.parts, pinned);
        if let Some((name, _)) = copies.iter().find(|(name, _)| !pinned_names.contains(name)) {
            return Err(invalid(format!(
                "manifests/{name}.toml is a copy of no pinned part"
            )));
        }
        Catalog {
            skills_repo: raw.skills_repo,
            skills_version: raw.skills_version,
            skills_sha256: raw.skills_sha256,
            retired_names: raw.retired_names,
            audit: raw.audit,
            foreign_markers: raw.foreign_markers,
            order: raw.order,
            parts,
        }
        .checked()
    }

    /// Every part is a valid manifest; names and aliases are unique; every
    /// `needs` and `enhances` names a part, and every retired name is no part
    /// name or alias and lists published parts only; no cycle runs
    /// through `needs` and `enhances` together; every part is in `order` and
    /// every edge points to an earlier entry; no part that is not
    /// experimental needs an experimental one. So `resolve` can walk the
    /// graph without a lookup that could fail, and never adds an experiment
    /// for another part.
    fn checked(self) -> Result<Catalog, Exit> {
        for part in &self.parts {
            part.check()
                .map_err(|e| invalid(format!("part {}: {}", part.name(), text(&e))))?;
            for rule in &part.recommend {
                for c in &rule.when {
                    let Scope::RepositoryPaths(audit_list) = c.signal.scope() else {
                        continue;
                    };
                    let name = audit_list.name();
                    let list = match audit_list {
                        AuditList::AdrFolder => &self.audit.adr_folder,
                        AuditList::ClaudeSetUp => &self.audit.claude_set_up,
                        AuditList::CiPresent => &self.audit.ci_present,
                    };
                    if list.is_empty() {
                        return Err(invalid(format!(
                            "part {} uses the signal {name}, but [audit] {name} lists no path, so it could never hold",
                            part.name()
                        )));
                    }
                }
            }
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
        for part in self.parts.iter().filter(|p| !p.is_experimental()) {
            if let Some(need) = part
                .needs()
                .iter()
                .filter_map(|n| self.part(n))
                .find(|n| n.is_experimental())
            {
                return Err(invalid(format!(
                    "{} needs {}, which is experimental; only an experimental part may need one",
                    part.name(),
                    need.name()
                )));
            }
        }
        for (retired, members) in &self.retired_names {
            if self.part(retired).is_some() {
                return Err(invalid(format!(
                    "the retired name {retired} is also a part name or alias"
                )));
            }
            if let Some(member) = members
                .iter()
                .find(|m| !self.part(m).is_some_and(Part::is_published))
            {
                return Err(invalid(format!(
                    "the retired name {retired} lists {member}, which is not a published part"
                )));
            }
        }
        if self.foreign_markers.iter().all(|m| m.is_empty()) {
            return Err(invalid(
                "foreign_markers must list at least one non-empty mark: files carrying one are never overwritten or removed",
            ));
        }
        let audit = &self.audit;
        for path in audit
            .adr_folder
            .iter()
            .chain(&audit.claude_set_up)
            .chain(&audit.ci_present)
        {
            if !crate::manifest::stays_inside(path) {
                return Err(invalid(format!(
                    "the [audit] path {path:?} must be relative and stay inside the repository"
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
        let ranks: BTreeMap<&str, usize> = self
            .order
            .iter()
            .enumerate()
            .map(|(rank, name)| (name.as_str(), rank))
            .collect();
        let rank = |name: &str| ranks.get(name).copied();
        for part in &self.parts {
            for edge in self.edges(part.name()) {
                if rank(edge) >= rank(part.name()) {
                    return Err(invalid(format!(
                        "{} points up order to {edge}: the edge names a part at or after it in order",
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

    /// The notice for a part asked for by an alias, `None` for its own name.
    pub fn alias_notice(&self, name: &str, verb: Verb) -> Option<String> {
        if self.parts.iter().any(|p| p.name() == name) {
            return None;
        }
        let canonical = self.name_of_alias(name)?;
        let command = match verb {
            Verb::Install => format!("grh install {canonical}"),
            Verb::Uninstall => format!("grh uninstall {canonical}"),
            Verb::Dispatch => format!("grh {canonical}"),
        };
        Some(format!("{name} is now {canonical}: run {command}"))
    }

    /// One warning per alias among `names`, naming the command to use instead.
    pub fn warn_aliases(&self, ui: &Ui, names: &[String], verb: Verb) {
        let mut seen: Vec<&String> = Vec::new();
        for name in names {
            if seen.contains(&name) {
                continue;
            }
            seen.push(name);
            if let Some(notice) = self.alias_notice(name, verb) {
                ui.warn(&notice);
            }
        }
    }

    /// The published parts that are not experiments, by name, in catalog
    /// order: what `install --all` installs.
    pub fn all_names(&self) -> Vec<String> {
        self.parts
            .iter()
            .filter(|p| p.is_published() && !p.is_experimental())
            .map(|p| p.name().to_string())
            .collect()
    }

    /// The published experimental parts, by name, in catalog order.
    pub fn experimental_names(&self) -> Vec<&str> {
        self.parts
            .iter()
            .filter(|p| p.is_published() && p.is_experimental())
            .map(|p| p.name())
            .collect()
    }

    /// The members `name` used to stand for, when it is a retired name.
    pub fn retired_members(&self, name: &str) -> Option<&[String]> {
        self.retired_names.get(name).map(Vec::as_slice)
    }

    /// The usage error for the first retired name among `names`, naming the
    /// command that does what the old name did.
    pub fn retired_notice(&self, names: &[String], verb: Verb) -> Option<String> {
        let (name, members) = names
            .iter()
            .find_map(|n| self.retired_members(n).map(|m| (n, m)))?;
        let members = members.join(" ");
        Some(match verb {
            Verb::Uninstall => format!(
                "profiles were removed in 0.3.0.\n  to remove what {name} installed: grh uninstall {members}"
            ),
            Verb::Install | Verb::Dispatch => format!(
                "profiles were removed in 0.3.0.\n  to install what {name} installed: grh install {members}\n  or every part but the experiments: grh install --all"
            ),
        })
    }

    /// Parts named on the command line, with every dependency added, in
    /// install order (a part after what it needs). `enhances` are never added.
    pub fn resolve(&self, names: &[String]) -> Result<Vec<&Part>, Unknown> {
        let mut wanted = BTreeSet::new();
        for name in names {
            if let Some(part) = self.part(name) {
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
            "skills_repo = \"o/s\"\nskills_version = \"1\"\nskills_sha256 = \"x\"\nforeign_markers = [\"m:begin\"]\norder = {}\n[retired_names]\n",
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

    /// A copy of the part `lab`, version 1.2.0, with `extra` appended.
    fn lab_copy(extra: &str) -> String {
        format!("[extension]\nname = \"lab\"\nversion = \"1.2.0\"\nharness = \">=0.2\"\n{extra}")
    }

    /// A catalog where inline part `a` needs the pinned part `lab`, pinned at
    /// `tag` from `repo` with the sha256 of `pinned_bytes`.
    fn with_lab_pin(pinned_bytes: &str, tag: &str, repo: &str) -> String {
        fixture(&["lab", "a"], &[("a", &["lab"], &[], &[])])
            + &format!(
                "[[pinned]]\nname = \"lab\"\nrepo = \"{repo}\"\ntag = \"{tag}\"\nsha256 = \"{}\"\n",
                sha256_hex(pinned_bytes.as_bytes())
            )
    }

    fn refused_with(source: &str, copies: &[(&str, &str)], mentions: &[&str]) {
        let copies: Vec<(&str, &[u8])> = copies.iter().map(|(n, b)| (*n, b.as_bytes())).collect();
        let message = match Catalog::parse_with(source, &copies) {
            Ok(_) => panic!("the catalog was accepted:\n{source}"),
            Err(e) => text(&e),
        };
        for word in mentions {
            assert!(message.contains(word), "{message:?} does not name {word}");
        }
    }

    const TOOL: &str = "[tool]\nbin = \"lab\"\n[tool.release]\nrepo = \"o/lab\"\nversion = \"1.2.0\"\n[tool.release.sha256]\nx86_64-unknown-linux-gnu = \"00\"\n";

    #[test]
    fn a_pinned_part_resolves_and_takes_part_in_order_and_needs() {
        let copy = lab_copy("");
        let source = with_lab_pin(&copy, "v1.2.0", "o/lab");
        let catalog = Catalog::parse_with(&source, &[("lab", copy.as_bytes())]).unwrap();
        assert!(catalog.part("lab").is_some());
        assert_eq!(catalog.order, ["lab", "a"]);
        let resolved: Vec<&str> = catalog
            .resolve(&args(&["a"]))
            .unwrap()
            .iter()
            .map(|p| p.name())
            .collect();
        assert_eq!(resolved, ["lab", "a"]);
    }

    #[test]
    fn a_pinned_part_keeps_its_declared_place_among_the_inline_parts() {
        let copy = lab_copy("");
        let source = fixture(&["a", "lab", "b"], &[("a", &[], &[], &[])])
            + &format!(
                "[[pinned]]\nname = \"lab\"\nrepo = \"o/lab\"\ntag = \"v1.2.0\"\nsha256 = \"{}\"\n",
                sha256_hex(copy.as_bytes())
            )
            + "[[part]]\n[part.extension]\nname = \"b\"\nversion = \"1\"\nharness = \">=0.2\"\n";
        let catalog = Catalog::parse_with(&source, &[("lab", copy.as_bytes())]).unwrap();
        let listed: Vec<&str> = catalog.parts.iter().map(|p| p.name()).collect();
        assert_eq!(listed, ["a", "lab", "b"]);
    }

    #[test]
    fn a_pinned_part_with_a_tool_of_this_release_resolves() {
        let copy = lab_copy(TOOL);
        let source = with_lab_pin(&copy, "v1.2.0", "o/lab");
        assert!(Catalog::parse_with(&source, &[("lab", copy.as_bytes())]).is_ok());
    }

    #[test]
    fn each_pin_refusal_names_the_part_and_why() {
        let copy = lab_copy("");
        // no copy, on the embedded and the sibling path alike: parse_with
        // takes whatever copies the caller found
        refused_with(
            &with_lab_pin(&copy, "v1.2.0", "o/lab"),
            &[],
            &["lab", "no copy"],
        );
        refused(&with_lab_pin(&copy, "v1.2.0", "o/lab"), &["lab", "no copy"]);
        // the copy is not the pinned bytes
        let other = lab_copy("summary = \"changed\"\n");
        refused_with(
            &with_lab_pin(&copy, "v1.2.0", "o/lab"),
            &[("lab", &other)],
            &[
                "lab",
                "sha256",
                &sha256_hex(other.as_bytes()),
                &sha256_hex(copy.as_bytes()),
            ],
        );
        // a tag without v
        refused_with(
            &with_lab_pin(&copy, "1.2.0", "o/lab"),
            &[("lab", &copy)],
            &["lab", "does not start with v"],
        );
        // not a manifest
        let junk = "not = [toml";
        refused_with(
            &with_lab_pin(junk, "v1.2.0", "o/lab"),
            &[("lab", junk)],
            &["lab"],
        );
        // the copy names another part
        let renamed = copy.replace("name = \"lab\"", "name = \"other\"");
        refused_with(
            &with_lab_pin(&renamed, "v1.2.0", "o/lab"),
            &[("lab", &renamed)],
            &["lab", "names the part other"],
        );
        // [extension] version is not the tag
        refused_with(
            &with_lab_pin(&copy, "v1.3.0", "o/lab"),
            &[("lab", &copy)],
            &["lab", "version 1.2.0", "1.3.0"],
        );
        // [tool.release] version or repo differs
        let tool_old = lab_copy(&TOOL.replace("version = \"1.2.0\"", "version = \"1.1.0\""));
        refused_with(
            &with_lab_pin(&tool_old, "v1.2.0", "o/lab"),
            &[("lab", &tool_old)],
            &["lab", "[tool.release] version is 1.1.0"],
        );
        let tool = lab_copy(TOOL);
        refused_with(
            &with_lab_pin(&tool, "v1.2.0", "o/elsewhere"),
            &[("lab", &tool)],
            &["lab", "[tool.release] repo is o/lab"],
        );
        // published = false
        let hidden = lab_copy("published = false\n");
        refused_with(
            &with_lab_pin(&hidden, "v1.2.0", "o/lab"),
            &[("lab", &hidden)],
            &["lab", "published = false"],
        );
        // a copy with no pin
        refused_with(
            &fixture(&["a"], &[("a", &[], &[], &[])]),
            &[("stray", "x")],
            &["manifests/stray.toml", "no pinned part"],
        );
        // a name both inline and pinned
        let both = fixture(&["lab"], &[("lab", &[], &[], &[])])
            + &format!(
                "[[pinned]]\nname = \"lab\"\nrepo = \"o/lab\"\ntag = \"v1.2.0\"\nsha256 = \"{}\"\n",
                sha256_hex(copy.as_bytes())
            );
        refused_with(
            &both,
            &[("lab", &copy)],
            &["lab", "both a [[part]] and a [[pinned]]"],
        );
        // two pins with one name
        let twice = with_lab_pin(&copy, "v1.2.0", "o/lab")
            + &format!(
                "[[pinned]]\nname = \"lab\"\nrepo = \"o/lab\"\ntag = \"v1.2.0\"\nsha256 = \"{}\"\n",
                sha256_hex(copy.as_bytes())
            );
        refused_with(&twice, &[("lab", &copy)], &["two pins name the part lab"]);
        // an alias that clashes with an inline part's name
        let aliased = lab_copy("aliases = [\"a\"]\n");
        refused_with(
            &with_lab_pin(&aliased, "v1.2.0", "o/lab"),
            &[("lab", &aliased)],
            &["a"],
        );
    }

    #[test]
    fn a_part_that_is_not_experimental_may_not_need_an_experimental_one() {
        let source = fixture(
            &["a", "b"],
            &[("a", &[], &[], &[]), ("b", &["a"], &[], &[])],
        )
        .replace(">=0.2", ">=0.4")
        .replace("name = \"a\"\n", "name = \"a\"\nexperimental = true\n");
        refused(&source, &["b needs a, which is experimental"]);
        let both = source.replace("name = \"b\"\n", "name = \"b\"\nexperimental = true\n");
        assert!(Catalog::parse(&both).is_ok());
    }

    #[test]
    fn a_path_signal_with_no_audit_paths_is_refused() {
        let rule = "[[part.recommend]]\nwhen = [\"ci_present\"]\nverdict = \"recommend\"\nreason = \"ci\"\n";
        let source = fixture(&["a"], &[("a", &[], &[], &[])]).replace(">=0.2", ">=0.3") + rule;
        refused(&source, &["a", "ci_present", "[audit]"]);
        let with_paths = source.replacen(
            "[retired_names]\n",
            "[retired_names]\n[audit]\nci_present = [\".ci\"]\n",
            1,
        );
        assert!(Catalog::parse(&with_paths).is_ok());
    }

    #[test]
    fn the_shipped_catalog_passes_checked_and_every_edge_points_down_order() {
        let catalog = Catalog::load().unwrap();
        let rank = |n: &str| catalog.order.iter().position(|o| o == n).unwrap();
        for p in &catalog.parts {
            for edge in p.needs().iter().chain(p.enhances()) {
                let target = catalog.part(edge).unwrap();
                assert!(
                    rank(target.name()) < rank(p.name()),
                    "{} -> {edge}",
                    p.name()
                );
            }
        }
        assert_eq!(catalog.part("gate").unwrap().name(), "greengate");
        assert_eq!(catalog.part("guardrails").unwrap().name(), "greenpiste");
    }

    #[test]
    fn the_embedded_manifests_are_the_copies_in_the_repository() {
        let on_disk =
            read_copies(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("manifests"))
                .unwrap();
        let on_disk: Vec<(&str, &[u8])> = on_disk
            .iter()
            .map(|(name, bytes)| (name.as_str(), bytes.as_slice()))
            .collect();
        assert_eq!(embedded::MANIFESTS, on_disk.as_slice());
    }

    #[test]
    fn a_toml_string_that_breaks_an_invariant_does_not_deserialise_into_a_catalog() {
        let source = fixture(
            &["a", "b"],
            &[("a", &["b"], &[], &[]), ("b", &[], &[], &[])],
        );
        let error = text(&Catalog::parse(&source).unwrap_err());
        assert!(error.contains("at or after it in order"), "{error}");
    }

    #[test]
    fn every_need_names_a_part_and_no_retired_name_collides_with_one() {
        let catalog = Catalog::load().unwrap();
        for p in &catalog.parts {
            for n in p.needs() {
                assert!(catalog.part(n).is_some(), "{} needs unknown {n}", p.name());
            }
        }
        assert!(!catalog.retired_names.is_empty());
        for (name, members) in &catalog.retired_names {
            assert!(
                catalog.part(name).is_none(),
                "retired name {name} shadows a part"
            );
            for m in members {
                assert!(
                    catalog.part(m).is_some_and(Part::is_published),
                    "retired name {name} lists {m}, which is not a published part"
                );
            }
        }
    }

    #[test]
    fn a_binary_part_pins_every_unix_target() {
        let catalog = Catalog::load().unwrap();
        for p in catalog.parts.iter().filter(|p| p.bin().is_some()) {
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
    fn a_retired_name_is_refused_when_it_is_a_part_or_lists_an_unpublished_part() {
        let base = fixture(
            &["a", "b"],
            &[("a", &[], &[], &["x"]), ("b", &[], &[], &[])],
        );
        let with =
            |table: &str| base.replace("[retired_names]\n", &format!("[retired_names]\n{table}"));
        assert!(Catalog::parse(&with("old = [\"a\", \"b\"]\n")).is_ok());
        refused(&with("a = [\"b\"]\n"), &["retired name a"]);
        refused(&with("x = [\"b\"]\n"), &["retired name x"]);
        refused(&with("old = [\"ghost\"]\n"), &["old", "ghost"]);
        let unpublished =
            with("old = [\"b\"]\n").replace("name = \"b\"\n", "name = \"b\"\npublished = false\n");
        refused(&unpublished, &["old", "b", "published"]);
    }

    #[test]
    fn the_retired_notice_names_the_former_members_and_the_verb() {
        let catalog = Catalog::load().unwrap();
        let notice = catalog
            .retired_notice(&args(&["solo"]), Verb::Install)
            .unwrap();
        assert_eq!(
            notice,
            "profiles were removed in 0.3.0.\n  to install what solo installed: grh install greengate greenpiste\n  or every part but the experiments: grh install --all"
        );
        let notice = catalog
            .retired_notice(&args(&["greengate", "solo"]), Verb::Uninstall)
            .unwrap();
        assert!(
            notice.ends_with("to remove what solo installed: grh uninstall greengate greenpiste")
        );
        assert!(catalog
            .retired_notice(&args(&["greengate"]), Verb::Install)
            .is_none());
    }

    #[test]
    fn all_names_leave_out_an_unpublished_part() {
        let source = fixture(&["a", "b"], &[("a", &[], &[], &[]), ("b", &[], &[], &[])])
            .replace("name = \"b\"\n", "name = \"b\"\npublished = false\n");
        let catalog = Catalog::parse(&source).unwrap();
        assert_eq!(catalog.all_names(), ["a"]);
    }

    #[test]
    fn dependencies_are_added_before_what_needs_them() {
        let catalog = Catalog::load().unwrap();
        assert_eq!(
            names(&catalog.resolve(&args(&["conform"])).unwrap()),
            ["decisions", "conform"]
        );
        assert_eq!(
            names(&catalog.resolve(&args(&["attest"])).unwrap()),
            ["greengate", "attest"]
        );
        assert_eq!(
            names(&catalog.resolve(&args(&["workflow"])).unwrap()),
            ["greengate", "greenpiste", "workflow"]
        );
    }

    #[test]
    fn resolving_a_part_returns_it_and_its_needs_only() {
        let catalog = Catalog::load().unwrap();
        assert_eq!(
            names(&catalog.resolve(&args(&["greengate"])).unwrap()),
            ["greengate"]
        );
        assert_eq!(
            names(&catalog.resolve(&args(&["gate"])).unwrap()),
            ["greengate"]
        );
    }

    #[test]
    fn enhances_are_never_installed_for_a_part() {
        let source = fixture(
            &["a", "b"],
            &[("a", &[], &[], &[]), ("b", &[], &["a"], &[])],
        );
        let catalog = Catalog::parse(&source).unwrap();
        assert_eq!(names(&catalog.resolve(&args(&["b"])).unwrap()), ["b"]);
    }

    #[test]
    fn an_unknown_name_is_refused() {
        let catalog = Catalog::load().unwrap();
        assert_eq!(
            catalog.resolve(&args(&["gate", "nope"])).unwrap_err(),
            Unknown::Name("nope".into())
        );
    }

    #[test]
    fn dependents_are_the_installed_parts_that_need_it() {
        let catalog = Catalog::load().unwrap();
        let installed: BTreeSet<String> = ["decisions", "conform", "greengate"]
            .map(String::from)
            .into();
        assert_eq!(catalog.dependents("decisions", &installed), ["conform"]);
        assert!(catalog.dependents("greengate", &installed).is_empty());
        assert!(catalog.dependents("gate", &installed).is_empty());
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
