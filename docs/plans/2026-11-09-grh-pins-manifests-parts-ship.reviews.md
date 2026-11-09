# Reviews: grh pins manifests its parts ship

## Full reviews (reference)

Round 1 (body d71ede1014b5): approve-with-changes from lang:rust (27k/27 s), backend (31k/45 s), architect (32k/36 s), unix (34k/38 s), platform (31k/48 s); 23 findings, all accepted and merged by hand (the overlapping script findings led to the `pins` example).

Round 2 (body 494bcc4a83f4): approve-with-changes from platform (27k/17 s), unix (28k/23 s), lang:rust (37k/57 s), backend (58k/84 s), architect (37k/72 s). One high, raised by lang:rust, backend and architect alike (typed download failures), applied by plan-apply.

Final-body checks: on 6b0f27b99088, lang:rust approve (20k/14 s), backend approve-with-changes (23k/34 s), and platform approve-with-changes with a new high (43k/87 s) defining the three failure kinds across routes, applied by plan-apply; on 576c54b9f367, lang:rust (19k/17 s), platform (23k/26 s) and backend (31k/68 s) approve-with-changes, medium and low only. codex: not installed.

Carried into implementation:
1. platform: on a pull request the `pinned manifests` job treats exit 3 as a warning and fails on 1 or 2; a nightly and `workflow_dispatch` run fails on 3 too.
2. platform: `check` passes with `GH_TOKEN` unset against the `file://` fixture (the web route serves public releases).
3. unix, lang:rust, backend: the `check` bullet and `-h` say the same as Phase 2: exit 1 if any pin mismatched or is missing (even when another host was unreachable), else 3; `pin` maps failures the same way.
4. unix: `-h` prints usage to stdout and exits 0; a usage error prints one line plus usage to stderr and exits 2; a closed pipe ends quietly (`check | head -0`); an unknown flag exits 2.
5. unix: the token is sent only to the first request's host, never across a redirect.
6. lang:rust: the `pin`/`check` logic lives in `src/pins.rs` with typed outcomes that `tests/pins.rs` calls (cargo sets no `CARGO_BIN_EXE_` for examples); `examples/pins.rs` is a thin argv and exit wrapper with one smoke test.
7. lang:rust: the atomic write is `receipt::write_atomic`.
8. backend: `Catalog` loses `serde(try_from = "RawCatalog")`; `Catalog::parse` calls `parse_with(source, no copies)`, so a pin with no copy is refused as such on every path.
9. backend: the example's exit codes are its own, not grh's table (where 3 is unpublished), and `-h` says so.
10. architect: the Outcome command carries `--locked`.
11. lang:rust, platform, backend: existing callers of `fetch::download` keep their `Exit` and message byte for byte (`From<FetchFailure> for Exit`); the existing `src/fetch.rs` tests pass with no assertion edited; `grh install`'s 404 exit and message are unchanged.
12. backend: the exit-3 test waits out the real 1 s + 4 s backoff; keep it to one test of about 5–10 s.
13. backend: the release listing's missing asset gets its own `Hit` variant, not a match on a `Hit::Fatal` string.
14. lang:rust, backend: tests for the precedence and the new kinds: one pin one byte off plus another pin at `http://127.0.0.1:1` exits 1 printing both; a local server answering 503 to every attempt exits 3; a local server's listing without the asset exits 1.
15. backend: a 403 rate limit falls under "other" (exit 1); with `GH_TOKEN` in CI this is unlikely; noted, no change.
16. backend (round 1, low): Phase 3's check is e2e's observed result, not "tests pass".

