---
canonical: decisions:docs/plans/2026-11-02-preview-stale-base-unchanged-screen.md
phases: [3]
status: done
---
# The preview guard ignores a stale remote branch, and an unchanged screen needs no approval (pointer)

Canonical plan: [decisions/docs/plans/2026-11-02-preview-stale-base-unchanged-screen.md](https://github.com/fredericrous/decisions/blob/main/docs/plans/2026-11-02-preview-stale-base-unchanged-screen.md).

This repository carries Phase 3: pin amont-agent 3.15.1 and skills 0.3.3 (3.15.0 left a squash-merged stale ref held; 3.15.1 fixes it), as grh 0.1.8.

## Code review

- grh: none — pin bump only

## Verification

- Phase 3: each `catalog.toml` pin compared with its release's SHA256SUMS line → all match. That is amont-agent 3.15.1 on four targets, and skills 0.3.3 `8e781f4d…`, whose downloaded tarball hashed to the same value. `Cargo.lock` changes only grh's own version, 0.1.7 → 0.1.8.
- Under amont-agent 3.15.1, installed by `brew upgrade` (`amont-agent --version` → 3.15.1, doctor → every check ✓):
  - duro-app's pack refresh landed with no preview, journalled `base=merge-base c82d631 over refs/remotes/origin/chore/refresh-fleet-pack reason=not-descended`, then `no_interface_change`;
  - a scratch UI repo with a byte-identical before/after was journalled `unchanged … sha256=02df2113… before=before.png after=after.png`, and its push passed with no question.

## Implementation review

- Phase 3: approve (27k), then approve-with-changes on the 3.15.1 pins (32k). All pins match. The finding is about the canonical plan's record, updated in decisions.
