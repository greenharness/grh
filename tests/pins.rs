//! `grh::pins` against a `file://` fixture release and local servers.

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use grh::catalog::Catalog;
use grh::fetch::{sha256_hex, FetchFailure, FetchOptions};
use grh::pins::{
    check_exit_code, check_with, is_part_name, pin_with, Existing, PinError, PinOutcome,
};

const MANIFEST: &[u8] = include_bytes!("fixtures/pinned/manifests/pinned-demo.toml");

/// Short retry waits: only the retry count is under test, not the delays.
const RETRY_DELAY: Duration = Duration::from_millis(1);

fn catalog_head(names: &[&str]) -> String {
    let order: Vec<String> = names.iter().map(|name| format!("{name:?}")).collect();
    format!(
        "skills_repo = \"greenharness/skills\"\nskills_version = \"0.1.0\"\nskills_sha256 = \"x\"\nforeign_markers = [\"relais:begin\"]\norder = [{}]\n\n",
        order.join(", ")
    )
}

/// The fixture manifest, named `name`.
fn manifest_named(name: &str) -> Vec<u8> {
    String::from_utf8(MANIFEST.to_vec())
        .unwrap()
        .replacen("name = \"pinned-demo\"", &format!("name = {name:?}"), 1)
        .into_bytes()
}

fn release(root: &Path, repo: &str, bytes: &[u8]) {
    let dir = root.join(format!("{repo}/releases/download/v1.0.0"));
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("greenharness.toml"), bytes).unwrap();
}

fn file_base(root: &Path) -> String {
    format!("file://{}", root.display())
}

fn options(base: &str) -> FetchOptions {
    FetchOptions::anonymous(base, RETRY_DELAY)
}

fn one_byte_off(bytes: &[u8]) -> Vec<u8> {
    let mut off = bytes.to_vec();
    off.push(b'\n');
    off
}

fn pin_block_with_sha(name: &str, repo: &str, sha256: &str) -> String {
    format!("[[pinned]]\nname = \"{name}\"\nrepo = \"{repo}\"\ntag = \"v1.0.0\"\nsha256 = \"{sha256}\"\n")
}

fn pin_block(name: &str, repo: &str, bytes: &[u8]) -> String {
    pin_block_with_sha(name, repo, &sha256_hex(bytes))
}

fn catalog_with(names: &[&str], blocks: &[String]) -> String {
    format!("{}{}", catalog_head(names), blocks.join("\n"))
}

fn demo_catalog() -> String {
    catalog_with(
        &["pinned-demo"],
        &[pin_block("pinned-demo", "fixture/pinned-demo", MANIFEST)],
    )
}

fn two_pins() -> String {
    catalog_with(
        &["one", "two"],
        &[
            pin_block("one", "fixture/one", &manifest_named("one")),
            pin_block("two", "fixture/two", &manifest_named("two")),
        ],
    )
}

/// A `manifests/` directory holding the fixture manifest for each name.
fn copies_of(root: &Path, names: &[&str]) -> PathBuf {
    let copies = root.join("manifests");
    fs::create_dir_all(&copies).unwrap();
    for name in names {
        fs::write(copies.join(format!("{name}.toml")), manifest_named(name)).unwrap();
    }
    copies
}

