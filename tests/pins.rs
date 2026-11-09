//! `grh::pins` against a `file://` fixture release and local servers.

use std::fs;
use std::io::{Read, Write};
use std::path::Path;
use std::process::Command;
use std::sync::Mutex;

use grh::catalog::Catalog;
use grh::fetch::{sha256_hex, FetchFailure};
use grh::pins::{
    check_exit_code, check_from, check_from_with_retry_delay, pin_from, Existing, PinError,
    PinOutcome,
};

const MANIFEST: &[u8] = include_bytes!("fixtures/pinned/manifests/pinned-demo.toml");
const CATALOG_HEAD: &str = "skills_repo = \"greenharness/skills\"\nskills_version = \"0.1.0\"\nskills_sha256 = \"x\"\nforeign_markers = [\"relais:begin\"]\norder = [\"pinned-demo\"]\n\n";

/// Tests that read `GRH_API_BASE` or `GH_TOKEN` (every http one) run one at a time.
static NETWORK: Mutex<()> = Mutex::new(());

fn release(root: &Path, repo: &str, bytes: &[u8]) {
    let dir = root.join(format!("{repo}/releases/download/v1.0.0"));
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("greenharness.toml"), bytes).unwrap();
}

fn file_base(root: &Path) -> String {
    format!("file://{}", root.display())
}

fn one_byte_off(bytes: &[u8]) -> Vec<u8> {
    let mut off = bytes.to_vec();
    off.push(b'\n');
    off
}

fn pin_block(name: &str, repo: &str, bytes: &[u8]) -> String {
    format!(
        "[[pinned]]\nname = \"{name}\"\nrepo = \"{repo}\"\ntag = \"v1.0.0\"\nsha256 = \"{}\"\n",
        sha256_hex(bytes)
    )
}

fn catalog_with(blocks: &[String]) -> String {
    format!("{CATALOG_HEAD}{}", blocks.join("\n"))
}

fn demo_catalog() -> String {
    catalog_with(&[pin_block("pinned-demo", "fixture/pinned-demo", MANIFEST)])
}

fn two_pins() -> String {
    catalog_with(&[
        pin_block("one", "fixture/one", MANIFEST),
        pin_block("two", "fixture/two", MANIFEST),
    ])
}

fn pin_demo(base: &str, copies: &Path, existing: Existing) -> Result<String, PinError> {
    pin_from(
        base,
        copies,
        "pinned-demo",
        "fixture/pinned-demo",
        "v1.0.0",
        "greenharness.toml",
        existing,
    )
}

#[test]
fn pin_writes_the_copy_and_returns_a_block_the_catalog_accepts() {
    let tmp = tempfile::tempdir().unwrap();
    let srv = tmp.path().join("srv");
    release(&srv, "fixture/pinned-demo", MANIFEST);
    let copies = tmp.path().join("manifests");
    let block = pin_demo(&file_base(&srv), &copies, Existing::Keep).unwrap();
    let written = fs::read(copies.join("pinned-demo.toml")).unwrap();
    assert_eq!(written, MANIFEST);
    Catalog::parse_with(
        &catalog_with(&[block]),
        &[("pinned-demo", written.as_slice())],
    )
    .unwrap();
}

#[test]
fn pin_refuses_to_overwrite_a_differing_copy_without_replace() {
    let tmp = tempfile::tempdir().unwrap();
    let srv = tmp.path().join("srv");
    release(&srv, "fixture/pinned-demo", MANIFEST);
    let copies = tmp.path().join("manifests");
    fs::create_dir_all(&copies).unwrap();
    fs::write(copies.join("pinned-demo.toml"), b"other bytes").unwrap();
    let base = file_base(&srv);
    assert!(matches!(
        pin_demo(&base, &copies, Existing::Keep),
        Err(PinError::WouldOverwrite(_))
    ));
    assert_eq!(
        fs::read(copies.join("pinned-demo.toml")).unwrap(),
        b"other bytes"
    );
    pin_demo(&base, &copies, Existing::Replace).unwrap();
    assert_eq!(fs::read(copies.join("pinned-demo.toml")).unwrap(), MANIFEST);
}

#[test]
fn pin_of_a_missing_asset_is_not_found_and_writes_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let srv = tmp.path().join("srv");
    fs::create_dir_all(&srv).unwrap();
    let copies = tmp.path().join("manifests");
    let outcome = pin_demo(&file_base(&srv), &copies, Existing::Keep);
    assert!(
        matches!(outcome, Err(PinError::Download(FetchFailure::NotFound(_)))),
        "{outcome:?}"
    );
    assert!(!copies.exists());
}

