---
canonical: fredericrous/decisions docs/plans/2026-11-08-amont-moves-to-greenharness.md
phases: [7]
status: active
branch: fix/catalog-greenharness-repos
repos: [grh]
---
# amont and amont-agent move to greenharness (pointer)

This repository's slice of the fleet plan: Phase 7 (the catalog fetches the
two tools from their new repositories, pinned to 2.3.1 and 3.23.1).

## Code review
- grh: backend (unix, tui and lang:rust left out: two `repo =` values and their pins in `catalog.toml`, no code)