fn pin_demo(base: &str, copies: &Path, existing: Existing) -> Result<String, PinError> {
    pin_with(
        &options(base),
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
    let scratch = tempfile::tempdir().unwrap();
    let srv = scratch.path().join("srv");
    release(&srv, "fixture/pinned-demo", MANIFEST);
    let copies = scratch.path().join("manifests");
    let block = pin_demo(&file_base(&srv), &copies, Existing::Keep).unwrap();
    let written = fs::read(copies.join("pinned-demo.toml")).unwrap();
    assert_eq!(written, MANIFEST);
    Catalog::parse_with(
        &catalog_with(&["pinned-demo"], &[block]),
        &[("pinned-demo", written.as_slice())],
    )
    .unwrap();
}

#[test]
fn pin_refuses_to_overwrite_a_differing_copy_without_replace() {
    let scratch = tempfile::tempdir().unwrap();
    let srv = scratch.path().join("srv");
    release(&srv, "fixture/pinned-demo", MANIFEST);
    let copies = scratch.path().join("manifests");
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
    let scratch = tempfile::tempdir().unwrap();
    let srv = scratch.path().join("srv");
    fs::create_dir_all(&srv).unwrap();
    let copies = scratch.path().join("manifests");
    let outcome = pin_demo(&file_base(&srv), &copies, Existing::Keep);
    assert!(
        matches!(outcome, Err(PinError::Download(FetchFailure::NotFound(_)))),
        "{outcome:?}"
    );
    assert!(!copies.exists());
}

#[test]
fn pin_refuses_a_repo_tag_or_asset_that_could_leave_its_place_in_the_url() {
    let scratch = tempfile::tempdir().unwrap();
    let srv = scratch.path().join("srv");
    release(&srv, "fixture/pinned-demo", MANIFEST);
    let base = file_base(&srv);
    let copies = scratch.path().join("manifests");
    let attempts = [
        ("fixture/pinned-demo", "v1.0.0", "../greenharness.toml"),
        ("fixture/pinned-demo", "v1.0.0", "a/greenharness.toml"),
        ("fixture/pinned-demo", "v1.0.0", ""),
        ("fixture", "v1.0.0", "greenharness.toml"),
        ("a/b/c", "v1.0.0", "greenharness.toml"),
        ("fixture/..", "v1.0.0", "greenharness.toml"),
        ("/pinned-demo", "v1.0.0", "greenharness.toml"),
        ("fixture/pinned-demo", "", "greenharness.toml"),
        ("fixture/pinned-demo", "v1/../x", "greenharness.toml"),
    ];
    for (repo, tag, asset) in attempts {
        let outcome = pin_with(
            &options(&base),
            &copies,
            "pinned-demo",
            repo,
            tag,
            asset,
            Existing::Keep,
        );
        assert!(
            matches!(outcome, Err(PinError::Invalid(_))),
            "{repo} {tag} {asset}: {outcome:?}"
        );
    }
    assert!(!copies.exists());
}

#[test]
fn a_part_name_is_letters_digits_dash_and_underscore() {
    assert!(is_part_name("pinned-demo_2"));
    for name in ["", "a b", "../x", "a/b", "a.toml"] {
        assert!(!is_part_name(name), "{name:?}");
    }
}

#[test]
fn pin_errors_are_errors_with_their_cause_as_source() {
    let download = PinError::Download(FetchFailure::NotFound("gone".to_string()));
    let source = std::error::Error::source(&download).unwrap();
    assert_eq!(source.to_string(), "gone");
    assert!(std::error::Error::source(&PinError::Invalid("x".to_string())).is_none());
}

#[test]
fn check_passes_on_a_matching_asset() {
    let scratch = tempfile::tempdir().unwrap();
    let srv = scratch.path().join("srv");
    release(&srv, "fixture/pinned-demo", MANIFEST);
    let copies = copies_of(scratch.path(), &["pinned-demo"]);
    let base = file_base(&srv);
    let results = check_with(|_| options(&base), &demo_catalog(), &copies).unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].outcome, PinOutcome::Match);
    assert_eq!(check_exit_code(&results), 0);
}

#[test]
fn an_uppercase_pinned_sha256_is_accepted() {
    let scratch = tempfile::tempdir().unwrap();
    let srv = scratch.path().join("srv");
    release(&srv, "fixture/pinned-demo", MANIFEST);
    let copies = copies_of(scratch.path(), &["pinned-demo"]);
    let upper = sha256_hex(MANIFEST).to_uppercase();
    let catalog = catalog_with(
        &["pinned-demo"],
        &[pin_block_with_sha(
            "pinned-demo",
            "fixture/pinned-demo",
            &upper,
        )],
    );
    let base = file_base(&srv);
    let results = check_with(|_| options(&base), &catalog, &copies).unwrap();
    assert_eq!(results[0].outcome, PinOutcome::Match, "{results:?}");
}

#[test]
fn check_reports_both_parts_when_two_assets_are_one_byte_off() {
    let scratch = tempfile::tempdir().unwrap();
    let srv = scratch.path().join("srv");
    for name in ["one", "two"] {
        release(
            &srv,
            &format!("fixture/{name}"),
            &one_byte_off(&manifest_named(name)),
        );
    }
    let copies = copies_of(scratch.path(), &["one", "two"]);
    let base = file_base(&srv);
    let results = check_with(|_| options(&base), &two_pins(), &copies).unwrap();
    let names: Vec<_> = results.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(names, ["one", "two"]);
    assert!(results
        .iter()
        .all(|r| matches!(r.outcome, PinOutcome::Mismatch { .. })));
    assert_eq!(check_exit_code(&results), 1);
}

#[test]
fn check_of_a_file_base_without_the_asset_is_not_found() {
    let scratch = tempfile::tempdir().unwrap();
    let srv = scratch.path().join("srv");
    fs::create_dir_all(&srv).unwrap();
    let copies = copies_of(scratch.path(), &["pinned-demo"]);
    let base = file_base(&srv);
    let results = check_with(|_| options(&base), &demo_catalog(), &copies).unwrap();
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

#[test]
fn check_reports_unreachable_for_a_port_nobody_listens_on() {
    let scratch = tempfile::tempdir().unwrap();
    let copies = copies_of(scratch.path(), &["pinned-demo"]);
    let results = check_with(|_| options("http://127.0.0.1:1"), &demo_catalog(), &copies).unwrap();
    assert!(
        matches!(results[0].outcome, PinOutcome::Unreachable(_)),
        "{results:?}"
    );
    assert_eq!(check_exit_code(&results), 3);
}

#[test]
fn a_server_answering_503_to_every_attempt_is_unreachable_after_three_attempts() {
    let attempts = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&attempts);
    let port = serve(move |_| {
        counted.fetch_add(1, Ordering::SeqCst);
        Some((503, Vec::new()))
    });
    let scratch = tempfile::tempdir().unwrap();
    let copies = copies_of(scratch.path(), &["pinned-demo"]);
    let base = format!("http://127.0.0.1:{port}");
    let results = check_with(|_| options(&base), &demo_catalog(), &copies).unwrap();
    assert!(
        matches!(results[0].outcome, PinOutcome::Unreachable(_)),
        "{results:?}"
    );
    assert_eq!(attempts.load(Ordering::SeqCst), 3);
}

