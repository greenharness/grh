//! Download a part's release tarball, verify it against the sha256 pinned in
//! the catalog, and only then unpack the named binary into the bin dir.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

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

/// Attempts per route, the first included.
const ATTEMPTS: usize = 3;
const API_DEFAULT: &str = "https://api.github.com";
const REDIRECT_LIMIT: usize = 5;

type Cache = Mutex<HashMap<String, Vec<u8>>>;

/// How a download is routed; the token is never printed (no `Debug`).
struct Routes {
    token: Option<String>,
    api_override: Option<String>,
    backoff: [Duration; ATTEMPTS - 1],
    max_bytes: u64,
    notice: Box<dyn Fn(&str) + Send + Sync>,
}

impl Routes {
    fn from_env() -> Self {
        let non_empty = |name: &str| std::env::var(name).ok().filter(|v| !v.is_empty());
        Routes {
            token: non_empty("GH_TOKEN").or_else(|| non_empty("GITHUB_TOKEN")),
            api_override: non_empty("GRH_API_BASE"),
            backoff: [Duration::from_secs(1), Duration::from_secs(4)],
            max_bytes: MAX_BYTES,
            notice: Box::new(|line| eprintln!("{line}")),
        }
    }

    fn api_for(&self, base: &str) -> Option<String> {
        match &self.api_override {
            Some(api) => Some(api.trim_end_matches('/').to_string()),
            None if base == DEFAULT_BASE => Some(API_DEFAULT.to_string()),
            None => None,
        }
    }

    fn say(&self, line: &str) {
        (self.notice)(line);
    }

    fn say_switch(&self, url: &str, failed: &str, next: &str) {
        self.say(&format!(
            "grh: {url}: {failed} route failed, trying the {next} route"
        ));
    }
}

/// One failed request. Only a 5xx, a 429 and a transient failure are retried.
enum Hit {
    Status(u16),
    Transient(String),
    Fatal(String),
}

impl Hit {
    fn retryable(&self) -> bool {
        match self {
            Hit::Status(code) => *code >= 500 || *code == 429,
            Hit::Transient(_) => true,
            Hit::Fatal(_) => false,
        }
    }
}

impl std::fmt::Display for Hit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Hit::Status(code) => write!(f, "HTTP {code}"),
            Hit::Transient(text) | Hit::Fatal(text) => f.write_str(text),
        }
    }
}

/// A route that gave up: its last failure, how many attempts it made, and the
/// status that rejected the token when the anonymous retry failed too.
struct RouteFailure {
    hit: Hit,
    attempts: usize,
    token_rejected: Option<u16>,
}

impl RouteFailure {
    fn describe(&self) -> String {
        let plural = if self.attempts == 1 { "" } else { "s" };
        let last = format!("{} after {} attempt{plural}", self.hit, self.attempts);
        match self.token_rejected {
            Some(code) => format!("token rejected (HTTP {code}), anonymous: {last}"),
            None => last,
        }
    }
}

struct ReleaseUrl<'a> {
    base: &'a str,
    owner: &'a str,
    repo: &'a str,
    tag: &'a str,
    name: &'a str,
}

fn parse_release_url(url: &str) -> Option<ReleaseUrl<'_>> {
    let (head, tail) = url.split_once("/releases/download/")?;
    let (tag, name) = tail.split_once('/')?;
    let (rest, repo) = head.rsplit_once('/')?;
    let (base, owner) = rest.rsplit_once('/')?;
    let valid = base.starts_with("http") && [tag, name, repo, owner].iter().all(|p| !p.is_empty());
    (valid && !name.contains('/')).then_some(ReleaseUrl {
        base,
        owner,
        repo,
        tag,
        name,
    })
}

fn hit_from(error: ureq::Error) -> Hit {
    match error {
        ureq::Error::Status(code, _) => Hit::Status(code),
        ureq::Error::Transport(transport) => Hit::Transient(match transport.message() {
            Some(message) => format!("{}: {message}", transport.kind()),
            None => transport.kind().to_string(),
        }),
    }
}

