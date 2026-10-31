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

- The pinned skills hash: `curl` of `greenharness-skills-0.2.0.tar.gz` piped to `shasum -a 256` → expected the release's SHA256SUMS entry → actual `a08b98254882735df0c27101710af19b9efcb9d0670a59556e0e6a19e4a4c148`, the same.
- The pinned amont-agent hashes: the four `catalog.toml` entries → expected the v3.10.0 SHA256SUMS lines for the same targets → actual equal (copied from that file; the 3.9.0 entries were checked to follow the same scheme).
- `grh install workflow --dry-run` with the branch build → expected guardrails at amont-agent 3.10.0 and the workflow skills → actual `binary amont-agent 3.10.0` and `11 skills and 11 agents`.
- The skills 0.2.0 tarball, unpacked → expected the board step in the three skills → actual `amont-agent board` in worktree-task, diagnosing-bugs and tag-release.

## Implementation review
