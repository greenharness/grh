---
canonical: greenrack:docs/plans/2026-11-23-drop-old-scope-detection.md
phases: [2]
status: active
---

# Drop the old design-system scope from detection (pointer)

Canonical plan: [greenrack/docs/plans/2026-11-23-drop-old-scope-detection.md](https://github.com/greenharness/greenrack/blob/main/docs/plans/2026-11-23-drop-old-scope-detection.md) (greenrack-rename t13).

This repository carries Phase 2: the `design` part stops recommending itself for `dep:@duro-app/`. An app on the retired scope audits as "no greenrack dependency". It ships as a minor release.

## Code review

- grh: backend, lang:rust (unix, tui left out: no output change)
