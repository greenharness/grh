---
canonical: amont-agent:docs/plans/2026-11-02-an-approval-carries-from-the-session.md
phases: [3]
status: active
---
# An approval carries from the session (pointer)

Canonical plan: [amont-agent/docs/plans/2026-11-02-an-approval-carries-from-the-session.md](https://github.com/fredericrous/amont-agent/blob/main/docs/plans/2026-11-02-an-approval-carries-from-the-session.md).

This repository carries Phase 3: pin amont-agent 3.13.0 and skills 0.3.1, as grh 0.1.6. 0.1.5 was released by another session, pinning amont-agent 3.12.0.

## Verification

- Phase 3: each `catalog.toml` pin compared with its release's SHA256SUMS line → all match: amont-agent 3.13.0 on four targets, and skills 0.3.1 `949789bd…`, whose tarball checked `OK` when downloaded. `Cargo.lock` changes only grh's own version, 0.1.5 → 0.1.6. The gate's tests pass.
- After the release: `grh install workflow --yes` on the laptop, then `amont-agent --version` and doctor. Recorded in the canonical plan.

## Implementation review

- Phase 3: approve-with-changes (30k tokens, 30 s). The pins match. Its findings concern the canonical plan's record in amont-agent: the version moved to 0.1.6, Phase 2's tick, and the branch name. They are fixed there.
