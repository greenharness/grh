//! Embeds the pinned manifest copies: writes `OUT_DIR/manifests.rs`, the
//! sorted `(name, bytes)` of every `manifests/*.toml`. A missing `manifests/`
//! directory holds no copies.

use std::fmt::Write as _;
use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=manifests");
    let root = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let mut copies: Vec<(String, PathBuf)> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(root.join("manifests")) {
        for entry in entries {
            let path = entry.expect("a readable manifests/ entry").path();
            if path.extension().is_some_and(|x| x == "toml") && path.is_file() {
                let name = path.file_stem().expect("a file stem").to_string_lossy();
                copies.push((name.into_owned(), path));
            }
        }
    }
    copies.sort();
    let mut out = String::from("pub const MANIFESTS: &[(&str, &[u8])] = &[\n");
    for (name, path) in &copies {
        let path = path.to_string_lossy();
        writeln!(out, "    ({name:?}, &include_bytes!({path:?})[..]),").expect("write to a String");
    }
    out.push_str("];\n");
    let target = PathBuf::from(std::env::var_os("OUT_DIR").expect("OUT_DIR")).join("manifests.rs");
    std::fs::write(target, out).expect("write manifests.rs");
}
