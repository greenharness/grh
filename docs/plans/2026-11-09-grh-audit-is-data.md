---
status: done
branch: feat/audit-is-data
repos: [grh]
adrs: [ADR-0064]
---
# grh audit is data and profiles go

## Review panel

📄 Full reviews: [2026-11-09-grh-audit-is-data.reviews.md](2026-11-09-grh-audit-is-data.reviews.md)

👉 **Decide:** none — approve if audit's rules living in the manifests, profiles gone with a hint, and `--all` is the right second slice of step 2.
📍 grh · Phases 1–7 built and verified · next: land, merge, tag v0.3.0, then the day-7 outcome check. Panel: backend, lang:rust, platform, unix, tui, architect, po (all opus).
**Changed by review:** signals name their level; reasons and warnings are templates with a fixed placeholder list; path lists move to catalog data; goldens run under a stub PATH; `--all` needs `--yes` off a terminal.
**Verdicts:** round 2 approve 2 (tui, architect), approve-with-changes 5; 0 rework; 12 findings carried into implementation (listed in the full reviews).

## Picture

```
 today                           after this plan
 -----                           ---------------
 audit.rs: 8 hard-coded rows     manifest [[recommend]] rules, per part
 audit.rs: husky, hooksPath      manifest [[conflict]], per part
 audit.rs: ADR/CI/Claude paths   catalog.toml [audit] path lists
 catalog.toml [profiles]         gone: install --all, part names, or (2c)
                                 a checklist
 grh install solo               --> exit 2: profiles were removed
 no_part_names: 6 files          every src file, more words, CI step
```

Everything audit knew about particular tools moves into the parts'
manifests and the catalog, and the profiles that audit chose between
disappear.

## Goal

Build step 2b of ADR-0064 (programme `decisions/docs/programmes/gate-guardrails`).
After plan 2a (grh 0.2.0), the last code that names tools is `grh audit`
(eight hard-coded recommendation rows, the hook-manager warnings, a Duro
check, the ADR, CI and Claude path lists) and the profiles it chooses
between. This plan makes `audit` read its rules from each part's manifest and
its path lists from the catalog, removes the profiles (solo, team, full),
adds `grh install --all`, and widens the test that grh's source names no tool
to every source file. The interactive checklist that replaces a bare
`grh install` is plan 2c; remote manifests are plan 2d. Released as grh 0.3.0.

## Non-goals

- The interactive checklist for a bare `grh install`: plan 2c. Until then a
  bare `grh install` prints the part list and exits 2, as today, without
  the profiles.
- Manifests fetched from the tools' own releases: plan 2d. The site's copy of
  the quickstart is a named follow-up outside this repository.
- Renaming the binaries (build step 3) and the other parts' brand names.
- Any change to what `grh install`, `uninstall` or `status` do for a part.

## Code review
- grh: backend, lang:rust, platform, unix, tui, architect

## Behaviour

- **`[[recommend]]` in a manifest.** Rules in order; the first whose `when`
  holds gives the part's audit row its `verdict` (`already here`,
  `recommend`, `not useful`) and `reason`. A `reason` (and a conflict
  `warning`) is a template: its placeholders are a fixed list that is part
  of the contract (`{adr_folder}`, `{ci}`, `{claude_files}`, `{part}`,
  `{hooks_path}`, `{manager}`), and an unknown placeholder is refused when
  the manifest loads. `when` is a list of signals that must all hold, each
  optionally negated with `!`; an empty list always holds. A part with no
  rule that holds is `not useful`. The signals are a fixed vocabulary,
  repository facts and never part names:
  - `machine_present` and `repo_present`: the part's declared status query
    at that level answers present. A rule names the level it reads, and a
    part that declares no status at that level may not use the signal
    (refused when the catalog is checked). The `decisions` row uses
    `machine_present`; any difference from 0.2.0's `claude mcp get aval`
    probe is written in the Decision log as intended;
  - `adr_folder`, `claude_set_up`, `ci_present`: a path from the lists in
    `catalog.toml` `[audit]` exists (`.adr.yaml`, `docs/adr`, `docs/decisions`,
    `doc/adr`; `.claude`, `CLAUDE.md`, `AGENTS.md`; `.github/workflows`,
    `.forgejo/workflows`), so `audit.rs` holds no such literal;
  - `language_supported`: a file in a supported language;
  - `repo_file_here`: the part's first `[[repo_file]]` exists in a CI dir;
  - `dep:<prefix>`: `package.json` declares a dependency starting with it.
- **`[[conflict]]` in a manifest** replaces the hook-manager warnings: a
  list of `paths`, or a `git_config` key, or `foreign_hooks`, the `warning`
  text it gives when it holds, and a `label` added to `hooks`. The
  `hooks` field of the output is built from the conflicts that hold plus the
  names of the parts whose `repo_present` holds; no literal names a tool. The
  shadowed-binary warning, which already loops over the catalog, stays
  generic.
