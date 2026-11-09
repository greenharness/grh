//! Pinned manifests: fetch a release's manifest asset into `manifests/` and
//! re-check every pin of a catalog against its release. The `pins` example is
//! a thin wrapper over this; nothing here prints.

use std::fmt;
use std::path::Path;
use std::time::Duration;

use serde::Deserialize;

use crate::catalog::Pin;
use crate::exit::Exit;
use crate::fetch::{download, download_with_retry_delay, release_base, sha256_hex, FetchFailure};
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
    pin_from(
        &release_base(),
        copies_dir,
        name,
        repo,
        tag,
        asset,
        existing,
    )
}

/// `pin` against the release host `base` instead of `GRH_RELEASE_BASE`.
pub fn pin_from(
    base: &str,
    copies_dir: &Path,
    name: &str,
    repo: &str,
    tag: &str,
    asset: &str,
    existing: Existing,
) -> Result<String, PinError> {
    let usable = !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if !usable {
        return Err(PinError::Invalid(format!(
            "{name:?} is not a part name (letters, digits, - and _)"
        )));
    }
    let url = asset_url(base, repo, tag, asset);
    let bytes = download(&url).map_err(PinError::Download)?;
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
/// order, every pin checked.
pub fn check(catalog_source: &str, copies_dir: &Path) -> Result<Vec<PinResult>, Exit> {
    let base = release_base();
    check_from(|_| base.clone(), catalog_source, copies_dir)
}

/// `check` against the release host `base_of(pin name)` instead of
/// `GRH_RELEASE_BASE`.
pub fn check_from(
    base_of: impl Fn(&str) -> String,
    catalog_source: &str,
    copies_dir: &Path,
) -> Result<Vec<PinResult>, Exit> {
    check_pausing(None, base_of, catalog_source, copies_dir)
}

/// `check_from` waiting `retry_delay` between download attempts instead of
/// 1 s then 4 s: for tests that count retries, not delays.
pub fn check_from_with_retry_delay(
    retry_delay: Duration,
    base_of: impl Fn(&str) -> String,
    catalog_source: &str,
    copies_dir: &Path,
) -> Result<Vec<PinResult>, Exit> {
    check_pausing(Some(retry_delay), base_of, catalog_source, copies_dir)
}

fn check_pausing(
    retry_delay: Option<Duration>,
    base_of: impl Fn(&str) -> String,
    catalog_source: &str,
    copies_dir: &Path,
) -> Result<Vec<PinResult>, Exit> {
    let catalog: CatalogPins =
        toml::from_str(catalog_source).map_err(|e| Exit::Failed(format!("the catalog: {e}")))?;
    Ok(catalog
        .pinned
        .iter()
        .map(|pin| PinResult {
            name: pin.name.clone(),
            outcome: check_one(&base_of(&pin.name), retry_delay, pin, copies_dir),
        })
        .collect())
}

fn check_one(
    base: &str,
    retry_delay: Option<Duration>,
    pin: &Pin,
    copies_dir: &Path,
) -> PinOutcome {
    let url = asset_url(base, &pin.repo, &pin.tag, &pin.asset);
    let fetched = match retry_delay {
        Some(delay) => download_with_retry_delay(&url, delay),
        None => download(&url),
    };
    let bytes = match fetched {
        Ok(bytes) => bytes,
        Err(FetchFailure::NotFound(m)) => return PinOutcome::NotFound(m),
        Err(FetchFailure::Unreachable(m)) => return PinOutcome::Unreachable(m),
        Err(FetchFailure::Other(m)) => return PinOutcome::Other(m),
    };
    let asset = sha256_hex(&bytes);
    let copy = match std::fs::read(copies_dir.join(format!("{}.toml", pin.name))) {
        Ok(copy) => Some(sha256_hex(&copy)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return PinOutcome::Other(format!("cannot read the copy of {}: {e}", pin.name)),
    };
    if asset == pin.sha256 && copy.as_deref() == Some(asset.as_str()) {
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
