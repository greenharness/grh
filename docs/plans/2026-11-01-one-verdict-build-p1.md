---
status: active
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
