---
status: done
branch: feat/part-model-is-data
repos: [grh]
adrs: [ADR-0064]
---
# grh's part model is data

## Review panel

📄 Full reviews: [2026-11-08-grh-part-model-is-data.reviews.md](2026-11-08-grh-part-model-is-data.reviews.md)

👉 **Decide:** none — approve if one manifest type for catalog and extensions, with the parts renamed and receipts migrated once, is the right first slice of step 2.
📍 grh · nothing built · next: Phase 1, manifest v2. Panel: backend, lang:rust, unix, tui, architect, po (all opus).
**Changed by review:** `[tool.release]`/`[tool.repo_level]` (a TOML clash); receipt migration only under the lock, `receipt.json.pre-0.2` backup, `receipt::VERSION` 2; unpinned manifests list every command shell-quoted on stderr, even under `--quiet`.
**Verdicts:** round 2 approve 4 (tui, po, architect, unix), approve-with-changes 2 (backend, lang:rust); 0 rework; 13 findings carried into implementation (listed in the full reviews).

## Picture

```
 today                               after this plan
 -----                               ---------------
 catalog.toml --> Part               catalog.toml --> [[part]] = manifest v2
 owner.rs: match "amont-list" ...    status = { args, json | exit }  (data)
 uninstall: if name == "gate"        keep_binary = { args, paths }   (data)
 install: ATTEST_WORKFLOW const      [[repo_file]] { dirs, path, content }
 extension.rs: its own install path  --extension dir --> the same Run::part
 parts "gate", "guardrails"          "greengate", "greenpiste", aliases kept
```

What each part does used to live in grh's code; after this plan it lives in
the part's manifest, and the catalog and `--extension` read the same type.

## Goal

Build step 2a of ADR-0064 (programme `decisions/docs/programmes/gate-guardrails`).
grh describes every part with one manifest type, `greenharness.toml` v2, used
both for the catalog's entries and for `grh install --extension <dir>`. What
grh's code knew about particular tools moves into those manifests: how to ask
a tool whether it is installed, when a binary must be kept, which file a part
places in a repository, which host command an integration needs. The parts
take their brand names, `greengate` (was `gate`) and `greenpiste` (was
`guardrails`); the old names keep working with a warning, and a receipt that
records them is migrated once.

## Non-goals

- Profiles, the interactive checklist for a bare `grh install`, a generic
  `grh audit`, and the CI check that grh's source names no part: plan 2b.
  `audit.rs` keeps naming parts until then, rewired onto the generic query.
- Manifests fetched from the tools' own releases (a catalog entry that pins a
  manifest's location and sha): plan 2c. Here the catalog holds every
  manifest inline.
- Renaming the binaries (`amont`, `amont-agent` stay): build step 3.
- The other parts' brand names (greentrail, greenlift, greenpatrol,
  greenskipass) as catalog names: their rename programme.

## Code review
- grh: backend, lang:rust, unix, tui, architect (po left out: no product wording beyond the alias warning, which the tui seat reviews)


## Behaviour

