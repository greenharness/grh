---
canonical: github:fredericrous/decisions/docs/plans/2026-10-30-sweep-rooted-processes.md
phases: [3]
status: active
branch: chore/skills-0.1.1
---
# worktree-sweep stops what still runs in a deleted worktree

Pointer. The plan and its review panel live in decisions; the change itself
shipped in greenharness/skills v0.1.1. This repository carries phase 3:
`catalog.toml` pins skills 0.1.1, and grh is released so `grh install
workflow` places the new `worktree-sweep`.

## Decision log

- 2026-10-30: the plan named this release grh 0.1.1, but 0.1.1 had already
  shipped (#2, relais 0.39.0) when this branch started, so it is 0.1.2.

## Verification

Observed 2026-10-30, on f98b856.

| Check | Input | Expected | Actual |
|---|---|---|---|
| the pin is the published tarball | skills v0.1.1's `SHA256SUMS`, then `shasum -c` on the downloaded tarball | the catalog's sha256 | `452068cd…36cf`, `OK` |
| grh downloads what is pinned | `curl` of the URL `skills_url()` builds, hashed | the catalog's sha256 | the same hash |
| the shipped sweep is the merged one | the tarball's `worktree-sweep/sweep.py` vs skills merge commit 913781c | identical | `cmp` silent |
| the lockfile agrees | `cargo build --locked` | exit 0 | exit 0 |
| project checks | `make check` (lint, test) | pass | pass, 59 tests |
| version | `grh --version` | `grh 0.1.2` | `grh 0.1.2` |
| release guard, by reading the files (it runs only in CI) | `Cargo.toml` version and `CHANGELOG.md` heading | `0.1.2` and `## v0.1.2` | both match |

The tag, the release workflow, the tap and `grh install workflow` follow the
merge; they are recorded in the canonical plan, not here.

## Implementation review

- Round 1: approve-with-changes, 40k tokens, 31 s.
- Fixed: the Verification record above (high).
- Carried to the canonical plan's close: it still names grh 0.1.1; this pointer's Decision log says why it is 0.1.2 (low).
