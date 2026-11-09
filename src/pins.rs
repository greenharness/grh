//! Pinned manifests: fetch a release's manifest asset into `manifests/` and
//! re-check every pin of a catalog against its release. The `pins` example is
//! a thin wrapper over this; nothing here prints.

use std::fmt;
use std::path::Path;

use serde::Deserialize;

use crate::catalog::{read_copies, Catalog, Pin};
use crate::exit::Exit;
use crate::fetch::{
    download, download_with_options, release_base, sha256_hex, FetchFailure, FetchOptions,
};
use crate::manifest::FILE_NAME;
use crate::ops::text;
use crate::receipt::write_atomic;

/// What `pin` does with a copy already holding other bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Existing {
    Keep,
    Replace,
}

/// Why `pin` did not write a copy.
#[derive(Debug, PartialEq, Eq)]
pub enum PinError {
    Download(FetchFailure),
    /// The asset is not a manifest for this pin, or an argument is unusable.
    Invalid(String),
    /// A copy with other bytes exists and `replace` was not given.
    WouldOverwrite(String),
    Write(Exit),
}

impl std::error::Error for PinError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            PinError::Download(failure) => Some(failure),
            PinError::Invalid(_) | PinError::WouldOverwrite(_) | PinError::Write(_) => None,
        }
    }
}

impl PinError {
    /// True when the only trouble was reaching the host.
    pub fn is_unreachable(&self) -> bool {
        matches!(self, PinError::Download(FetchFailure::Unreachable(_)))
    }
}

impl fmt::Display for PinError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PinError::Download(failure) => write!(f, "{failure}"),
            PinError::Invalid(message) | PinError::WouldOverwrite(message) => f.write_str(message),
            PinError::Write(exit) => f.write_str(&text(exit)),
        }
    }
}

fn asset_url(base: &str, repo: &str, tag: &str, asset: &str) -> String {
    format!(
        "{}/{repo}/releases/download/{tag}/{asset}",
        base.trim_end_matches('/')
    )
}

/// Download a release's manifest asset, check it, write
/// `<copies_dir>/<name>.toml` and return the `[[pinned]]` block for the
/// catalog. A copy with other bytes is kept unless `replace`.
pub fn pin(
    copies_dir: &Path,
    name: &str,
    repo: &str,
    tag: &str,
    asset: &str,
    existing: Existing,
) -> Result<String, PinError> {
    pin_via(
        &release_base(),
        download,
        copies_dir,
        Target {
            name,
            repo,
            tag,
            asset,
        },
        existing,
    )
}

/// `pin` with `options` (release host, API host, token, retry delay)
/// instead of the environment: for tests.
#[doc(hidden)]
pub fn pin_with(
    options: &FetchOptions,
    copies_dir: &Path,
    name: &str,
    repo: &str,
    tag: &str,
    asset: &str,
    existing: Existing,
) -> Result<String, PinError> {
    pin_via(
        &options.release_base,
        |url| download_with_options(url, options),
        copies_dir,
        Target {
            name,
            repo,
            tag,
            asset,
        },
        existing,
    )
}

/// A repository, tag or asset segment that cannot climb out of its place in
/// the URL: not empty, no `..`, no path separator.
fn plain_segment(segment: &str) -> bool {
    !segment.is_empty() && !segment.contains("..") && !segment.contains(['/', '\\'])
}

/// True for a name `pin` can write as `manifests/<name>.toml`: letters,
/// digits, `-` and `_`.
pub fn is_part_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn refuse_unusable(name: &str, repo: &str, tag: &str, asset: &str) -> Result<(), PinError> {
    if !is_part_name(name) {
        return Err(PinError::Invalid(format!(
            "{name:?} is not a part name (letters, digits, - and _)"
        )));
    }
    let usable_repo = repo
        .split_once('/')
        .is_some_and(|(owner, project)| plain_segment(owner) && plain_segment(project));
    if !usable_repo {
        return Err(PinError::Invalid(format!(
            "{repo:?} is not a repository (owner/name)"
        )));
    }
    if !plain_segment(tag) {
        return Err(PinError::Invalid(format!("{tag:?} is not a release tag")));
    }
    if !plain_segment(asset) {
        return Err(PinError::Invalid(format!("{asset:?} is not an asset name")));
    }
    Ok(())
}

/// The release asset `pin` is asked for.
#[derive(Clone, Copy)]
struct Target<'a> {
    name: &'a str,
    repo: &'a str,
    tag: &'a str,
    asset: &'a str,
}

fn pin_via(
    base: &str,
    fetch: impl Fn(&str) -> Result<Vec<u8>, FetchFailure>,
    copies_dir: &Path,
    target: Target<'_>,
    existing: Existing,
) -> Result<String, PinError> {
    let Target {
        name,
        repo,
        tag,
        asset,
    } = target;
    refuse_unusable(name, repo, tag, asset)?;
    let url = asset_url(base, repo, tag, asset);
    let bytes = fetch(&url).map_err(PinError::Download)?;
    let wanted = Pin {
        name: name.to_string(),
        repo: repo.to_string(),
        tag: tag.to_string(),
        asset: asset.to_string(),
        sha256: sha256_hex(&bytes),
    };
    wanted
        .check_copy(&bytes)
        .map_err(|why| PinError::Invalid(format!("{url}: pinned part {name}: {why}")))?;
    let path = copies_dir.join(format!("{name}.toml"));
    match std::fs::read(&path) {
        Ok(held) if held != bytes && existing == Existing::Keep => {
            return Err(PinError::WouldOverwrite(format!(
                "{} holds other bytes than {url}; --replace overwrites it",
                path.display()
            )));
        }
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => {
            return Err(PinError::Write(Exit::Failed(format!(
                "cannot read {}: {e}",
                path.display()
            ))));
        }
    }
    write_atomic(&path, &bytes, 0o644).map_err(PinError::Write)?;
    let asset_line = if asset == FILE_NAME {
        String::new()
    } else {
        format!("asset = {asset:?}\n")
    };
    Ok(format!(
        "[[pinned]]\nname = {name:?}\nrepo = {repo:?}\ntag = {tag:?}\n{asset_line}sha256 = \"{}\"\n",
        sha256_hex(&bytes)
    ))
}

