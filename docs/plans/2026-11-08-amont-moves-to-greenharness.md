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

## Implementation review
- implementation-review: approve-with-changes; both findings are records, written in the canonical plan's Decision log (Phase 7's checks; the pins copied from the published SHA256SUMS, since grh has no `scripts/pin.sh`); 41k/44 s.
- backend: approve; its one check (the real 2.3.1 binary's repo cycle) was run and is below; 42k/70 s.

## Verification
- input: `cargo test --locked` → expected: pass → actual: ok, 46.9 s.
- input: scratch HOME, `PATH=/usr/bin:/bin`, `grh install gate -y` → expected: amont 2.3.1 fetched from greenharness/greengate, sha matches the pin → actual: placed `amont`, `--version` 2.3.1, `--help` names greenharness.github.io/greengate.
- input: the same for `grh install guardrails -y` → expected: amont-agent 3.23.1 from greenharness/greenpiste → actual: placed, `--version` 3.23.1; the Claude Code step reported a gap (no `claude` on that PATH), as expected.
- input: amont 2.3.1 in a scratch repo, `init` → `list --json` → `uninstall --repo` → `list --json` → expected: `installed.state` present, then absent → actual: present, absent; `template_dir.state` and `shims[].binary` still carry the fields `src/owner.rs` reads.
