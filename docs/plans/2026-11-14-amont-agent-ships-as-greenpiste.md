---
canonical: greenharness/greenpiste:docs/plans/2026-11-14-amont-agent-ships-as-greenpiste.md
phases: [5]
status: active
branch: feat/greenpiste-alias
---
# amont-agent ships as greenpiste (pointer)

grh's slice of the greenpiste plan (gate-guardrails step 3b-1): the manifest
format declares `[tool] bin_aliases`; grh places each declared alias as a
copy of the verified binary (temporary name, then rename), tracks it in the
receipt and removes it on uninstall; a recorded binary whose file name is not
the part's `bin` is stale and fetched again; a grh-owned ToolInstall whose
recorded executable changed is refreshed even when Present, its install and
undo arguments rewritten. The catalog's greenpiste `[[part]]` becomes a
`[[pinned]]` entry for greenpiste 4.0.0 once that release exists, in the same
grh release as the `bin_aliases` support.

## Code review
- grh: backend, lang:rust, unix (lang:python left out: no Python file; platform left out: no workflow changed; tui left out: one recommend string; architect left out: a catalog entry in the existing format)