#[derive(Deserialize)]
struct CatalogPins {
    #[serde(default)]
    pinned: Vec<Pin>,
}

/// What `check` found for one pin.
#[derive(Debug, PartialEq, Eq)]
pub enum PinOutcome {
    Match,
    /// The asset, the pin and the copy are not one sum (`None`: no copy).
    Mismatch {
        asset: String,
        pin: String,
        copy: Option<String>,
    },
    NotFound(String),
    Unreachable(String),
    Other(String),
}

#[derive(Debug, PartialEq, Eq)]
pub struct PinResult {
    pub name: String,
    pub outcome: PinOutcome,
}

impl fmt::Display for PinResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = &self.name;
        match &self.outcome {
            PinOutcome::Match => write!(f, "{name}: ok"),
            PinOutcome::Mismatch { asset, pin, copy } => write!(
                f,
                "{name}: mismatch (asset {asset}, pin {pin}, copy {})",
                copy.as_deref().unwrap_or("missing")
            ),
            PinOutcome::NotFound(why) => write!(f, "{name}: not found ({why})"),
            PinOutcome::Unreachable(why) => write!(f, "{name}: unreachable ({why})"),
            PinOutcome::Other(why) => write!(f, "{name}: failed ({why})"),
        }
    }
}

/// Download every pin of `catalog_source` and compare its sha256 with the
/// pin and with `<copies_dir>/<name>.toml`: one result per pin, in catalog
/// order, every pin checked. A catalog grh would refuse, with the copies in
/// `copies_dir`, is refused before any download.
pub fn check(catalog_source: &str, copies_dir: &Path) -> Result<Vec<PinResult>, Exit> {
    let base = release_base();
    check_via(
        |_| base.clone(),
        |_, url| download(url),
        catalog_source,
        copies_dir,
    )
}

/// `check` with the options `options_of(pin name)` names (release host, API
/// host, token, retry delay) instead of the environment: for tests.
#[doc(hidden)]
pub fn check_with(
    options_of: impl Fn(&str) -> FetchOptions,
    catalog_source: &str,
    copies_dir: &Path,
) -> Result<Vec<PinResult>, Exit> {
    check_via(
        |name| options_of(name).release_base,
        |name, url| download_with_options(url, &options_of(name)),
        catalog_source,
        copies_dir,
    )
}

fn check_via(
    base_of: impl Fn(&str) -> String,
    fetch: impl Fn(&str, &str) -> Result<Vec<u8>, FetchFailure>,
    catalog_source: &str,
    copies_dir: &Path,
) -> Result<Vec<PinResult>, Exit> {
    let owned = read_copies(copies_dir)
        .map_err(|e| Exit::Failed(format!("cannot read {}: {e}", copies_dir.display())))?;
    let copies: Vec<(&str, &[u8])> = owned
        .iter()
        .map(|(name, bytes)| (name.as_str(), bytes.as_slice()))
        .collect();
    Catalog::parse_with(catalog_source, &copies)?;
    let catalog: CatalogPins =
        toml::from_str(catalog_source).map_err(|e| Exit::Failed(format!("the catalog: {e}")))?;
    Ok(catalog
        .pinned
        .iter()
        .map(|pin| {
            let copy = copies
                .iter()
                .find(|(name, _)| *name == pin.name)
                .map(|(_, bytes)| *bytes);
            let url = asset_url(&base_of(&pin.name), &pin.repo, &pin.tag, &pin.asset);
            PinResult {
                name: pin.name.clone(),
                outcome: check_one(fetch(&pin.name, &url), pin, copy),
            }
        })
        .collect())
}

fn check_one(fetched: Result<Vec<u8>, FetchFailure>, pin: &Pin, copy: Option<&[u8]>) -> PinOutcome {
    let bytes = match fetched {
        Ok(bytes) => bytes,
        Err(FetchFailure::NotFound(m)) => return PinOutcome::NotFound(m),
        Err(FetchFailure::Unreachable(m)) => return PinOutcome::Unreachable(m),
        Err(FetchFailure::Other(m)) => return PinOutcome::Other(m),
    };
    let asset = sha256_hex(&bytes);
    let copy = copy.map(sha256_hex);
    if asset.eq_ignore_ascii_case(&pin.sha256) && copy.as_deref() == Some(asset.as_str()) {
        PinOutcome::Match
    } else {
        PinOutcome::Mismatch {
            asset,
            pin: pin.sha256.clone(),
            copy,
        }
    }
}

/// The `pins` example's exit code for `check`: 1 when any pin mismatched,
/// is missing or failed otherwise, else 3 when any was unreachable, else 0.
pub fn check_exit_code(results: &[PinResult]) -> u8 {
    let mut unreachable = false;
    for result in results {
        match result.outcome {
            PinOutcome::Match => {}
            PinOutcome::Unreachable(_) => unreachable = true,
            PinOutcome::Mismatch { .. } | PinOutcome::NotFound(_) | PinOutcome::Other(_) => {
                return 1
            }
        }
    }
    if unreachable {
        3
    } else {
        0
    }
}
