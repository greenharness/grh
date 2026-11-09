# grh audit is data and profiles go: full reviews

Round 1 (body 05304b47aab4): all seven approve-with-changes; 36 findings accepted and merged by hand into Behaviour, Phases, Decision log, Verification and Outcome (several seats edited the same lines); two declined: lang:rust#2 (an `mcp_registered` signal: the explicit machine_present/repo_present levels cover the decisions row) and po#4 (phase order: answered in the Decision log). Round 2 (body ca45e2a9404a): approve tui, architect; approve-with-changes platform, lang:rust, po, backend, unix. No high or blocking finding survived.

Carried into implementation:
1. platform, po: the rollback is `GRH_VERSION=v0.2.0 sh install.sh`; Phase 7 proves 0.2.0 reads a receipt written by 0.3.0 (`grh status` exits 0, no `unknown`).
2. lang:rust: `--all` skips any part whose `needs` did not install and names it as skipped; a Phase 5 test with `decisions` failing leaves `conform` out of the receipt.
3. po, backend: Phase 7 commits `scripts/outcome-diff.sh` (the day-1/day-7 diff) and keeps the 0.2.0 release binary at `~/.cache/grh/0.2.0/grh`.
4. po: the Decision log says why `"schema": 2` and the quiet broken pipe ship in 0.3.0 (the output path is rewritten anyway; the marker is the only way to detect the breaking JSON shape).
5. backend, unix, architect: the goldens also compare `hooks`, `ci`, `adr_folder`, `languages`, `dir` (placeholder) and `schema`.
6. backend: the Outcome excludes the differences the Decision log records as intended.
7. unix, po: the `decisions` row's switch from `claude mcp get aval` to `machine_present` is recorded in the Decision log before Phase 4 closes; if the golden shows a verdict change, Phase 4 stops for the person's choice.
8. unix: the broken-pipe check uses `set -o pipefail`, because a pipeline's status is `head`'s otherwise.
9. architect, lang:rust: in the aval-MCP fixture the stub `claude` and the stub `aval` answer the same way.
10. tui: Phase 5 asserts every `retired_names` member is a published part, so a copied hint command never fails.
11. platform: whether `grh uninstall --all` exists (the way back from a partial `--all`) is the author's call; uninstall by name works.
12. unix: `status.rs` still uses `println!` (a broken pipe on `grh status` panics): outside this plan, a note for a later fix.

Tokens and time: round 1 tui 33k/47 s, platform 37k/57 s, backend 41k/67 s, architect 45k/53 s, unix 35k/63 s, lang:rust 46k/72 s, po 31k/65 s; round 2 tui 41k/11 s, platform 46k/19 s, lang:rust 57k/23 s, po 40k/16 s, architect 56k/26 s, backend 50k/40 s, unix 54k/38 s. codex: not installed.

