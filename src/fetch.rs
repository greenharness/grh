//! Download a part's release tarball, verify it against the sha256 pinned in
//! the catalog, and only then unpack the named binary into the bin dir.

use std::io::Read;
use std::path::{Path, PathBuf};

use flate2::read::GzDecoder;
use sha2::{Digest, Sha256};

use crate::exit::Exit;
use crate::receipt::write_atomic;

const DEFAULT_BASE: &str = "https://github.com";
/// Upper bound on a download, so a bad server cannot fill memory.
const MAX_BYTES: u64 = 512 * 1024 * 1024;

/// A binary grh placed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placed {
    pub path: PathBuf,
    pub sha256: String,
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// The release host: `$GRH_RELEASE_BASE` (tests), else GitHub.
pub fn release_base() -> String {
    std::env::var("GRH_RELEASE_BASE")
        .ok()
        .filter(|b| !b.is_empty())
        .unwrap_or_else(|| DEFAULT_BASE.to_string())
}

/// The target triple for an OS and architecture, as named in release assets.
pub fn target_for(os: &str, arch: &str) -> Result<&'static str, Exit> {
    match (os, arch) {
        ("macos", "aarch64") => Ok("aarch64-apple-darwin"),
        ("macos", "x86_64") => Ok("x86_64-apple-darwin"),
        ("linux", "aarch64") => Ok("aarch64-unknown-linux-gnu"),
        ("linux", "x86_64") => Ok("x86_64-unknown-linux-gnu"),
        _ => Err(Exit::Failed(format!(
            "no prebuilt release for {arch} on {os}: build from source with `cargo install` instead"
        ))),
    }
}

/// The target triple of this machine.
pub fn target() -> Result<&'static str, Exit> {
    target_for(std::env::consts::OS, std::env::consts::ARCH)
}

pub fn binary_url(base: &str, repo: &str, bin: &str, version: &str, target: &str) -> String {
    format!(
        "{}/{repo}/releases/download/v{version}/{bin}-{version}-{target}.tar.gz",
        base.trim_end_matches('/')
    )
}

/// Fetch `url` (`http(s)://` or `file://`) into memory.
pub fn download(url: &str) -> Result<Vec<u8>, Exit> {
    if let Some(path) = url.strip_prefix("file://") {
        return std::fs::read(path).map_err(|e| Exit::Failed(format!("cannot read {url}: {e}")));
    }
    let resp = ureq::get(url)
        .call()
        .map_err(|e| Exit::Failed(format!("cannot download {url}: {e}")))?;
    let mut buf = Vec::new();
    resp.into_reader()
        .take(MAX_BYTES + 1)
        .read_to_end(&mut buf)
        .map_err(|e| Exit::Failed(format!("cannot download {url}: {e}")))?;
    if buf.len() as u64 > MAX_BYTES {
        return Err(Exit::Failed(format!(
            "{url} is larger than {MAX_BYTES} bytes"
        )));
    }
    Ok(buf)
}

/// Download `url` and check it against `pin`; nothing is returned on a mismatch.
pub fn download_verified(url: &str, pin: &str) -> Result<Vec<u8>, Exit> {
    let bytes = download(url)?;
    let got = sha256_hex(&bytes);
    if !got.eq_ignore_ascii_case(pin.trim()) {
        return Err(Exit::Failed(format!(
            "{url} does not match its pinned sha256 (expected {pin}, got {got}); nothing was written"
        )));
    }
    Ok(bytes)
}

/// Every regular file of a gzip'd tar as (path, bytes, mode).
pub fn read_tarball(bytes: &[u8]) -> Result<Vec<(String, Vec<u8>, u32)>, Exit> {
    let bad = |e: std::io::Error| Exit::Failed(format!("cannot read the tarball: {e}"));
    let mut out = Vec::new();
    let mut ar = tar::Archive::new(GzDecoder::new(bytes));
    for entry in ar.entries().map_err(bad)? {
        let mut entry = entry.map_err(bad)?;
        if !entry.header().entry_type().is_file() {
            continue;
        }
        let path = entry.path().map_err(bad)?.to_string_lossy().into_owned();
        let mode = entry.header().mode().unwrap_or(0o644);
        let mut data = Vec::new();
        entry.read_to_end(&mut data).map_err(bad)?;
        out.push((path, data, mode));
    }
    Ok(out)
}

/// Fetch, verify and place one binary: `base` is the release host.
#[allow(clippy::too_many_arguments)]
pub fn install_binary_from(
    base: &str,
    repo: &str,
    bin: &str,
    version: &str,
    target: &str,
    pin: &str,
    dir: &Path,
) -> Result<Placed, Exit> {
    let url = binary_url(base, repo, bin, version, target);
    let tarball = download_verified(&url, pin)?;
    let data = read_tarball(&tarball)?
        .into_iter()
        .find(|(p, _, _)| Path::new(p).file_name().is_some_and(|n| n == bin))
        .map(|(_, d, _)| d)
        .ok_or_else(|| Exit::Failed(format!("{url} holds no file named {bin}")))?;
    let path = dir.join(bin);
    write_atomic(&path, &data, 0o755)?;
    Ok(Placed {
        path,
        sha256: sha256_hex(&data),
    })
}