- **One manifest type.** `greenharness.toml` v2 (`HARNESS_VERSION` 0.2) keeps
  every v1 section (`[extension]`, `[[skill]]`, `[[agent]]`, `[[mcp]]`,
  `[[session_context]]`, `[[doctor]]`, `[conform]`) and adds:
  - in `[extension]`: `summary`, `needs`, `enhances`, `aliases`, `hint`
    (printed, never run) and `published` (default true; replaces the catalog's
    `extension = "unpublished"`);
  - `[tool]`: `bin`; `[tool.release]`: `repo`, `version`, `sha256` per target;
  - `[tool.machine]` and `[tool.repo_level]` (not `[tool.repo]`, which would
    clash with a string key): `install`, `uninstall`, `requires` (host
    commands that must be on PATH, e.g. `["claude"]`) and `status`, either
    `{ args, exit = { present, absent } }` or
    `{ args, json = ["installed", "state"] }` (present, absent, partial,
    unknown; anything else is unknown);
  - `[tool.keep_binary]`: `{ args, state = [...], absent = "...",
    shims = [...], binary = "...", warning = "..." }`;
  - `[[repo_file]]`: `{ dirs = [".forgejo", ".github"], path, content }`;
  - `[bundle]`: `skills`, `agents` taken from grh's pinned skills bundle.

  A v1 manifest (Duro's `harness = ">=0.1"`) still loads and installs. The v2
  sections and keys require `harness = ">=0.2"` and are refused otherwise.
- **The catalog is manifests.** `catalog.toml` keeps its top-level skills
  bundle pin and profiles, and each `[[part]]` is a v2 manifest.
  `Catalog::checked` refuses:
  - a `needs` or `enhances` naming no part;
  - a cycle across both;
  - an alias equal to any part's name or alias;
  - a part missing from the catalog's `order` list;
  - an edge that points up `order`.

  `order` is ADR-0064's list in catalog names; edges resolve through aliases.
  `needs` resolve transitively, as today; `enhances` are shown and never
  installed for a part.
- **No tool in the install path.**
  - `owner.rs` keeps `json_state` and `exit_pair` and loses its five named
    queries.
  - `uninstall.rs` loses its `name == "gate"` branch.
  - `install.rs` loses `ATTEST_WORKFLOW` and the attest-only kind check. A
    `repo_file` lands only in a directory that, canonicalised, lies under
    `git_root()`; `..`, absolute and symlinked-out paths are refused. It is
    written through `write_atomic`.
  - A machine integration whose `requires` is missing is skipped with a
    Summary gap naming the missing command and the `grh install <part>` to
    rerun.
- **Extensions use the same path.** `--extension <dir>` goes through
  `Run::part`, so an extension can carry a tool. The `status` and
  `keep_binary` args are recorded in the receipt's `ToolInstall` entry at
  install time, so uninstall undoes a tool and confirms it absent even after
  the extension directory is gone; an MCP registration keeps its own check.
- **Unpinned manifests show what they run.** For a manifest the catalog does
  not pin, the plan lists, on stderr and even under `--quiet`, before the
  consent prompt: every command it would run (install, uninstall, status,
  keep-binary), one per line with each argument shell-quoted; every
  `repo_file` path (not its content); every `requires` entry. Off a terminal
  without `--yes` the list still prints, then exit 2 with nothing changed.
- **Names.** The catalog's parts are `greengate` and `greenpiste`, with
  `aliases = ["gate"]` and `["guardrails"]`. `grh install gate`,
  `grh uninstall gate` and `grh gate …` work. Each prints one `ui.warn` line
  on stderr naming the replacement, e.g.
  `gate is now greengate: run grh install greengate`. For `grh gate …` the
  line comes before the exec, so stdout is the tool's alone.
- **Receipt migration.** Only the command that opens the receipt's `Store`,
  under its exclusive lock, migrates it:
  - it first copies the receipt to `receipt.json.pre-0.2`, the downgrade
    path to 0.1.19;
  - it rewrites entries that name an alias to the canonical name, through
    `Store::save` (temp file, fsync, rename);
  - it raises `receipt::VERSION` to 2, so grh 0.1.x refuses the new receipt
    instead of misreading it;
  - it prints one stderr line naming the receipt path and each rename.

  Commands that do not hold the lock map aliases in memory and never write,
  so the migration never makes them exit `Locked`. `grh status --json` stays
  valid JSON on stdout on the migrating run. The CHANGELOG names the
  downgrade step.


## Phases

- [x] Phase 1 — manifest v2: the types, `HARNESS_VERSION` 0.2, v1
  compatibility and the `>=0.2` rule for v2 sections, validation
  (`requires`, `status`, `keep_binary`, `repo_file` paths relative and
  inside the repository):
  `src/manifest.rs`, `docs/extension-manifest.md`.
- [x] Phase 2 — the catalog as manifests: `Part` becomes the v2 part type,
  `catalog.toml` rewritten (attest's workflow content, the queries and the
  keep-binary spec move into it; parts renamed with aliases; an `order`
  list), `checked` with the cycle and order checks, `resolve` and
  `dependents` unchanged in behaviour: `src/catalog.rs`, `catalog.toml`.
- [x] Phase 3 — generic queries and keep-binary: `src/owner.rs`,
  `src/uninstall.rs`, `src/status.rs`.
- [x] Phase 4 — generic install: repo files, `requires`, bundle skills,
  `--extension` through `Run::part`, unpinned manifests show their commands:
  `src/install.rs`, `src/extension.rs`, `src/skills.rs`, `src/ops.rs`,
  `src/main.rs`.
- [x] Phase 5 — aliases and the receipt migration: `src/receipt.rs`,
  `src/catalog.rs`, `src/dispatch.rs`, `src/main.rs`.
- [x] Phase 6 — audit on the generic query (names stay until 2b):
  `src/audit.rs`.
- [x] Phase 7 — tests, e2e and release 0.2.0: `tests/cli.rs`,
  `scripts/e2e.sh`, `CHANGELOG.md`, `Cargo.toml`, `Cargo.lock`, `README.md`.

## Decision log

- 2026-11-08 — build step 2 of ADR-0064 is split into three plans (2a this
  one, 2b profiles/checklist/audit/CI name check, 2c remote manifests),
  because the coupling map found eight hard-coded audit rows and five named
  queries on top of the manifest work; one plan for all of it would not fit
  a session.
- 2026-11-08 — built through three relais runs (Phases 1–2, 3–4, 5–6) and finished by hand: each review caught a real gap (aliases persisted too early, skills under an old name left behind, the five named queries still in `owner.rs`, a version-1 receipt with no alias left at version 1, audit keeping its own copy of the queries), each fixed with a test proven red without the fix where it mattered.
- 2026-11-08 — audit finds a part by its binary (`amont`, `amont-agent`, `relais`) rather than by name, so it reads the same declared status query whatever the catalog calls the part; its rows keep their names until plan 2b.
- 2026-11-08 — "migrated once and never again" is measured on the backup (its bytes and mtime unchanged) and the absent notice, not on the receipt's mtime: an install legitimately writes pending steps to the receipt.
- 2026-11-08 — Phase 3's grep became a permanent test, `tests/no_part_names.rs`, ahead of plan 2b; 2b widens it to `audit.rs` and wires it into CI as the "names no part" check.
- 2026-11-08 — reversed during the implementation review: `receipt.json.pre-0.2` is rewritten from the current file whenever that file is older than version 2 (backend: an upgrade, downgrade, upgrade must leave the copy the downgrade restored); a version-2 receipt never overwrites an existing copy.
- 2026-11-08 — two behaviour changes the review asked to name, both in the CHANGELOG: the keep-binary check reads states with `parse_state` (`installed` counts as present; it keeps more, never fewer), and uninstall refuses an undo it cannot check when the binary is gone.

## Verification

- Phase 1: unit tests load Duro's v1 manifest
  (`duro-design-system/packages/cli/greenharness.toml`, copied as a fixture)
  and a v2 one that uses `[tool.release]` and `[tool.repo_level]`; an unknown
  key is refused; `harness = "^0.1"` is refused and `">=0.1"` accepted at
  0.2; a v2 section under `harness = ">=0.1"` is refused.
- Phase 2: `cargo test`: the shipped catalog passes `checked`, and every one
  of its edges points down `order`; fixtures with a `needs` cycle, an
  `enhances` cycle, an upward edge, a part missing from `order` and an alias
  colliding with a name each fail, naming it; `resolve(["greengate"])` returns
  greengate and its declared `needs` only, never an `enhances` or the bundle.
- Phase 3: `grep -nE '"(amont|aval|relais|gate|greengate|greenpiste|attest|claude)' src/owner.rs src/uninstall.rs src/install.rs src/extension.rs src/status.rs src/ops.rs`
  finds nothing outside `#[cfg(test)]` modules; the template-dir test
  (`tests/cli.rs`) still keeps the binary while a shim points at it, now
  driven by the manifest spec; unit tests cover each keep branch of the spec
  (non-zero exit, non-JSON, unknown state, present with no shims, a shim
  without `binary`, a canonicalised match) and `absent` removes.
- Phase 4: `tests/cli.rs`:
  - attest's workflow is placed from the manifest's `[[repo_file]]` with the
    same backup and identical-bytes rules;
  - a symlinked `.github` pointing outside the repository, `..` and an
    absolute path are refused;
  - an extension fixture with a `[tool]` installs, its directory is deleted,
    and its uninstall still runs the tool's undo and confirms absent;
  - with `--quiet` and stdin closed (`</dev/null`), an unpinned manifest's
    shell-quoted commands, repo file paths and `requires` appear on stderr,
    and the exit is 2;
  - a missing `requires` leaves a Summary gap naming the command.
- Phase 5:
  - a seeded receipt with `"part": "gate"` is rewritten to `greengate` on the
    first locked command and never again (its mtime unchanged by the second);
  - `receipt.json.pre-0.2` holds the original bytes, which grh 0.1.19 reads;
  - grh 0.1.19 refuses the migrated receipt (version 2);
  - with `receipt.lock` held by a second process, `grh status` exits 0 and
    leaves the receipt unchanged;
  - `grh status --json | jq .` exits 0 on the migrating run;
  - `grh gate --version 2>/dev/null` is byte-identical to
    `grh greengate --version 2>/dev/null`, and the warning appears once on
    stderr.
- Phase 6: `grh audit` in a scratch HOME with amont 2.3.1 installed reports
  the same rows as 0.1.19, now from the manifest status query.
- Phase 7: `cargo test`, clippy, `scripts/e2e.sh` against the real releases
  (amont 2.3.1, amont-agent 3.23.1); in a scratch HOME, `grh install
  greengate` places amont 2.3.1 and `grh install gate` the same with a
  warning; Duro's real manifest installs with `--extension` with the same
  output as under 0.1.19.

Observed (2026-11-09, branch `feat/part-model-is-data`):
- Phase 1 → `src/manifest.rs` tests load Duro's v1 manifest (`tests/fixtures/duro-greenharness.toml`, byte-identical to Duro's) and a v2 one with `[tool.release]` and `[tool.repo_level]`; an unknown key, `^0.1` and a v2 section under `>=0.1` are each refused.
- Phase 2 → the shipped catalog passes `checked` with every edge pointing down `order`; the cycle, upward-edge, missing-from-order and alias-collision fixtures each fail naming the cause; `resolve(["greengate"])` is greengate alone; a toml string breaking an invariant fails to deserialise into `Catalog`.
- Phase 3 → the five named queries, `amont_template` and the `name == "gate"` branch are gone; `tests/no_part_names.rs` passes (and catches a part name in a two-line literal); eight keep-binary unit tests, one per branch, plus `installed`.
- Phase 4 → `tests/cli.rs`: attest's workflow placed from `[[repo_file]]` with the backup and identical-bytes rules; symlinked-out, `..` and absolute paths refused; an extension with a `[tool]` installs, its directory is deleted, and uninstall still undoes it; an adopted extension tool leaves no entry; the unpinned listing (download, commands, doctor, repo files, `requires`) appears on stderr under `--quiet </dev/null` with exit 2.
- Phase 5 → a seeded `gate` receipt migrates once to version 2 with the original kept in `receipt.json.pre-0.2`; a version-1 receipt with no alias is raised too; upgrade, downgrade, upgrade leaves the second receipt as the backup; with the lock held `grh status` exits 0 and writes nothing; `status --json` parses on the migrating run; `grh gate --version` stdout equals `grh greengate --version`, one warning naming `grh greengate`; `grh uninstall gate` names `grh uninstall greengate`. By hand, not run: grh 0.1.19 reading the backup and refusing version 2 (its version check at `receipt.rs` refuses any newer version).
- Phase 6 → audit's rows are unchanged with the fake amont installed, and a changed catalog query changes audit's verdict.
- Phase 7 → `cargo test` (48 cli tests and the unit tests), clippy `-D warnings`, fmt; `scripts/e2e.sh` against amont 2.3.1 and amont-agent 3.23.1: every part installed and placed, nothing left behind, adopted integration and binary left alone, the `gate` alias warning on stderr; Duro's real manifest dry-run gives the same plan as 0.1.18 plus the pre-consent listing. Version 0.2.0, CHANGELOG with the downgrade path; the tag follows the merge.

