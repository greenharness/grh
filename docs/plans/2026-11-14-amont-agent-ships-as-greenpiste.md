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

## Notes

- Pinned at **v4.0.1**, not 4.0.0: 4.0.0's release reached GitHub and
  crates.io but its tap job failed (greenpiste#256), so 4.0.1 is the first
  release Homebrew offers and the one this pin names.
- `scripts/e2e.sh` now copies and runs `greenpiste` (the adopted-tool case
  installs and uninstalls through it); its one remaining `amont-agent` is the
  `~/.claude/amont-agent/` data directory greenpiste still writes (plan 3b-2).
- Observed before the push: `cargo test --locked` passes (the audit golden's
  stub renamed to `greenpiste`, its reason text from the shipped manifest);
  `cargo run --example pins -- pin greenpiste greenharness/greenpiste v4.0.1`
  wrote `manifests/greenpiste.toml` and the pin's sha256. CI's `e2e.sh` is the
  run against the real 4.0.1 release.

## Code review
- grh: backend, lang:rust, unix (lang:python left out: no Python file; platform left out: no workflow changed; tui left out: one recommend string; architect left out: a catalog entry in the existing format)

## Implementation review

- implementation-review: approve-with-changes, then approve ×2 (7dd63768…). Fixed: the uninstall step in the rename test, the duplicate CHANGELOG heading, e2e.sh running greenpiste, the 4.0.1 note, an inline FoundBinary. Deliberate: `settle_superseded`'s `current` flag and a test variable `t` (low).
- backend: approve-with-changes ×2, then approve. Fixed: alias refresh writes before dropping its row (high); renamed non-alias binary removed; old files deleted only after their successor is in place and every integration is current. Deliberate: two lows (an empty-sha alias-named row kept, a foreign alias file holding the old binary).
- lang:rust: approve-with-changes ×2, then approve. Fixed: `binary()` returns whether it replaced, let-else, rows dropped by content, an unreadable adopted binary is an error. Deliberate: `Superseded` as one struct with a row tag, and deleting a pending row's file (grh's own unfinished copy).
- unix: approve-with-changes ×2, then approve. Fixed: a pending alias row recovers; an adopted binary's stale grh copy is removed; nothing is deleted before its replacement works (hooks never name a missing file).