/// Fetch a part's binary for this machine into `dir` (see `dispatch::bin_dir`).
pub fn install_binary(
    repo: &str,
    bin: &str,
    version: &str,
    pins: &std::collections::BTreeMap<String, String>,
    dir: &Path,
) -> Result<Placed, Exit> {
    let target = target()?;
    let pin = pins
        .get(target)
        .ok_or_else(|| Exit::Failed(format!("{bin} {version} pins no sha256 for {target}")))?;
    install_binary_from(&release_base(), repo, bin, version, target, pin, dir)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    pub fn tarball(files: &[(&str, &[u8])]) -> Vec<u8> {
        let gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        let mut b = tar::Builder::new(gz);
        for (name, data) in files {
            let mut h = tar::Header::new_gnu();
            h.set_size(data.len() as u64);
            h.set_mode(0o644);
            h.set_cksum();
            b.append_data(&mut h, name, *data).unwrap();
        }
        b.into_inner().unwrap().finish().unwrap()
    }

    fn serve(root: &Path, repo: &str, bin: &str, tgz: &[u8]) -> String {
        let d = root.join(format!("{repo}/releases/download/v1.0.0"));
        fs::create_dir_all(&d).unwrap();
        fs::write(
            d.join(format!("{bin}-1.0.0-x86_64-unknown-linux-gnu.tar.gz")),
            tgz,
        )
        .unwrap();
        format!("file://{}", root.display())
    }

    #[test]
    fn target_triples() {
        assert_eq!(
            target_for("macos", "aarch64").unwrap(),
            "aarch64-apple-darwin"
        );
        assert_eq!(
            target_for("macos", "x86_64").unwrap(),
            "x86_64-apple-darwin"
        );
        assert_eq!(
            target_for("linux", "aarch64").unwrap(),
            "aarch64-unknown-linux-gnu"
        );
        assert_eq!(
            target_for("linux", "x86_64").unwrap(),
            "x86_64-unknown-linux-gnu"
        );
        let Err(Exit::Failed(m)) = target_for("windows", "x86_64") else {
            panic!("expected an error");
        };
        assert!(m.contains("cargo install"));
    }

    #[test]
    fn url_shape() {
        assert_eq!(
            binary_url("https://github.com/", "o/r", "aval", "1.2.3", "t"),
            "https://github.com/o/r/releases/download/v1.2.3/aval-1.2.3-t.tar.gz"
        );
    }

    #[test]
    fn a_verified_tarball_unpacks_only_the_named_binary() {
        let tmp = tempfile::tempdir().unwrap();
        let tgz = tarball(&[("aval-1.0.0/README", b"hi"), ("aval-1.0.0/aval", b"BIN")]);
        let base = serve(&tmp.path().join("srv"), "o/r", "aval", &tgz);
        let dir = tmp.path().join("bin");
        let placed = install_binary_from(
            &base,
            "o/r",
            "aval",
            "1.0.0",
            "x86_64-unknown-linux-gnu",
            &sha256_hex(&tgz),
            &dir,
        )
        .unwrap();
        assert_eq!(placed.path, dir.join("aval"));
        assert_eq!(placed.sha256, sha256_hex(b"BIN"));
        assert_eq!(fs::read(dir.join("aval")).unwrap(), b"BIN");
        let mode = fs::metadata(dir.join("aval")).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o755);
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
    }

    #[test]
    fn a_wrong_sha_writes_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let tgz = tarball(&[("aval", b"BIN")]);
        let base = serve(&tmp.path().join("srv"), "o/r", "aval", &tgz);
        let dir = tmp.path().join("bin");
        let r = install_binary_from(
            &base,
            "o/r",
            "aval",
            "1.0.0",
            "x86_64-unknown-linux-gnu",
            &"0".repeat(64),
            &dir,
        );
        let Err(Exit::Failed(m)) = r else {
            panic!("expected a failure");
        };
        assert!(m.contains("pinned sha256"), "{m}");
        assert!(!dir.exists());
    }

    #[test]
    fn a_tarball_without_the_binary_is_an_error() {
        let tmp = tempfile::tempdir().unwrap();
        let tgz = tarball(&[("other", b"x")]);
        let base = serve(&tmp.path().join("srv"), "o/r", "aval", &tgz);
        let dir = tmp.path().join("bin");
        let r = install_binary_from(
            &base,
            "o/r",
            "aval",
            "1.0.0",
            "x86_64-unknown-linux-gnu",
            &sha256_hex(&tgz),
            &dir,
        );
        assert!(matches!(r, Err(Exit::Failed(_))));
        assert!(!dir.join("aval").exists());
    }

    #[test]
    fn downloads_over_http_from_a_local_server() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let body = b"hello".to_vec();
        let b2 = body.clone();
        let h = std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let mut buf = [0u8; 1024];
            let _ = s.read(&mut buf); // the request bytes are not inspected by this test server
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                b2.len()
            );
            s.write_all(head.as_bytes()).unwrap();
            s.write_all(&b2).unwrap();
        });
        let got = download(&format!("http://127.0.0.1:{port}/x")).unwrap();
        h.join().unwrap();
        assert_eq!(got, body);
    }
}
