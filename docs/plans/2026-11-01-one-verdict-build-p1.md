---
status: done
branch: chore/relais-profile-trim
repos: [grh]
canonical: decisions:docs/plans/2026-11-01-one-verdict-build.md
phases: [P1]
---
# One verdict per tree: build (pointer, P1)

The canonical plan lives in decisions at `docs/plans/2026-11-01-one-verdict-build.md`. This branch is phase **P1** for grh: profile trims and `cache_baseline`.

## Verification record (P1)

- relais.toml profile, read with tomllib → expected `make check` unchanged (already `lint test`, no msrv) and `cache_baseline = true` → actual: matches.
- `relais plan --task <noop contract> --json` → expected exit 0 → actual: exit 0, policy_hash `ba3de5e5…`.

## Implementation review

- Approve-with-changes, round 1.
- Already present: the verification record (input → expected → actual) is this pointer's "Verification record (P1)". The review brief carries only the canonical plan, so the reviewer could not see it.
- Deliberate: the fleet pack refresh is its own commit, the worktree-task start step (`aval add --check` returned 1), not part of P1's change.
- Deliberate: `[verification.profiles.default]` sits after its command arrays. TOML 1.0 allows it, and all three P1 repositories keep the same layout.
