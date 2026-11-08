---
canonical: fredericrous/decisions docs/plans/2026-10-31-design-is-an-optional-part.md
phases: [4]
status: active
branch: feat/pin-skills-040-adrlint-017
---
# Design is an optional part: grh pins (pointer)

This repository's slice of the ADR-0048 plan is section 4 (maintenance only):

- skills pinned to **0.4.0**;
- `plan-review-react` and `plan-review-ui-design` added to the workflow
  part's `agents` list;
- `conform` pinned to adr-lint **0.17.0**;
- the release is **0.1.17**.

The canonical body says skills 0.2.0, adr-lint 0.13.0 and grh 0.1.2; its
Decision log renumbers them, and grh 0.1.16 was taken by the skills 0.3.11
pin. The installer itself is out of scope.

## Implementation review

- Round 1: approve (28k, 13 s). The canonical plan's renumbering is in its Decision log.
- Verification: `make check` green. A fresh `CLAUDE_CONFIG_DIR` and state dir with `grh install workflow --yes` exits 0 and places 74 files, including both new reviewers (no MCP tool); the receipt records them. The checksums were copied by script from the published SHA256SUMS. `grh install conform` was not run on this machine: grh adopts the installed adr-lint rather than fetching, so the run would prove nothing. Next: release 0.1.17, then the laptop.