#[test]
fn a_web_503_beside_an_api_403_is_unreachable_and_exit_three() {
    let port = serve(|path| {
        if path.starts_with("/api/") {
            Some((403, Vec::new()))
        } else {
            Some((503, Vec::new()))
        }
    });
    let scratch = tempfile::tempdir().unwrap();
    let copies = copies_of(scratch.path(), &["pinned-demo"]);
    let base = format!("http://127.0.0.1:{port}");
    let with_api = FetchOptions {
        api_base: Some(format!("{base}/api")),
        ..options(&base)
    };
    let results = check_with(|_| with_api.clone(), &demo_catalog(), &copies).unwrap();
    assert!(
        matches!(results[0].outcome, PinOutcome::Unreachable(_)),
        "{results:?}"
    );
    assert_eq!(check_exit_code(&results), 3);
}

#[test]
fn a_release_listing_without_the_asset_is_not_found() {
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
    let scratch = tempfile::tempdir().unwrap();
    let copies = copies_of(scratch.path(), &["pinned-demo"]);
    let base = format!("http://127.0.0.1:{port}");
    let with_api = FetchOptions {
        api_base: Some(format!("{base}/api")),
        ..options(&base)
    };
    let results = check_with(|_| with_api.clone(), &demo_catalog(), &copies).unwrap();
    assert!(
        matches!(results[0].outcome, PinOutcome::NotFound(_)),
        "{results:?}"
    );
}

#[test]
fn a_mismatch_beside_an_unreachable_host_is_exit_one_with_both_listed() {
    let off = one_byte_off(&manifest_named("one"));
    let port = serve(move |_| Some((200, off.clone())));
    let scratch = tempfile::tempdir().unwrap();
    let copies = copies_of(scratch.path(), &["one", "two"]);
    let live = format!("http://127.0.0.1:{port}");
    let results = check_with(
        |name| {
            if name == "two" {
                options("http://127.0.0.1:1")
            } else {
                options(&live)
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
fn check_of_a_catalog_grh_refuses_fails_before_downloading() {
    let requests = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&requests);
    let port = serve(move |_| {
        counted.fetch_add(1, Ordering::SeqCst);
        Some((200, MANIFEST.to_vec()))
    });
    let scratch = tempfile::tempdir().unwrap();
    let copies = copies_of(scratch.path(), &["pinned-demo"]);
    let block = pin_block("pinned-demo", "fixture/pinned-demo", MANIFEST);
    let twice = catalog_with(&["pinned-demo"], &[block.clone(), block]);
    let base = format!("http://127.0.0.1:{port}");
    let refused = check_with(|_| options(&base), &twice, &copies).unwrap_err();
    assert!(
        grh::ops::text(&refused).contains("two pins name the part pinned-demo"),
        "{refused:?}"
    );
    assert_eq!(requests.load(Ordering::SeqCst), 0);
}

#[test]
fn the_built_example_answers_help_usage_errors_and_a_catalog_with_no_pin() {
    // A catalog with no pin, not the shipped one: that gains pins (and so
    // network downloads) once parts ship their own manifests.
    let scratch = tempfile::tempdir().unwrap();
    let empty = scratch.path().join("catalog.toml");
    std::fs::write(&empty, catalog_head(&[])).unwrap();
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
    let bad_name = run(&["pin", "../x", "fixture/x", "v1.0.0"]);
    assert_eq!(bad_name.status.code(), Some(2));
    let bogus_then_help = run(&["--bogus", "--help"]);
    assert_eq!(bogus_then_help.status.code(), Some(0));
    let mut closed = Command::new(env!("CARGO"))
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
    drop(closed.stdout.take());
    let finished = closed.wait_with_output().unwrap();
    assert_eq!(finished.status.code(), Some(0));
    assert!(finished.stderr.is_empty(), "{:?}", finished.stderr);
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
    let scratch = tempfile::tempdir().unwrap();
    let srv = scratch.path().join("srv");
    release(&srv, "fixture/pinned-demo", wrong_repo.as_bytes());
    let copies = scratch.path().join("manifests");
    let Err(PinError::Invalid(from_pin)) = pin_demo(&file_base(&srv), &copies, Existing::Keep)
    else {
        panic!("pin accepted an asset naming another repo");
    };
    let source = catalog_with(
        &["pinned-demo"],
        &[pin_block(
            "pinned-demo",
            "fixture/pinned-demo",
            wrong_repo.as_bytes(),
        )],
    );
    let from_load = grh::ops::text(
        &Catalog::parse_with(&source, &[("pinned-demo", wrong_repo.as_bytes())]).unwrap_err(),
    );
    let why = "its [tool.release] repo is fixture/elsewhere, the pin says fixture/pinned-demo";
    assert!(from_pin.contains(why), "{from_pin}");
    assert!(from_load.contains(why), "{from_load}");
    assert!(!copies.exists());
}
