---
canonical: decisions:docs/plans/2026-11-02-show-the-page-before-asking.md
phases: [3]
status: active
---
# Show the page before asking about it (pointer)

Canonical plan: [decisions/docs/plans/2026-11-02-show-the-page-before-asking.md](https://github.com/fredericrous/decisions/blob/main/docs/plans/2026-11-02-show-the-page-before-asking.md) (ADR-0058).

Phase 3 ends with `grh install` updating `~/.claude/skills/worktree-task`. grh pins the skills release in its catalog, so this repository carries that step: pin skills 0.3.4 (worktree-task F7 opens the app with `amont-agent open`) and amont-agent 3.16.0 (the `open` command and the `ask-unopened-page` rule), released as grh 0.1.9.

## Code review

- grh: none — two pins and their sha256s copied from the releases' SHA256SUMS, and the version bump

## Verification

| Check | Actual |
| --- | --- |
| sums | copied from amont-agent v3.16.0 and skills v0.3.4 `SHA256SUMS` (downloaded with `gh release download`) |
