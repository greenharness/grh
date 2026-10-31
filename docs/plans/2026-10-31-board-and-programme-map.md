---
canonical: github:fredericrous/decisions/docs/plans/2026-10-31-board-and-programme-map.md
phases: [3]
status: active
branch: chore/skills-0.2.0
---
# Board and programme map

Pointer. The plan and its review panel live in decisions, with ADR-0051.
This repository carries the grh half of phase 3: pin skills 0.2.0, whose
`worktree-task`, `diagnosing-bugs` and `tag-release` check what other
sessions are doing before they start, and release grh.

## Verification

- Scope: this phase also pins amont-agent 3.10.0 (guardrails), because the 0.2.0 skills call `amont-agent board`, which 3.10.0 adds. The canonical Decision log records it in the decisions closing PR.

- The pinned skills hash: `curl` of `greenharness-skills-0.2.0.tar.gz` piped to `shasum -a 256` → expected the release's SHA256SUMS entry → actual `a08b98254882735df0c27101710af19b9efcb9d0670a59556e0e6a19e4a4c148`, the same.
- The pinned amont-agent hashes, observed on the published assets: `curl` of `amont-agent-3.10.0-x86_64-apple-darwin.tar.gz` and `…-aarch64-apple-darwin.tar.gz` piped to `shasum -a 256` → expected guardrails' pins → actual `c723eaba…f4b562` and `bb3bbf81…746cc4`, both equal. The target names are the ones `src/fetch.rs` builds asset URLs from.
- `cargo test` (0.1.3 build) → expected every test to pass → actual 30 + 29 passed, 0 failed.
- `grh install workflow --dry-run` with the branch build → expected guardrails at amont-agent 3.10.0 and the workflow skills → actual `binary amont-agent 3.10.0` and `11 skills and 11 agents`.
- The skills 0.2.0 tarball, unpacked → expected the board step in the three skills → actual `amont-agent board` in worktree-task, diagnosing-bugs and tag-release.

## Implementation review

- Round 1 approve-with-changes (32k tokens, 41 s). Fixed by observation, plan only: the amont-agent pin recorded as a scope addition, the asset hashes taken from the downloads, the test run recorded.