fn io_hit(error: &std::io::Error) -> Hit {
    Hit::Transient(format!("read failed: {}", error.kind()))
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .redirects(0)
        .timeout_connect(Duration::from_secs(15))
        .timeout(Duration::from_secs(300))
        .build()
}

fn get(
    agent: &ureq::Agent,
    url: &str,
    accept: &str,
    bearer: Option<&str>,
) -> Result<ureq::Response, Hit> {
    let mut request = agent.get(url).set("Accept", accept);
    if let Some(token) = bearer {
        request = request.set("Authorization", &format!("Bearer {token}"));
    }
    request.call().map_err(hit_from)
}

fn read_limited(resp: ureq::Response, max_bytes: u64) -> Result<Vec<u8>, Hit> {
    let mut buf = Vec::new();
    resp.into_reader()
        .take(max_bytes + 1)
        .read_to_end(&mut buf)
        .map_err(|e| io_hit(&e))?;
    if buf.len() as u64 > max_bytes {
        return Err(Hit::Fatal(format!("larger than {max_bytes} bytes")));
    }
    Ok(buf)
}

/// Resolve a `Location` against the URL that sent it: an absolute URL stays,
/// `//host/…` takes that URL's scheme, `/path` its scheme and host. Anything
/// else (`../x`, a bare name) is refused rather than guessed.
fn resolve_location(from: &str, location: &str) -> Result<String, Hit> {
    let Some(scheme_end) = from.find("://") else {
        return Err(Hit::Fatal(format!("cannot resolve Location {location}")));
    };
    // An RFC 3986 scheme: a letter, then letters, digits, `+`, `-` or `.`.
    let is_absolute = location.split_once("://").is_some_and(|(scheme, _)| {
        scheme.starts_with(|c: char| c.is_ascii_alphabetic())
            && scheme
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
    });
    if is_absolute {
        return Ok(location.to_string());
    }
    if location.starts_with("//") {
        return Ok(format!("{}:{location}", &from[..scheme_end]));
    }
    if location.starts_with('/') {
        let host_start = scheme_end + 3;
        let origin_end = from[host_start..]
            .find('/')
            .map_or(from.len(), |path_start| host_start + path_start);
        return Ok(format!("{}{location}", &from[..origin_end]));
    }
    Err(Hit::Fatal(format!(
        "relative Location {location} not followed"
    )))
}

/// Follow redirects by hand so `bearer` only ever goes to the first host.
fn get_following(
    agent: &ureq::Agent,
    first: &str,
    accept: &str,
    bearer: Option<&str>,
    max_bytes: u64,
) -> Result<Vec<u8>, Hit> {
    let mut url = first.to_string();
    let mut token = bearer;
    for _ in 0..REDIRECT_LIMIT {
        let resp = get(agent, &url, accept, token)?;
        if (300..400).contains(&resp.status()) {
            let location = resp
                .header("location")
                .ok_or_else(|| Hit::Fatal(format!("HTTP {} without a location", resp.status())))?;
            url = resolve_location(&url, location)?;
            token = None;
            continue;
        }
        return read_limited(resp, max_bytes);
    }
    Err(Hit::Fatal("too many redirects".to_string()))
}

fn api_once(
    agent: &ureq::Agent,
    api: &str,
    release: &ReleaseUrl<'_>,
    bearer: Option<&str>,
    max_bytes: u64,
) -> Result<Vec<u8>, Hit> {
    let ReleaseUrl {
        owner,
        repo,
        tag,
        name,
        ..
    } = release;
    let listing = get_following(
        agent,
        &format!("{api}/repos/{owner}/{repo}/releases/tags/{tag}"),
        "application/vnd.github+json",
        bearer,
        max_bytes,
    )?;
    let json: serde_json::Value = serde_json::from_slice(&listing)
        .map_err(|_| Hit::Fatal("unreadable release listing".to_string()))?;
    let id = json["assets"]
        .as_array()
        .and_then(|assets| assets.iter().find(|a| a["name"] == *name))
        .and_then(|asset| asset["id"].as_u64())
        .ok_or_else(|| Hit::Fatal(format!("no asset {name} in release {tag}")))?;
    get_following(
        agent,
        &format!("{api}/repos/{owner}/{repo}/releases/assets/{id}"),
        "application/octet-stream",
        bearer,
        max_bytes,
    )
}

