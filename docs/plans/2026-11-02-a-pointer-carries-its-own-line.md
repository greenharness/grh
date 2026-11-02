---
canonical: amont-agent:docs/plans/2026-11-02-a-pointer-carries-its-own-line.md
phases: [3]
status: done
---
# A pointer carries its own code-review line (pointer)

Canonical plan: [amont-agent/docs/plans/2026-11-02-a-pointer-carries-its-own-line.md](https://github.com/fredericrous/amont-agent/blob/main/docs/plans/2026-11-02-a-pointer-carries-its-own-line.md).

This repository carries Phase 3: pin amont-agent 3.14.0 and skills 0.3.2, as grh 0.1.7.

## Code review

- grh: none — pin bump only

## Verification

- Phase 3: each `catalog.toml` pin compared with its release's SHA256SUMS line → all match: amont-agent 3.14.0 on four targets, and skills 0.3.2 `36e4364b…`, whose downloaded tarball hashed to the same value. `Cargo.lock` changes only grh's own version, 0.1.6 → 0.1.7. The gate's tests pass.
- After the release: `grh install workflow --yes` on the laptop, then `amont-agent --version` and doctor. Recorded in the canonical plan.

## Implementation review

- Phase 3: approve-with-changes (29k tokens, 34 s). The reviewer compared the five pins itself: all match. Fixed: this Verification record. The Phase 2 tick goes in the canonical plan's closing record in amont-agent.