- **Harness 0.3.** `HARNESS_VERSION` becomes 0.3. `[[recommend]]`,
  `[[conflict]]` and the placeholders need a `harness` that 0.2 does not
  satisfy (a `uses_v3()` check beside `uses_v2()`), so every catalog part
  moves to `harness = ">=0.3"`. The Duro manifest that replaces the `design`
  stub must carry its own rules.
- **`grh audit`.** Eight rows come from the catalog, in catalog order, under
  the part names the catalog gives (`greengate`, `greenpiste`, ...); the name
  column is as wide as the longest. Human output keeps one line per row, then
  the `install` line: `grh install` and the names whose verdict is
  `recommend`; when none is, the line reads `nothing to install`. `--json`
  keeps `dir`, `languages`, `hooks`, `adr_folder`, `ci`, `warnings` and
  `parts[{part, verdict, reason}]`, and adds `"schema": 2`, `signals` (a map
  of the vocabulary above) and `install` (a command string, or `null` when
  nothing is recommended). `profile`, `profile_parts`, `extras`, `claude` and
  `duro` are removed; the CHANGELOG marks the shape change breaking. Output
  goes through `writeln!` on a locked stdout; a broken pipe ends quietly with
  exit 0 (`grh audit | head -1`).
- **Same answers as 0.2.0.** Golden `--json` outputs captured from the 0.2.0
  binary for a fixed set of fixture repositories (empty; Rust with husky;
  TypeScript with ADRs; the gate installed; Claude set up; CI present; Duro;
  the aval MCP registered) are committed first, recorded under a scratch HOME
  and a PATH of stub binaries whose answers each fixture names, with `dir`
  replaced by a placeholder, so CI reproduces them. The new audit reproduces
  every row's verdict and reason and every warning, under the new part names
  (an explicit table `gate`→`greengate`, `guardrails`→`greenpiste`). The
  goldens also hold 0.2.0's `install` line next to the expected new line: a
  profile's unrecommended members drop out (only greengate recommended gives
  `grh install greengate`, not `solo`), and the difference is visible.
- **No profiles.** `[profiles]` leaves `catalog.toml`, `Catalog` and its
  checks; `uninstall` and the help text lose "or profile". `retired_names` in
  `catalog.toml` maps each old name to its former members. `grh install solo`
  (or `grh uninstall solo`) exits 2, changes nothing, writes the hint to
  stderr and nothing to stdout: `profiles were removed in 0.3.0; to install
  what solo installed: grh install greengate greenpiste (or grh install
  --all)`. A retired name mixed with part names (`grh install solo greengate`)
  is refused before any part installs. The three words live in the catalog,
  not in the code.
- **`grh install --all`** installs every published part with what it needs.
  Like any install, off a terminal it needs `--yes` (else exit 2, nothing
  changed, and the refusal names `grh install --all --yes`). `--all` with
  part names or with `--extension` exits 2. `--repo` combines with `--all`
  and with part names, which replaces the quickstart's `grh install team
  --repo`. On a part that fails, `--all` continues with the others, exits 1
  naming it, the receipt holds only the parts that installed, and a rerun
  changes nothing for those. A bare `grh install` still prints the part list
  and exits 2, now without profiles and naming `--all`. The help text lists
  the parts from the catalog.
- **The name check covers all of grh.** `tests/no_part_names.rs` reads `src/`
  recursively and scans every `.rs` file (not `#[cfg(test)]` modules),
  comparing lowercased words, and asserts that each file it finds was
  scanned. The banned words grow to every part and tool name (`greengate`,
  `greenpiste`, `decisions`, `conform`, `router`, `attest`, `workflow`,
  `design`, `amont`, `aval`, `relais`, `duro`, `husky`, `lefthook`) and the
  old `gate` and `guardrails`. `claude` is allowed only in the files that
  talk to the host (`extension.rs`, `skills.rs`, `audit.rs`), which the test
  lists by name and checks exist. `extension.rs`'s `conform eslint` messages
  are reworded, and the relais ownership marker in `skills.rs` becomes
  catalog data (`foreign_markers`). CI runs the test as its own step.

## Phases

- [x] Phase 1 — golden audit outputs from the 0.2.0 binary, before any
  change, under a scratch HOME and stub PATH: `tests/fixtures/audit/`,
  `tests/audit_golden.rs`.
- [x] Phase 2 — manifest `[[recommend]]` and `[[conflict]]`, the placeholder
  list, `HARNESS_VERSION` 0.3 with `uses_v3()`, validation of signals and
  `when`: `src/manifest.rs`, `docs/extension-manifest.md`.