/// Up to `ATTEMPTS` tries of `once`, backing off between transient failures.
fn with_retries(
    routes: &Routes,
    url: &str,
    mut once: impl FnMut() -> Result<Vec<u8>, Hit>,
) -> Result<Vec<u8>, RouteFailure> {
    let mut attempts = 0;
    loop {
        attempts += 1;
        match once() {
            Ok(bytes) => return Ok(bytes),
            Err(hit) if hit.retryable() && attempts < ATTEMPTS => {
                let delay = routes.backoff[attempts - 1];
                routes.say(&format!(
                    "grh: {url}: {hit}, attempt {}/{ATTEMPTS} in {} s",
                    attempts + 1,
                    delay.as_secs()
                ));
                std::thread::sleep(delay);
            }
            Err(hit) => {
                return Err(RouteFailure {
                    hit,
                    attempts,
                    token_rejected: None,
                })
            }
        }
    }
}

fn web_route(agent: &ureq::Agent, url: &str, routes: &Routes) -> Result<Vec<u8>, RouteFailure> {
    with_retries(routes, url, || {
        get_following(agent, url, "*/*", None, routes.max_bytes)
    })
}

fn api_route(
    agent: &ureq::Agent,
    url: &str,
    api: &str,
    release: &ReleaseUrl<'_>,
    routes: &Routes,
) -> Result<Vec<u8>, RouteFailure> {
    let with_token = routes.token.as_deref();
    let first = with_retries(routes, url, || {
        api_once(agent, api, release, with_token, routes.max_bytes)
    });
    match first {
        Err(failure) if with_token.is_some() => {
            let rejection = match failure.hit {
                Hit::Status(code @ (401 | 403)) => Some(code),
                Hit::Status(_) | Hit::Transient(_) | Hit::Fatal(_) => None,
            };
            let Some(code) = rejection else {
                return Err(failure);
            };
            with_retries(routes, url, || {
                api_once(agent, api, release, None, routes.max_bytes)
            })
            .map_err(|anonymous| RouteFailure {
                token_rejected: Some(code),
                ..anonymous
            })
        }
        other => other,
    }
}

fn download_with(url: &str, routes: &Routes, cache: &Cache) -> Result<Vec<u8>, Exit> {
    if let Some(path) = url.strip_prefix("file://") {
        return std::fs::read(path).map_err(|e| Exit::Failed(format!("cannot read {url}: {e}")));
    }
    if let Some(bytes) = cache.lock().ok().and_then(|c| c.get(url).cloned()) {
        return Ok(bytes);
    }
    let agent = agent();
    let release = parse_release_url(url);
    let api = release.as_ref().and_then(|r| routes.api_for(r.base));
    let fail = |detail: String| Exit::Failed(format!("cannot download {url} ({detail})"));
    let outcome = match (&release, &api) {
        (Some(release), Some(api)) if routes.token.is_some() => {
            api_route(&agent, url, api, release, routes).or_else(|api_failure| {
                routes.say_switch(url, "API", "web");
                web_route(&agent, url, routes).map_err(|web_failure| {
                    fail(format!(
                        "API: {}; web fallback: {}",
                        api_failure.describe(),
                        web_failure.describe()
                    ))
                })
            })
        }
        (Some(release), Some(api)) => web_route(&agent, url, routes).or_else(|web_failure| {
            routes.say_switch(url, "web", "API");
            api_route(&agent, url, api, release, routes).map_err(|api_failure| {
                fail(format!(
                    "{}; API fallback: {}",
                    web_failure.describe(),
                    api_failure.describe()
                ))
            })
        }),
        _ => web_route(&agent, url, routes).map_err(|failure| fail(failure.describe())),
    };
    let bytes = outcome?;
    if let Ok(mut entries) = cache.lock() {
        entries.insert(url.to_string(), bytes.clone());
    }
    Ok(bytes)
}

