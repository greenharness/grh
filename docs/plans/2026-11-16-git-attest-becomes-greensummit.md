---
status: active
next: merge, then release grh 0.10.0 (tag-release); then canonical Phase 4 sweep
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
3. The built binary in a scratch repo names `greensummit` and warns that
   `attest` is an alias; `grh audit` lists `greensummit`. Held in the suite
   by `catalog::tests::attest_is_an_alias_of_greensummit_whose_workflow_uses_v2`.
4. Census outside history:
   `git grep -nE 'git-attest|git_attest|fredericrous/attest|greengate-attest|amont-attest|attest-local/|attest-inputs'`
   lists nothing but CHANGELOG and plans.

## Observed (before the push)

1. `make check` → green → green (after the review fix too).
2. `make pins-check` → both pins match → `greengate: ok`, `greenbelay: ok`.
3. Built grh in a scratch repo with `.github/workflows`: `audit --json`
   → greensummit recommended, install line `grh install greensummit` →
   as expected. `install attest --repo --yes` with HOME/XDG sandboxed →
   `! attest is now greensummit: run grh install greensummit`, plan shows
   greengate 4.11.0 and `workflows/attest.yml`; an earlier unsandboxed run
   wrote `attest.yml` with `uses: greenharness/greensummit@v2`.
4. Census → only CHANGELOG/plans → `git grep` of the old names outside
   history: none (the `attest` alias, file path and job name are role words).

## Decision log

- 2026-11-17 — an unsandboxed pilot added two scratch-repo entries to the
  machine's grh receipt; removed by hand (backup
  `~/.local/state/grh/receipt.json.before-scratch-cleanup-20261117`). Pilots
  of `grh install` run with HOME and XDG dirs sandboxed.
- 2026-11-17 — a CLI test of `install attest` on the shipped catalog cannot
  run offline (the catalog pins the real skills tarball by sha256), so the
  alias and the `@v2` content are held by a unit test on the shipped catalog.

## Implementation review

- implementation-review: approve-with-changes → Delta approve; alias/@v2 test added, part version in CHANGELOG; 31k+33k tokens, 15+23 s.
- backend: approve; upgrade-path CLI test deliberate: receipt migration by alias is existing code with its own alias tests; 38k, 35 s.
- lang:rust: approve; no findings; 34k, 21 s.