- [x] Phase 3 — the catalog carries each part's rules and conflicts, the
  `[audit]` path lists, `foreign_markers` and `retired_names`, and every part
  moves to `harness = ">=0.3"`: `catalog.toml`, `src/catalog.rs`.
- [x] Phase 4 — audit reads the rules: signals, rule evaluation, placeholders,
  the new output shape and quiet broken pipe: `src/audit.rs`,
  `src/skills.rs`, `tests/cli.rs`.
- [x] Phase 5 — profiles removed, `--all`, the retired-name error, help from
  the catalog (`grh uninstall solo` gives the same error; "or profile"
  leaves the errors and the help text): `src/install.rs`, `src/uninstall.rs`,
  `src/ops.rs`, `src/main.rs`, `src/catalog.rs`, `catalog.toml`,
  `tests/cli.rs`, `scripts/e2e.sh`.
- [x] Phase 6 — the widened name check, `extension.rs`'s reworded messages,
  and its CI step (`no part names`, running `cargo test --test
  no_part_names`, before `cargo test` in the `rust` job):
  `tests/no_part_names.rs`, `src/extension.rs`, `.github/workflows/ci.yaml`.
- [x] Phase 7 — docs and release 0.3.0: `README.md`, `CHANGELOG.md`,
  `Cargo.toml`, `Cargo.lock`.

## Decision log

- 2026-11-09 — plan 2b is split from the checklist: the checklist (a new
  dependency, ratatui, and a screen with its own review) becomes plan 2c, and
  remote manifests become plan 2d. Reason: the audit map found eight rules,
  the hook-manager warnings and a Duro check to make data, which fills a plan
  by itself.
- 2026-11-09 — the person chose: an old profile name is an error with a hint
  (no compatibility release), and `grh install --all` exists.
- 2026-11-09 — the order stays audit first, then profiles: audit's `install`
  line is the only thing that depends on the profiles, and the goldens must be
  green before the profiles go; the person-visible removal lands in the same
  release.

- 2026-11-09 — the `decisions` row reads `machine_present` instead of
  0.2.0's `claude mcp get aval` probe; the aval_mcp_registered golden shows
  no verdict change, so no stop for the person was needed.
- 2026-11-09 — `"schema": 2` and the quiet broken pipe ship in 0.3.0: the
  output path is rewritten anyway, and the marker is the only way a consumer
  can detect the breaking JSON shape.
- 2026-11-09 — `bundle_needs_binary` is replaced by one rule in `Run::part`:
  a part whose binary is missing places neither skills nor repo files (a
  named install used to place skills that pointed at no tool; proven red).
- 2026-11-09 — implementation review (round 1 + Delta) changed the
  contract: conflict kinds are a typed enum, a `foreign_hooks` conflict
  takes no text, placeholders are checked per context, the JSON `signals`
  map holds repository facts only, and an empty `[audit]` list is refused
  when a rule uses it.
