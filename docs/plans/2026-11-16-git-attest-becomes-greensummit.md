---
status: active
canonical: greenharness/greensummit docs/plans/2026-11-16-git-attest-becomes-greensummit.md
plan-id: p-01M7JZ490DTGP50C29SRAYEMYB
branch: feat/greensummit-part
repos: [grh]
adrs: [ADR-0073, ADR-0071]
phases: [3]
---
# git-attest becomes greensummit (grh slice)

Pointer to the canonical plan in greenharness/greensummit. This branch is
the grh slice of its Phase 3 (readers), released as grh 0.10.0.

## Code review
- grh: backend, lang:rust (platform, unix, tui, architect left out: a catalog entry, README and tests)

## This slice

- `catalog.toml`: part `attest` becomes `greensummit` with
  `aliases = ["attest"]` (an old name or receipt still resolves to it);
  `order` follows; its workflow file uses `greenharness/greensummit@v2`;
  the summary keeps the role. The file path `workflows/attest.yml` and its
  job name stay: role words (canonical Non-goals), and repositories that
  already hold that file keep matching "already here".
- Pins greengate 4.11.0 and greenbelay 6.2.0 (the release manifests, by
  `cargo run --locked --example pins -- pin`), which write and read
  `refs/notes/greensummit`.
- `README.md` parts table, `tests/no_part_names.rs` (`greensummit` joins the
  names grh's code may not hard-code), `src/catalog.rs` resolve test, and
  `tests/cli.rs` where it names the real part.
- CHANGELOG, version 0.10.0.

## Verification

1. `make check` green (tests, lint, fmt).
2. `make pins-check` green (both new pins re-downloaded and matched).
3. The built binary: `grh install attest --repo --yes --dry-run` (or the
   nearest read-only verb) in a scratch repo names `greensummit` and warns
   that `attest` is an alias; `grh audit` lists `greensummit`.
4. Census outside history:
   `git grep -nE 'git-attest|git_attest|fredericrous/attest|greengate-attest|amont-attest|attest-local/|attest-inputs'`
   lists nothing but CHANGELOG and plans.

## Decision log

## Implementation review
