---
canonical: decisions:docs/plans/2026-10-31-plan-names-its-code-reviewers.md
phases: [5]
status: active
---
# The plan names its code reviewers (pointer)

Canonical plan: [decisions/docs/plans/2026-10-31-plan-names-its-code-reviewers.md](https://github.com/fredericrous/decisions/blob/main/docs/plans/2026-10-31-plan-names-its-code-reviewers.md) (ADR-0052).

This repository carries Phase 5. It pins skills 0.3.0, which has the seven `code-review-*` agents, and amont-agent 3.11.0, whose push hook gates on the profiles a plan declares. The workflow part installs the new agents.

## Implementation review

- Phase 5: approve-with-changes, then approve on the delta (39k + 42k tokens, 29 s + 91 s).
- Fixed: the v0.1.4 entry says to run `grh install workflow`.
- The canonical plan's ticks for Phases 1b–4 come with the decisions record PR, after Phase 6. Each phase's review is in its own repository's pointer plan.

## Verification

- Pins against the published checksums:
  - skills 0.3.0: `a1cc0b31…`, the tarball's SHA256SUMS line;
  - amont-agent 3.11.0: four targets, each equal to its SHA256SUMS line;
  - grh pins the tarball's sha, as the 3.10.0 pin `bb3bbf…` matched 3.10.0's SHA256SUMS.
- Agent names: the tarball has the seven `code-review-*.md` files, and `grh install` refuses a name it cannot find.
- After the release: `grh install workflow` on the laptop, then `grh status` and `amont-agent doctor --json` (recorded in the canonical plan).