#[test]
fn check_passes_on_a_matching_asset() {
    let tmp = tempfile::tempdir().unwrap();
    let srv = tmp.path().join("srv");
    release(&srv, "fixture/pinned-demo", MANIFEST);
    let copies = tmp.path().join("manifests");
    fs::create_dir_all(&copies).unwrap();
    fs::write(copies.join("pinned-demo.toml"), MANIFEST).unwrap();
    let results = check_from(|_| file_base(&srv), &demo_catalog(), &copies).unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].outcome, PinOutcome::Match);
    assert_eq!(check_exit_code(&results), 0);
}

#[test]
fn check_reports_both_parts_when_two_assets_are_one_byte_off() {
    let tmp = tempfile::tempdir().unwrap();
    let srv = tmp.path().join("srv");
    for repo in ["fixture/one", "fixture/two"] {
        release(&srv, repo, &one_byte_off(MANIFEST));
    }
    let copies = tmp.path().join("manifests");
    fs::create_dir_all(&copies).unwrap();
    fs::write(copies.join("one.toml"), MANIFEST).unwrap();
    fs::write(copies.join("two.toml"), MANIFEST).unwrap();
    let results = check_from(|_| file_base(&srv), &two_pins(), &copies).unwrap();
    let names: Vec<_> = results.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(names, ["one", "two"]);
    assert!(results
        .iter()
        .all(|r| matches!(r.outcome, PinOutcome::Mismatch { .. })));
    assert_eq!(check_exit_code(&results), 1);
}

#[test]
fn check_of_a_file_base_without_the_asset_is_not_found() {
    let tmp = tempfile::tempdir().unwrap();
    let srv = tmp.path().join("srv");
    fs::create_dir_all(&srv).unwrap();
    let results = check_from(|_| file_base(&srv), &demo_catalog(), &tmp.path().join("m")).unwrap();
    assert!(
        matches!(results[0].outcome, PinOutcome::NotFound(_)),
        "{results:?}"
    );
    assert_eq!(check_exit_code(&results), 1);
}

/// A local server answering `handler(path)`; `None` drops the connection.
fn serve(handler: impl Fn(&str) -> Option<(u16, Vec<u8>)> + Send + 'static) -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { break };
            let mut head = Vec::new();
            let mut byte = [0u8; 1];
            while !head.ends_with(b"\r\n\r\n") && stream.read(&mut byte).unwrap_or(0) == 1 {
                head.push(byte[0]);
            }
            let head = String::from_utf8_lossy(&head).into_owned();
            let path = head.split_whitespace().nth(1).unwrap_or("").to_string();
            let Some((status, body)) = handler(&path) else {
                continue;
            };
            let reply = format!(
                "HTTP/1.1 {status} X\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(reply.as_bytes()); // the client may have hung up
            let _ = stream.write_all(&body); // the client may have hung up
        }
    });
    port
}

/// Short retry waits: only the retry count is under test, not the delays.
const RETRY_DELAY: std::time::Duration = std::time::Duration::from_millis(1);

fn clear_routing_env() {
    std::env::remove_var("GH_TOKEN");
    std::env::remove_var("GITHUB_TOKEN");
    std::env::remove_var("GRH_API_BASE");
}

#[test]
fn check_reports_unreachable_for_a_port_nobody_listens_on() {
    let _guard = NETWORK.lock().unwrap_or_else(|e| e.into_inner());
    clear_routing_env();
    let tmp = tempfile::tempdir().unwrap();
    let results = check_from_with_retry_delay(
        RETRY_DELAY,
        |_| "http://127.0.0.1:1".to_string(),
        &demo_catalog(),
        &tmp.path().join("m"),
    )
    .unwrap();
    assert!(
        matches!(results[0].outcome, PinOutcome::Unreachable(_)),
        "{results:?}"
    );
    assert_eq!(check_exit_code(&results), 3);
}

#[test]
fn a_server_answering_503_to_every_attempt_is_unreachable() {
    let _guard = NETWORK.lock().unwrap_or_else(|e| e.into_inner());
    clear_routing_env();
    let port = serve(|_| Some((503, Vec::new())));
    let tmp = tempfile::tempdir().unwrap();
    let base = format!("http://127.0.0.1:{port}");
    let results = check_from_with_retry_delay(
        RETRY_DELAY,
        |_| base.clone(),
        &demo_catalog(),
        &tmp.path().join("m"),
    )
    .unwrap();
    assert!(
        matches!(results[0].outcome, PinOutcome::Unreachable(_)),
        "{results:?}"
    );
}