- 2026-11-09 — carried, outside this plan: `grh uninstall --all` (the
  author's call; uninstall by name works); `status.rs` still uses
  `println!`; `--extension` manifests' rules are not evaluated by `grh
  audit` (plan 2d); the Delta's low findings below go to plan 2c, which
  reworks install's output.

## Verification

Observed on the finished tree (2026-11-09):

- Phase 1: 13 goldens recorded by the 0.2.0 binary under a scratch HOME and
  the stub PATH; `tests/audit_golden.rs` replays them → green, no harness
  tool needed.
- Phase 2: `bad_rules_are_refused` covers each refusal (unknown signal, a
  part name in `when`, unknown and out-of-context placeholders, a level
  signal with no status, harness `>=0.2`, conflict kinds, foreign_hooks
  text) → green.
- Phase 3: `Catalog::checked` passes on the shipped catalog; every part
  declares `>=0.3`; a path signal with an empty `[audit]` list is refused →
  green.
- Phase 4: every golden row's verdict and reason, every warning, `hooks`,
  `ci`, `adr_folder`, `languages` and `dir` reproduced under the new names;
  `install` matches `expected-new.json`; `grh audit | head -1` exits 0 →
  green.
- Phase 5: retired names exit 2 with the hint (install, uninstall, mixed);
  `--all </dev/null` exits 2 naming the flags given; `--all` with a failing
  need exits 1, skips its dependent, and a rerun leaves the receipt
  unchanged; bare `grh install` lists parts and names `--all` → green.
- Phase 6: the widened check flagged 3 literals on the Phase 5 tree (two
  `conform eslint` messages, one "Claude config directory"), reworded; the
  probe test catches `"Aval"` and `"claude"` outside the host files → green.
- Phase 7: full suite (146 tests), clippy `-D warnings`, fmt → green;
  `scripts/e2e.sh` against the real releases → exit 0; the README quickstart
  in a scratch repo → each command exits as documented; 0.2.0 reads a
  receipt 0.3.0 wrote → `grh status` exit 0, 0 `unknown`;
  `scripts/outcome-diff.sh` 0.2.0 vs 0.3.0 over ~/Developer/Perso → 0
  verdict differences, 0 `unknown` (repositories where both versions fail
  alike, worktrees removed mid-run, are not counted).

- Phase 1: the golden files are produced by the installed 0.2.0 build (its
  `--json`) for the eight fixtures, under a scratch HOME and a PATH of stub
  binaries in `tests/fixtures/audit/bin`, `dir` replaced by a placeholder;
  `tests/audit_golden.rs` replays them under the same HOME and PATH, so it
  passes on a clean CI runner with no harness tool installed.
- Phase 2: unit tests load a manifest with `[[recommend]]` and `[[conflict]]`;
  an unknown signal, an unknown placeholder, a `when` naming a part, a level
  signal on a part with no status at that level, and the new keys under
  `harness = ">=0.2"` are each refused (`uses_v3()`); the `^0.2` assertion in
  `harness_requirements` is updated for 0.3.0.
- Phase 3: `Catalog::checked` still passes; every part has at least one rule
  that holds for an empty repository; every part declares `harness >=0.3`.
- Phase 4: `tests/audit_golden.rs` passes: every golden row's verdict and
  reason and every warning are reproduced under the new part names; the `install` line
  matches each fixture's expected new line; changing a part's rule in a
  fixture catalog changes its row; a conflict path produces its warning;
  `grh audit | head -1` exits 0 with no panic; an empty fixture prints
  `nothing to install` and `"install": null`.
- Phase 5: `grh install solo` and `grh uninstall solo` exit 2 with the hint
  on stderr, stdout empty and the receipt unchanged, and so does
  `grh install solo greengate`; `grh install --all </dev/null` exits 2 naming
  `--yes`; `grh install --all --yes` in a scratch HOME with no terminal
  installs every published part; `--all greengate` and `--all --extension
  DIR` exit 2; `--all` with a failing part continues, exits 1 naming it and
  leaves the others in the receipt; a bare `grh install` lists parts only and
  names `--all`; `scripts/e2e.sh` runs `--all --yes` instead of `full`.
- Phase 6: `cargo test --test no_part_names` on the Phase 5 tree lists every
  literal the widened check flags, and passes on the finished tree; it fails
  when a literal naming a part or a tool is added to any scanned file (a
  probe file containing `"aval"` in a fixture check), asserts that every
  `.rs` file found was scanned, and that each allow-listed `claude` file
  exists.
- Phase 7: `cargo test`, clippy, `scripts/e2e.sh` against the real releases;
  each README quickstart command is run in a scratch repository and exits 0;
  `grh audit` in a scratch repository with amont installed gives the 0.2.0
  rows under the new names.

## Implementation review

- implementation-review: approve-with-changes → Delta approve; tautological name-check assertion fixed; deliberate: `part_facts` loop shadowing (low, carried to 2c); 44k/24 s + 43k/24 s.
- backend approve-with-changes → approve; unknown status warns, git config failure warns, rerun test added; carried: a test for the git-config failure path; 66k/66 s + 36k/14 s.
- lang:rust approve-with-changes → approve-with-changes; ConflictWhen enum, per-context placeholders, stderr without panic; deliberate: `Selection` enum (exclusivity tested), `Vec<Level>` for unknown levels (low, 2c); 54k/56 s + 36k/27 s.
- tui approve-with-changes → approve-with-changes; failure line, listing, retired hint, `--repo` help reworded; carried to 2c: 80-column wrapping of two messages, warning wording; 52k/54 s + 36k/20 s.
- architect approve-with-changes → approve-with-changes; foreign_hooks, signals scope, empty lists fixed; carried: one signal-classification method on `Signal`, a shared git helper (low); platform approve, unix approve (rerun keeps `--quiet`, rows trimmed); 56k/77 s + 36k/21 s, 32k/21 s, 54k/66 s.

## Outcome

After 0.3.0, on day 1 and day 7: a script runs `grh audit --json` under 0.2.0
and 0.3.0 in every repository under ~/Developer/Perso and diffs the per-part
verdicts after the name map (target 0 differences), and counts `unknown` in
`grh status` (target 0). Roll back on any verdict difference or any
`unknown`: run `install.sh` pinned to 0.2.0; 0.3.0 leaves receipts unchanged,
so 0.2.0 reads them.

<!-- panel: repos=grh adds=contract reviewers=backend,lang:rust,platform,unix,tui,architect,po body-sha=ca45e2a9404a -->
