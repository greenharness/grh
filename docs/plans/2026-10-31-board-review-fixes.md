---
canonical: decisions:docs/plans/2026-10-31-board-review-fixes.md
phases: [4]
status: done
branch: chore/amont-agent-3.12.0
---
# Board review fixes

Pointer. The plan and its review panel live in decisions. This repository
carries phase 4 (C): pin amont-agent 3.12.0, which ships the board review
fixes, and release grh. The plan named 3.11.0 and grh 0.1.4; other sessions
released both first, so this pins 3.12.0 and releases grh 0.1.5. The relais
pin stays where it is (out of scope).

## Verification

- The pinned hashes: `gh release download v3.12.0` of SHA256SUMS and the four darwin and linux-gnu tarballs, `shasum -a 256 -c SHA256SUMS --ignore-missing` → expected four OK → actual four OK; each pin in `catalog.toml` equals its SHA256SUMS line.
- `cargo test` → expected every test to pass → actual 60 passed, 0 failed.
- `grh install guardrails --dry-run` with the branch build → expected amont-agent 3.12.0 → actual `binary amont-agent 3.12.0: adopt the one on PATH, else download`.
- Cargo.lock → expected only grh's own version → actual `0.1.4` → `0.1.5`, nothing else. `v3.11.1..v3.12.0` in amont-agent → #176 and the release commit only, as the CHANGELOG entry says.

## Implementation review

- Round 1 approve-with-changes (29k tokens, 30 s). Fixed: this Verification record (the hashes, the test count, the dry run). Deliberate: the release and `grh install guardrails` happen after this PR merges, so they are recorded in the canonical plan's closing PR, not here.