#[test]
fn a_release_listing_without_the_asset_is_not_found() {
    let _guard = NETWORK.lock().unwrap_or_else(|e| e.into_inner());
    clear_routing_env();
    let port = serve(|path| {
        if path.ends_with("/releases/tags/v1.0.0") {
            Some((
                200,
                br#"{"assets":[{"name":"other.toml","id":7}]}"#.to_vec(),
            ))
        } else {
            Some((404, Vec::new()))
        }
    });
    std::env::set_var("GRH_API_BASE", format!("http://127.0.0.1:{port}/api"));
    let tmp = tempfile::tempdir().unwrap();
    let base = format!("http://127.0.0.1:{port}");
    let results = check_from_with_retry_delay(
        RETRY_DELAY,
        |_| base.clone(),
        &demo_catalog(),
        &tmp.path().join("m"),
    )
    .unwrap();
    std::env::remove_var("GRH_API_BASE");
    assert!(
        matches!(results[0].outcome, PinOutcome::NotFound(_)),
        "{results:?}"
    );
}

#[test]
fn a_mismatch_beside_an_unreachable_host_is_exit_one_with_both_listed() {
    let _guard = NETWORK.lock().unwrap_or_else(|e| e.into_inner());
    clear_routing_env();
    let off = one_byte_off(MANIFEST);
    let port = serve(move |_| Some((200, off.clone())));
    let tmp = tempfile::tempdir().unwrap();
    let copies = tmp.path().join("manifests");
    fs::create_dir_all(&copies).unwrap();
    fs::write(copies.join("one.toml"), MANIFEST).unwrap();
    let live = format!("http://127.0.0.1:{port}");
    let results = check_from_with_retry_delay(
        RETRY_DELAY,
        |name| {
            if name == "two" {
                "http://127.0.0.1:1".to_string()
            } else {
                live.clone()
            }
        },
        &two_pins(),
        &copies,
    )
    .unwrap();
    assert_eq!(results.len(), 2);
    assert!(matches!(results[0].outcome, PinOutcome::Mismatch { .. }));
    assert!(matches!(results[1].outcome, PinOutcome::Unreachable(_)));
    assert_eq!(check_exit_code(&results), 1);
}

#[test]
fn the_built_example_answers_help_usage_errors_and_a_catalog_with_no_pin() {
    // A catalog with no pin, not the shipped one: that gains pins (and so
    // network downloads) once parts ship their own manifests.
    let scratch = tempfile::tempdir().unwrap();
    let empty = scratch.path().join("catalog.toml");
    std::fs::write(&empty, "").unwrap();
    let run = |args: &[&str]| {
        Command::new(env!("CARGO"))
            .args(["run", "--locked", "--quiet", "--example", "pins", "--"])
            .args(args)
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .env("GRH_CATALOG", &empty)
            .output()
            .unwrap()
    };
    let help = run(&["-h"]);
    assert_eq!(help.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&help.stdout).contains("Exit codes"));
    let unknown = run(&["--bogus"]);
    assert_eq!(unknown.status.code(), Some(2));
    assert!(unknown.stdout.is_empty());
    let none = run(&["check"]);
    assert_eq!(none.status.code(), Some(0));
    assert_eq!(
        String::from_utf8_lossy(&none.stdout),
        "0 pinned manifests\n"
    );
}

#[test]
fn pin_refuses_an_asset_the_catalog_load_refuses_with_the_same_reason() {
    let wrong_repo = String::from_utf8(MANIFEST.to_vec()).unwrap()
        + "\n[tool]\nbin = \"demo\"\n[tool.release]\nrepo = \"fixture/elsewhere\"\nversion = \"1.0.0\"\n[tool.release.sha256]\nx86_64-unknown-linux-gnu = \"00\"\n";
    let tmp = tempfile::tempdir().unwrap();
    let srv = tmp.path().join("srv");
    release(&srv, "fixture/pinned-demo", wrong_repo.as_bytes());
    let copies = tmp.path().join("manifests");
    let Err(PinError::Invalid(from_pin)) = pin_demo(&file_base(&srv), &copies, Existing::Keep)
    else {
        panic!("pin accepted an asset naming another repo");
    };
    let source = catalog_with(&[pin_block(
        "pinned-demo",
        "fixture/pinned-demo",
        wrong_repo.as_bytes(),
    )]);
    let from_load = grh::ops::text(
        &Catalog::parse_with(&source, &[("pinned-demo", wrong_repo.as_bytes())]).unwrap_err(),
    );
    let why = "its [tool.release] repo is fixture/elsewhere, the pin says fixture/pinned-demo";
    assert!(from_pin.contains(why), "{from_pin}");
    assert!(from_load.contains(why), "{from_load}");
    assert!(!copies.exists());
}

#[test]
fn the_example_ends_quietly_with_its_own_exit_code_when_stdout_is_closed() {
    let mut child = Command::new(env!("CARGO"))
        .args([
            "run",
            "--locked",
            "--quiet",
            "--example",
            "pins",
            "--",
            "-h",
        ])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    drop(child.stdout.take());
    let finished = child.wait_with_output().unwrap();
    assert_eq!(finished.status.code(), Some(0));
    assert!(finished.stderr.is_empty(), "{:?}", finished.stderr);
}