/// Fetch `url` (`http(s)://` or `file://`) into memory; a GitHub release
/// URL is retried and falls back between the web and API routes.
pub fn download(url: &str) -> Result<Vec<u8>, Exit> {
    download_with(url, &Routes::from_env(), process_cache())
}

/// The one cache of a grh process: `download` is its only entry point, so
/// a tarball several parts share is fetched once per run.
fn process_cache() -> &'static Cache {
    static CACHE: std::sync::OnceLock<Cache> = std::sync::OnceLock::new();
    CACHE.get_or_init(Cache::default)
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
        let outcome = install_binary_from(
            &base,
            "o/r",
            "aval",
            "1.0.0",
            "x86_64-unknown-linux-gnu",
            &"0".repeat(64),
            &dir,
        );
        let Err(Exit::Failed(m)) = outcome else {
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
        let outcome = install_binary_from(
            &base,
            "o/r",
            "aval",
            "1.0.0",
            "x86_64-unknown-linux-gnu",
            &sha256_hex(&tgz),
            &dir,
        );
        assert!(matches!(outcome, Err(Exit::Failed(_))));
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
        let url = format!("http://127.0.0.1:{port}/x");
        let got = download(&url).unwrap();
        h.join().unwrap();
        assert_eq!(got, body);
        // The server accepted one connection and is gone: a second
        // `download` of the same URL is served by the process cache.
        assert_eq!(download(&url).unwrap(), body);
    }

    type Reply = (u16, Vec<(String, String)>, Vec<u8>);

    /// A local server answering from `handler(path, head)`; returns its port
    /// and the heads it received.
    fn spawn(
        handler: impl Fn(&str, &str) -> Reply + Send + 'static,
    ) -> (u16, std::sync::Arc<Mutex<Vec<String>>>) {
        use std::io::Write;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let seen = std::sync::Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut s) = stream else { break };
                let mut head = Vec::new();
                let mut byte = [0u8; 1];
                while !head.ends_with(b"\r\n\r\n") && s.read(&mut byte).unwrap_or(0) == 1 {
                    head.push(byte[0]);
                }
                let head = String::from_utf8_lossy(&head).into_owned();
                let path = head.split_whitespace().nth(1).unwrap_or("").to_string();
                log.lock().unwrap().push(head.clone());
                let (status, headers, body) = handler(&path, &head);
                let mut out = format!(
                    "HTTP/1.1 {status} X\r\nContent-Length: {}\r\nConnection: close\r\n",
                    body.len()
                );
                for (k, v) in headers {
                    out.push_str(&format!("{k}: {v}\r\n"));
                }
                out.push_str("\r\n");
                let _ = s.write_all(out.as_bytes()); // the client may have hung up
                let _ = s.write_all(&body); // the client may have hung up
            }
        });
        (port, seen)
    }

    fn routes(token: Option<&str>, port: u16) -> Routes {
        Routes {
            token: token.map(str::to_string),
            api_override: Some(format!("http://127.0.0.1:{port}/api")),
            backoff: [Duration::from_millis(1); ATTEMPTS - 1],
            max_bytes: MAX_BYTES,
            notice: Box::new(|_| {}),
        }
    }

    fn capturing_routes(
        token: Option<&str>,
        port: u16,
    ) -> (Routes, std::sync::Arc<Mutex<Vec<String>>>) {
        let lines = std::sync::Arc::new(Mutex::new(Vec::new()));
        let sink = lines.clone();
        let mut captured = routes(token, port);
        captured.notice = Box::new(move |line| sink.lock().unwrap().push(line.to_string()));
        (captured, lines)
    }

    fn fetch(url: &str, routes: &Routes) -> Result<Vec<u8>, Exit> {
        download_with(url, routes, &Cache::default())
    }

    fn listing() -> Reply {
        (
            200,
            vec![],
            br#"{"assets":[{"name":"x.tgz","id":7}]}"#.to_vec(),
        )
    }

    fn count(seen: &Mutex<Vec<String>>, needle: &str) -> usize {
        seen.lock()
            .unwrap()
            .iter()
            .filter(|h| h.contains(needle))
            .count()
    }

    #[test]
    fn a_failing_web_route_falls_back_to_the_api() {
        let (port, seen) = spawn(|path, _| match path {
            "/api/repos/o/r/releases/tags/v1" => listing(),
            "/api/repos/o/r/releases/assets/7" => (200, vec![], b"BYTES".to_vec()),
            _ => (504, vec![], vec![]),
        });
        let url = format!("http://127.0.0.1:{port}/o/r/releases/download/v1/x.tgz");
        let got = fetch(&url, &routes(None, port)).unwrap();
        assert_eq!(got, b"BYTES");
        assert_eq!(count(&seen, "GET /o/r/releases/download"), 3);
    }

    #[test]
    fn a_404_is_requested_once_per_route_and_the_error_names_the_url_once() {
        let (port, seen) = spawn(|_, _| (404, vec![], vec![]));
        let url = format!("http://127.0.0.1:{port}/o/r/releases/download/v1/y.tgz");
        let Err(Exit::Failed(m)) = fetch(&url, &routes(None, port)) else {
            panic!("expected a failure");
        };
        assert_eq!(count(&seen, "GET /o/r/releases/download"), 1);
        assert_eq!(count(&seen, "GET /api/repos/o/r/releases/tags/v1"), 1);
        assert_eq!(m.matches(&url).count(), 1, "{m}");
        assert_eq!(
            m,
            format!(
                "cannot download {url} (HTTP 404 after 1 attempt; API fallback: HTTP 404 after 1 attempt)"
            )
        );
    }

    #[test]
    fn exhausted_retries_are_reported() {
        let (port, _) = spawn(|_, _| (504, vec![], vec![]));
        let url = format!("http://127.0.0.1:{port}/o/r/releases/download/v1/z.tgz");
        let Err(Exit::Failed(m)) = fetch(&url, &routes(None, port)) else {
            panic!("expected a failure");
        };
        assert!(
            m.ends_with("(HTTP 504 after 3 attempts; API fallback: HTTP 504 after 3 attempts)"),
            "{m}"
        );
    }

    #[test]
    fn the_token_goes_to_the_api_host_only() {
        let (blob_port, blob_seen) = spawn(|_, _| (200, vec![], b"BLOB".to_vec()));
        let (port, seen) = spawn(move |path, _| match path {
            "/api/repos/o/r/releases/tags/v1" => listing(),
            "/api/repos/o/r/releases/assets/7" => (
                302,
                vec![(
                    "Location".to_string(),
                    format!("http://127.0.0.1:{blob_port}/blob"),
                )],
                vec![],
            ),
            _ => (504, vec![], vec![]),
        });
        let url = format!("http://127.0.0.1:{port}/o/r/releases/download/v1/x.tgz");
        let got = fetch(&url, &routes(Some("sekret"), port)).unwrap();
        assert_eq!(got, b"BLOB");
        let first = seen.lock().unwrap()[0].clone();
        assert!(
            first.contains("GET /api/repos/o/r/releases/tags/v1"),
            "{first}"
        );
        assert!(first
            .to_lowercase()
            .contains("authorization: bearer sekret"));
        for head in blob_seen.lock().unwrap().iter() {
            assert!(!head.to_lowercase().contains("authorization"), "{head}");
        }
    }

    #[test]
    fn a_401_with_a_token_retries_the_api_anonymously() {
        let (port, seen) = spawn(|path, head| {
            let authed = head.to_lowercase().contains("authorization");
            match path {
                "/api/repos/o/r/releases/tags/v1" if authed => (401, vec![], vec![]),
                "/api/repos/o/r/releases/tags/v1" => listing(),
                "/api/repos/o/r/releases/assets/7" => (200, vec![], b"ANON".to_vec()),
                _ => (504, vec![], vec![]),
            }
        });
        let url = format!("http://127.0.0.1:{port}/o/r/releases/download/v1/x.tgz");
        let got = fetch(&url, &routes(Some("sekret"), port)).unwrap();
        assert_eq!(got, b"ANON");
        assert_eq!(count(&seen, "GET /api/repos/o/r/releases/tags/v1"), 2);
    }

    #[test]
    fn the_same_url_is_fetched_once_per_process() {
        let (port, seen) = spawn(|_, _| (200, vec![], b"ONCE".to_vec()));
        let url = format!("http://127.0.0.1:{port}/plain/file.bin");
        let plain_routes = routes(None, port);
        let cache = Cache::default();
        assert_eq!(download_with(&url, &plain_routes, &cache).unwrap(), b"ONCE");
        assert_eq!(download_with(&url, &plain_routes, &cache).unwrap(), b"ONCE");
        assert_eq!(seen.lock().unwrap().len(), 1);
    }

    #[test]
    fn a_429_on_the_web_route_is_retried_and_reported_on_stderr() {
        let hits = std::sync::atomic::AtomicUsize::new(0);
        let (port, seen) = spawn(move |_, _| {
            if hits.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
                (429, vec![], vec![])
            } else {
                (200, vec![], b"AFTER".to_vec())
            }
        });
        let url = format!("http://127.0.0.1:{port}/o/r/releases/download/v1/x.tgz");
        let (routes, lines) = capturing_routes(None, port);
        assert_eq!(fetch(&url, &routes).unwrap(), b"AFTER");
        assert_eq!(seen.lock().unwrap().len(), 2);
        assert_eq!(
            *lines.lock().unwrap(),
            vec![format!("grh: {url}: HTTP 429, attempt 2/3 in 0 s")]
        );
    }

    #[test]
    fn a_dropped_connection_is_transient_and_retried() {
        use std::io::Write;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        listener.set_nonblocking(true).unwrap();
        // A deadline on each accept: a client that never reconnects fails the
        // test instead of hanging it.
        let accept = move |listener: &std::net::TcpListener| {
            let deadline = std::time::Instant::now() + Duration::from_secs(10);
            loop {
                match listener.accept() {
                    Ok((stream, _)) => {
                        stream.set_nonblocking(false).unwrap();
                        return stream;
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(
                            std::time::Instant::now() < deadline,
                            "no connection within 10 s"
                        );
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Err(e) => panic!("accept failed: {e}"),
                }
            }
        };
        let server = std::thread::spawn(move || {
            drop(accept(&listener));
            let mut answered = accept(&listener);
            let mut head = Vec::new();
            let mut byte = [0u8; 1];
            while !head.ends_with(b"\r\n\r\n") && answered.read(&mut byte).unwrap_or(0) == 1 {
                head.push(byte[0]);
            }
            answered
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\nConnection: close\r\n\r\nBACK")
                .unwrap();
        });
        let url = format!("http://127.0.0.1:{port}/plain/file.bin");
        let (routes, lines) = capturing_routes(None, port);
        assert_eq!(fetch(&url, &routes).unwrap(), b"BACK");
        server.join().unwrap();
        assert_eq!(lines.lock().unwrap().len(), 1);
        assert!(lines.lock().unwrap()[0].contains("attempt 2/3"));
    }

    #[test]
    fn a_location_resolves_or_is_refused() {
        let from = "http://h:1/a/b";
        assert_eq!(
            resolve_location(from, "https://x/y").ok().as_deref(),
            Some("https://x/y")
        );
        assert_eq!(
            resolve_location(from, "//cdn/p").ok().as_deref(),
            Some("http://cdn/p")
        );
        assert_eq!(
            resolve_location(from, "/p?q=1").ok().as_deref(),
            Some("http://h:1/p?q=1")
        );
        assert_eq!(
            resolve_location("http://h:1", "/p").ok().as_deref(),
            Some("http://h:1/p")
        );
        for relative in ["../x", "final", "p/q?u=http://z", "final?u=http://z"] {
            let refused = resolve_location(from, relative);
            assert!(matches!(refused, Err(Hit::Fatal(_))), "{relative}");
        }
    }

    #[test]
    fn a_403_with_a_token_retries_the_api_anonymously() {
        let (port, seen) = spawn(|path, head| {
            let authed = head.to_lowercase().contains("authorization");
            match path {
                "/api/repos/o/r/releases/tags/v1" if authed => (403, vec![], vec![]),
                "/api/repos/o/r/releases/tags/v1" => listing(),
                "/api/repos/o/r/releases/assets/7" => (200, vec![], b"ANON".to_vec()),
                _ => (504, vec![], vec![]),
            }
        });
        let url = format!("http://127.0.0.1:{port}/o/r/releases/download/v1/x.tgz");
        assert_eq!(fetch(&url, &routes(Some("sekret"), port)).unwrap(), b"ANON");
        assert_eq!(count(&seen, "GET /api/repos/o/r/releases/tags/v1"), 2);
    }

    #[test]
    fn a_rejected_token_stays_visible_when_the_anonymous_attempt_fails() {
        let (port, _) = spawn(|path, head| {
            let authed = head.to_lowercase().contains("authorization");
            match path {
                "/api/repos/o/r/releases/tags/v1" if authed => (401, vec![], vec![]),
                _ => (504, vec![], vec![]),
            }
        });
        let url = format!("http://127.0.0.1:{port}/o/r/releases/download/v1/x.tgz");
        let (routes, lines) = capturing_routes(Some("sekret"), port);
        let Err(Exit::Failed(message)) = fetch(&url, &routes) else {
            panic!("expected a failure");
        };
        assert!(
            message
                .contains("API: token rejected (HTTP 401), anonymous: HTTP 504 after 3 attempts"),
            "{message}"
        );
        assert!(!message.contains("sekret"));
        assert!(lines
            .lock()
            .unwrap()
            .iter()
            .all(|line| !line.contains("sekret")));
        assert!(lines.lock().unwrap().contains(&format!(
            "grh: {url}: API route failed, trying the web route"
        )));
    }

    #[test]
    fn a_relative_location_is_resolved_against_the_sending_host() {
        let (port, _) = spawn(|path, _| match path {
            "/start" => (
                302,
                vec![("Location".to_string(), "/final".to_string())],
                vec![],
            ),
            "/final" => (200, vec![], b"LANDED".to_vec()),
            _ => (404, vec![], vec![]),
        });
        let url = format!("http://127.0.0.1:{port}/start");
        assert_eq!(fetch(&url, &routes(None, port)).unwrap(), b"LANDED");
    }

    #[test]
    fn resolving_a_location_keeps_absolute_urls_and_joins_paths() {
        let resolved = |from, location| resolve_location(from, location).ok();
        assert_eq!(
            resolved("http://h:1/a/b?q=1", "/c").as_deref(),
            Some("http://h:1/c")
        );
        assert_eq!(
            resolved("http://h:1", "/c").as_deref(),
            Some("http://h:1/c")
        );
        assert_eq!(
            resolved("http://h/a", "https://o/z").as_deref(),
            Some("https://o/z")
        );
    }

    #[test]
    fn an_oversized_body_is_fatal_and_requested_once_per_route() {
        let (port, seen) = spawn(|_, _| (200, vec![], vec![b'x'; 64]));
        let url = format!("http://127.0.0.1:{port}/o/r/releases/download/v1/x.tgz");
        let mut small = routes(None, port);
        small.max_bytes = 16;
        let Err(Exit::Failed(message)) = fetch(&url, &small) else {
            panic!("expected a failure");
        };
        assert_eq!(count(&seen, "GET /o/r/releases/download"), 1);
        assert_eq!(count(&seen, "GET /api/repos/o/r/releases/tags/v1"), 1);
        assert!(
            message.contains("larger than 16 bytes after 1 attempt"),
            "{message}"
        );
    }

    #[test]
    fn a_missing_asset_is_named_and_not_retried() {
        let (port, seen) = spawn(|path, _| match path {
            "/api/repos/o/r/releases/tags/v1" => listing(),
            _ => (404, vec![], vec![]),
        });
        let url = format!("http://127.0.0.1:{port}/o/r/releases/download/v1/other.tgz");
        let Err(Exit::Failed(message)) = fetch(&url, &routes(None, port)) else {
            panic!("expected a failure");
        };
        assert!(
            message.contains("no asset other.tgz in release v1 after 1 attempt"),
            "{message}"
        );
        assert_eq!(count(&seen, "GET /api/repos/o/r/releases/tags/v1"), 1);
    }
}