## Implementation review

- implementation-review: approve (two deltas); fixed: plan record, the edge-error direction, audit advice names, an `installed` test proven red; deliberate: the 20 ms mtime sleep (the bytes check still catches a rewrite); 86k/128 s + 60k/61 s + 37k/11 s.
- backend: approve (delta); fixed: backup refreshed on every older receipt, unreadable/unknown/gone-binary uninstall messages, `Store::open_migrating`; 55k/64 s + 39k/26 s.
- lang:rust: approve (delta); fixed: `Catalog` deserialises only checked, rank map, `parse_state`, shell-quoted argv; deliberate: `name_of_alias` allocation, `Serialize` derives; 64k/77 s + 37k/16 s.
- unix, tui, architect: approve (deltas); fixed: alias notices name the verb, `{bin}` substituted, repo-level `--repo` gap, CHANGELOG downgrade wording, two-line migration notice, `warn_aliases` in the catalog; deliberate: bare binary name in the unpinned listing, runtime-only `repo_file` path check, the JSON path in the keep-binary gap.

## Outcome

After 0.2.0, for 7 days on the person's machine: `grh status` reports every
installed part present (none `unknown`), and the alias warnings are
counted. Roll back to 0.1.19 (restoring `receipt.json.pre-0.2`) if an
installed part reads `unknown`, or `grh install <part>` installs a part
outside its `needs`.

<!-- panel: repos=grh adds=contract reviewers=backend,lang:rust,unix,tui,architect,po body-sha=fd3a8cdcd0a2 -->
