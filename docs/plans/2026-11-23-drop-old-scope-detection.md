---
canonical: greenrack:docs/plans/2026-11-23-drop-old-scope-detection.md
phases: [2]
status: active
---

# Drop the old design-system scope from detection (pointer)

Canonical plan: [greenrack/docs/plans/2026-11-23-drop-old-scope-detection.md](https://github.com/greenharness/greenrack/blob/main/docs/plans/2026-11-23-drop-old-scope-detection.md) (greenrack-rename t13).

This repository carries Phase 2: the `design` part stops recommending itself for `dep:@duro-app/`. An app on the retired scope audits as "no greenrack dependency". It ships as a minor release.

## Code review

- grh: backend, lang:rust (unix, tui left out: no output change)

## Decision log

- 2026-11-23 — `tests/fixtures/greenrack-greenharness.toml` stays a byte-identical 7.x snapshot, its `@duro-app/` text untouched (the canonical plan said to fix only its comment). `manifest.rs` loads it as `GREENRACK_7`, a released manifest; greenrack 7.8.0 drops the old guard from its own `greenharness.toml`, so the fixture records what 7.x shipped.
- 2026-11-23 — The kept 0.2.0 goldens still carry `"duro": false`: that key is the frozen golden's, not live output; nothing in `src/` emits it.

## Verification

Observed 2026-11-23 on grh `de30a9c`:

- `cargo test --test audit_golden`: passes. The new test `the_retired_scope_is_not_detected_and_the_greenrack_one_names_the_reason` checks the empty, the retired-scope-only and the greenrack apps: no `dep:@duro-app/` signal in any, `design` recommended only on `@greenharness/greenrack-*`, the retired-scope-only app reason "no greenrack dependency".
- With the old `catalog.toml` rule restored, that test fails at its first assertion (the retired scope is a signal again).
- The pre-commit gate (clippy, cargo-test) passed on each commit.

## Implementation review

- implementation-review: approve-with-changes in round 1 (plan records); fixed here. backend: approve-with-changes (CHANGELOG key note, comment), fixed. lang:rust: approve (assertion message style), fixed.
