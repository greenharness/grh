//! `grh <part> [args…]`: run the part's own binary in grh's place.

use std::ffi::OsString;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::catalog::Catalog;
use crate::exit::Exit;

/// Where grh puts the binaries it downloads: `$GRH_BIN_DIR`, else
/// `~/.local/bin`.
pub fn bin_dir() -> Option<PathBuf> {
    if let Some(d) = std::env::var_os("GRH_BIN_DIR").filter(|d| !d.is_empty()) {
        return Some(PathBuf::from(d));
    }
    std::env::var_os("HOME").map(|h| Path::new(&h).join(".local/bin"))
}

/// The first executable called `name` on PATH, then in grh's bin dir.
pub fn find(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    std::env::split_paths(&path)
        .chain(bin_dir())
        .map(|d| d.join(name))
        .find(|p| is_executable(p))
}

fn is_executable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(p)
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

/// Replace grh with the part's binary, so its exit status and signals are
/// the caller's. Returns only when the part cannot be run.
pub fn run(catalog: &Catalog, argv: &[OsString]) -> Exit {
    let Some((name, args)) = argv.split_first() else {
        return Exit::Usage("name a part or a command; see grh --help".into());
    };
    let name = name.to_string_lossy();
    let Some(part) = catalog.part(&name) else {
        return Exit::Usage(format!("unknown command or part: {name}; see grh --help"));
    };
    let Some(bin) = part.bin.as_deref() else {
        return Exit::Usage(format!("{name} has no command of its own"));
    };
    let Some(path) = find(bin) else {
        return Exit::NotInstalled(format!("{name} is not installed: grh install {name}"));
    };
    let err = Command::new(&path).args(args).exec();
    Exit::Failed(format!("cannot run {}: {err}", path.display()))
}
