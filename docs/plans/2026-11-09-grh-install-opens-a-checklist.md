---
status: active
branch: feat/install-checklist
repos: [grh]
adrs: [ADR-0064]
---
# grh install opens a checklist

## Review panel

📄 Full reviews: [2026-11-09-grh-install-opens-a-checklist.reviews.md](2026-11-09-grh-install-opens-a-checklist.reviews.md)

👉 **Decide:** none — approve if a pre-ticked checklist on a terminal, with experimental parts (router/relais) opt-in everywhere, is the right step 2c.
📍 grh · nothing built · next: Phase 1, the experimental marker. Panel: backend, lang:rust, ux-research, game-ux, tui, architect (opus-5-5; four round-1 seats relaunched with no model after a rate limit).
**Changed by review:** all three streams must be terminals, with size and cursor fallbacks; SIGTERM/SIGHUP restore; a part that is not experimental may not need an experimental one; typed-ahead keys drained.
**Verdicts:** round 1 approve-with-changes 6; round 2 approve-with-changes 5; 0 rework; 9 findings carried into implementation (listed in the full reviews).

## Picture

```
 grh install            (no part named, no --all)
      |
      +-- stdin, stdout and stderr are terminals ----------+
      |                                                     |
      |   audit pre-ticks what it recommends                |
      |   [x] greengate   what a commit or a push may ...   |
      |   [x] greenpiste  what an agent's tool call ...     |
      |   [ ] router      which model runs ...              |
      |                   experiment: not ticked by default |
      |   [x] also set up this repository                   |
      |   j/k move  space tick  enter review  q cancel      |
      |        |                                            |
      |        v                                            |
      |   the usual plan, then "Proceed? [y/N]"             |
      |                                                     |
      +-- otherwise: the part listing, exit 2 (as today) ---+

 grh install --all      every published part but the experiments
                        "left out: router (an experiment: install it
                        by name)"
 grh audit              rows as before; the install line leaves router
                        out: "router is an experiment: grh install router
                        to try it"
                        (both read one Assessment from audit::assess)
```

A bare `grh install` on a terminal becomes a checklist that audit fills in.
Off a terminal nothing changes. An experimental part is never chosen for the
person: not pre-ticked here, not in `--all`, not in audit's install line.

## Goal

Build step 2c of ADR-0064 (programme `decisions/docs/programmes/gate-guardrails`).
After 0.3.0 removed the profiles, a bare `grh install` only prints the part
list. This plan makes it, on a terminal, an interactive checklist (ratatui and
crossterm, the libraries amont-fleet uses) pre-ticked from `grh audit`'s
recommendations, with a line to also set up the current repository. It adds
an `experimental` marker to the manifest: an experimental part (router, which
is relais, today) is opt-in everywhere. The low findings carried from the
0.3.0 review land here too. Released as grh 0.4.0.

## Non-goals

- Uninstalling from the checklist: it installs only; `grh uninstall <part>`
  is unchanged.
- Manifests fetched from the tools' own releases: plan 2d.
- Renaming binaries (build step 3) and the Homebrew caveats (they already
  name parts, not the checklist).
- Windows: grh is Unix-only today.

## Code review
- grh: backend, lang:rust, unix, tui, architect (platform left out: no CI or deploy file changes)

## Behaviour

- **When the checklist opens.** `grh install` with no part, no `--all` and
  no `--extension`, when stdin, stdout and stderr are all terminals (the inline viewport's cursor query is written to stdout), and `TERM` is
  not `dumb`; a failed cursor-position read falls back to the listing. Otherwise, as today: the part listing on stderr, exit 2,
  nothing changed. `--yes` with no part keeps the listing (`--yes` means no
  prompt, and the checklist is one). `--dry-run` and `--replace` carry into
  the install the checklist starts.
- **What it shows.** One line per catalog part in catalog order: a box, the
  name, the summary, and a short state on its right, moved to a second indented line when the row would not fit the terminal width (the 60-column test pins this):
  - pre-ticked when audit's verdict is `recommend`, with audit's reason;
  - `installed` (from the receipt) and not toggleable;
  - `not published yet` and not toggleable for an unpublished part;
  - `experiment: not ticked by default` for an experimental part, never
    pre-ticked whatever audit says;
  - `needed by <part>` when a ticked part needs it: ticking a part ticks its
    `needs`; unticking a need while a ticked part needs it is refused with a
    one-line notice naming that part and the way out (`needed by greenpiste: untick greenpiste first`);
  - `works with <part>` for each of its `enhances`, shown and never ticked
    for it (ADR-0064 `harness.installer`).

  Below the parts, `also set up this repository`, pre-ticked inside a git
  repository and absent outside one (the same as `--repo`). Outside a git
  repository audit cannot run, so nothing is pre-ticked and the header says
  so. A footer names the keys in amont-fleet's form: `j/k move  space tick  enter review  q cancel` (enter shows the plan; it does not install).
- **Keys.** `up`/`down` and `k`/`j` move; `space` toggles (on a `[-]` line it says why in the footer, e.g. `greengate is installed`); `enter` confirms;
  `q` or `esc` cancels: `nothing changed`, exit 0, as a `n` at `Proceed?`.
  `ctrl-c` (a key event in raw mode) returns `Exit::Interrupted` (exit 130) through `main`, never `std::process::exit`, so the restore guard drops first. Enter with nothing ticked says `nothing
  ticked` in the footer and stays open. Every notice (refused untick, `[-]`
  line, nothing ticked) replaces the key line until the next key.
- **After enter.** Keys already waiting are drained (read and dropped
  through crossterm's event queue before raw mode ends), so a held or
  doubled enter cannot answer `Proceed?`. The checklist then clears its
  inline viewport and restores the terminal; only then is `install::run`
  called in-process with the ticked parts (and `repo` when that line is
  ticked), as if typed. It first prints the command as a bare line, starting
  `grh install `, that can be copied, then the usual plan and `Proceed?
  [y/N]`, which reads a line in cooked mode as it does today.
- **The terminal is always restored.** Raw mode and the cursor are restored
  on enter, cancel, an error, a panic (a panic hook that restores, then
  calls the previous hook), and SIGTERM or SIGHUP: signal-hook (already a
  crossterm dependency) sets a flag the event loop checks every 50 ms; the
  loop restores and returns exit 143 or 129. A drop guard restores on every
  return path. SIGTSTP (ctrl-z) is not handled: raw mode turns it into a key,
  which the checklist ignores.
- **Size.** The checklist needs as many rows as it has lines plus a header
  and the footer. When the terminal is shorter, the list scrolls with the
  cursor; under 8 rows, or under 40 columns, it does not open: the listing,
  exit 2, as off a terminal.
- **Colour.** Named ANSI colours, off under `NO_COLOR` or when stderr is not
  a terminal (the existing `use_color` in `src/ops.rs`); state is always a
  word or a glyph too (`[x]`, `[ ]`, `[-]` for not toggleable), never colour
  alone. The selected line is reversed and starts with `> ` (amont-fleet's cursor).
- **Experimental parts.** `[extension] experimental = true` in a manifest,
  which needs `harness = ">=0.4"` (`HARNESS_VERSION` becomes 0.4, a
  `uses_v4()` check beside `uses_v3()`). The catalog marks `router`. Such a
  part:
  - is left out of `grh install --all`, whose plan names it on one line:
    `left out: router (an experiment: install it by name)`;
  - is left out of audit's `install` line, while its row keeps its verdict;
    audit prints a line after the install line, `router is an experiment:
    grh install router to try it`, when its verdict is `recommend`; `--json`
    gains a top-level `"experimental": ["router"]` list (schema stays 2,
    the field is additive);
  - installs by name, or ticked in the checklist, as today.
    - can never be pulled in for another part: `Catalog::checked` refuses a
    part that is not experimental whose `needs` names one, so neither
    `--all` nor a tick can add it through `needs`.
  The retired-name hints are unchanged (they name members explicitly). The
  marker is the part's own claim, in its manifest; today the catalog holds
  router's manifest. A 0.3.0 grh refuses a manifest that carries it, so a
  rollback to 0.3.0 also reverts the catalog that marks router.
  The `left out:` and `is an experiment` lines are built from catalog data
  (part names never appear in grh's literals, `tests/no_part_names.rs`).
- **Audit as a library.** `audit.rs` exposes `assess(cat, dir) ->
  Result<Assessment<'_>, Exit>`: it runs git and the status queries (so the
  checklist calls it before raw mode starts) and returns rows, hooks,
  warnings, the facts, the install list (recommended and not experimental)
  and the experimental list; it derives `Debug`. Outside a git work tree it
  returns the usage error `run` gives today, which the checklist reads as
  "no repository: nothing pre-ticked". `run` prints the assessment; no
  output change: the goldens pin it.
- **Carried from the 0.3.0 review** (low): the `--all` failure line and the
  bare-install heading wrap before 80 columns (a line break before the
  command); the "status query gave no answer" and git-config warnings name
  what to run next; `PartFacts::unknown` is `Vec<Level>`; one method on
  `Signal` classifies per-part vs repository signals, used by manifest,
  catalog and audit; one git helper builds the command; a test makes `git
  config --get` fail; the `part_facts` loop variable is renamed.

## Phases

- [ ] Phase 1 — experimental parts: the manifest field and harness 0.4,
  router marked, `--all` and audit's install line leave it out through one `Part::is_experimental` (read by install, audit and the checklist), `Catalog::checked` refuses a non-experimental part whose `needs` names an experimental one, JSON
  `experimental`, e2e installs router by name after `--all`:
  `src/manifest.rs`, `src/catalog.rs`, `catalog.toml`, `src/install.rs`,
  `src/audit.rs`, `tests/fixtures/audit/expected-new.json`,
  `tests/audit_golden.rs`, `tests/cli.rs`, `scripts/e2e.sh`,
  `docs/extension-manifest.md`.
- [ ] Phase 2 — audit as a pure `assess`, no output change; `Assessment` carries the install list (recommended, non-experimental) and the experimental list, so the checklist and `run` share them: `src/audit.rs`.
- [ ] Phase 3 — the checklist model and its rendering: a pure state machine
  (rows, cursor, ticks, needs, notice; `on_key(Key) -> Action`) and a
  `draw(frame, &model)`; ratatui 0.30 and crossterm 0.29: `src/checklist.rs`,
  `src/main.rs` (module), `Cargo.toml`, `Cargo.lock`, `deny.toml` if a new
  licence appears. Record the release binary size and build time before and after.
- [ ] Phase 4 — wiring: the terminal check, an inline viewport, raw mode
  with restore on every path and a panic hook, the printed command, then
  `install::run`; raw mode, viewport and panic hook live in `src/checklist.rs`, and `install.rs` only calls it at the bare-install branch with no ratatui or crossterm import; off a terminal unchanged: `src/install.rs`, `src/ops.rs`,
  `src/main.rs`, `tests/cli.rs`, `tests/pty.rs`, `Cargo.toml` (dev-dependency
  for the pty test, named here with its MSRV ≤ 1.89, licence and no unsafe in the test).
- [ ] Phase 5 — the carried 0.3.0 review lows: `src/audit.rs`,
  `src/manifest.rs`, `src/catalog.rs`, `src/install.rs`, `src/ops.rs`,
  `tests/cli.rs`, `tests/audit_golden.rs`.
- [ ] Phase 6 — docs and release 0.4.0: `README.md`, `CHANGELOG.md`,
  `install/install.sh` (its next steps name the checklist), `Cargo.toml`,
  `Cargo.lock`.

## Decision log

- 2026-11-09 — the person chose: an experimental part is opt-in everywhere
  (not pre-ticked, not in `--all`, not in audit's install line), and the
  checklist says so next to its box ("experiment: not ticked by default").
  router (relais) is the experimental part.
- 2026-11-09 — `experimental` is the part's own claim in its manifest (`harness.extensions`): plan 2d, which fetches manifests from the tools' releases, must carry it, so relais's published manifest declares `experimental = true` and `harness = ">=0.4"`.
- 2026-11-09 — enter does not install by itself: it runs the same command a
  person would type, with its plan and `Proceed?`, so the checklist adds no
  second consent path and its output is the same as a typed install.
- 2026-11-09 — an inline viewport drawn on stderr, not the alternate
  screen: the checklist is short. stdout must be a terminal too, because
  crossterm writes the viewport's cursor-position query there; with stdout
  redirected the checklist does not open, so a redirected stdout stays
  empty.
- 2026-11-09 — dependencies: ratatui 0.30 and crossterm 0.29 (amont-fleet's
  versions; ratatui needs rustc 1.88, grh's floor is 1.89); signal-hook at a
  release at least two weeks old; rexpect 0.7.1 (MIT OR Apache-2.0, MSRV
  1.85) as the pty test's dev-dependency. `cargo deny check` gates each.
- 2026-11-09 — audit's `install` line changes for fixtures where router was
  recommended (Claude Code set up): intended, recorded in
  `expected-new.json`; per-part verdicts are unchanged, so the 0.3.0 outcome
  diff still reads 0.

## Verification

- Phase 1: a manifest with `experimental = true` under `harness = ">=0.3"`
  is refused; under `>=0.4` it loads; a fixture catalog where a part that is
  not experimental needs an experimental one is refused by
  `Catalog::checked`, naming both; `grh install --all --yes` in the
  fixture installs every published part but router and its plan names it;
  `grh install router --yes` installs it; the audit goldens stay green with
  `expected-new.json`'s install lines updated where router was recommended,
  and the JSON carries `"experimental": ["router"]`; `scripts/e2e.sh` passes
  with router installed by name.
- Phase 2: the 13 goldens unchanged; `assess` is called by `run` and by a
  unit test without printing.
- Phase 3: state-machine tests: pre-ticks follow the verdicts; an
  experimental part is never pre-ticked; space on an installed or
  unpublished line leaves it unchanged and sets a footer notice naming why; ticking a part ticks its needs, and
  unticking a needed part is refused with the notice; enter with nothing
  ticked stays open; q, esc and ctrl-c give their actions. `TestBackend`
  renders at 80x24, at 60 columns and at 80x10 (scrolled, cursor visible): the experiment text is on the router
  line, no line is cut mid-word, and under no colour the selection is still
  visible (reversed) and every state has its word.
- Phase 4: off a terminal (`stdin` null, as every cli test runs) a bare
  `grh install` still exits 2 with the listing; `--yes` alone too; stdout redirected to a file gives the listing, exit 2,
  file empty; stderr redirected gives the listing in the file, exit 2. A pty
  test (harness answers `ESC[6n`) runs `sh -c 'grh install; stty -a'` on a fixture: the checklist appears, `space`,
  `enter`, the printed `grh install …` command, the plan, `n` → `nothing
  changed`, exit 0, receipt unchanged; a second run sends two enters in one write and `Proceed?` still waits; a further run sends `q` → exit 0; a
  third sends ctrl-c → exit 130; a fourth sends SIGTERM to grh → exit 143;
  after each the terminal is out of raw mode (`stty -a` in the pty shows
  `icanon echo`). A 6-row pty gives the listing and exit 2.
- Phase 5: each carried item has its test or its observable change (the
  80-column lines, the git-config failure test, the renamed variable).
- Phase 6: `cargo test`, clippy `-D warnings`, `cargo deny check`, `cargo +1.89 check`, release binary size 0.3.0 vs 0.4.0 recorded,
  `scripts/e2e.sh` against the real releases; a person-like run in a real
  terminal of `grh install` in a scratch repository (pre-ticks, toggle,
  enter, `n`) is recorded with what it showed; `install.sh` run from a
  scratch HOME prints the new next steps.

## Outcome

On day 7 after 0.4.0: `scripts/outcome-diff.sh` 0.3.0 vs 0.4.0 over
~/Developer/Perso gives 0 per-part verdict differences and 0 `unknown`; and
in `scripts/e2e.sh`, `grh status --json` after `install --all` lists no
router, which appears only after `grh install router`.
Roll back on a verdict difference or a terminal left in raw mode: run
`install.sh` pinned to 0.3.0 (receipts are unchanged).

<!-- panel: repos=grh adds=screen,contract reviewers=backend,lang:rust,ux-research,game-ux,tui,architect body-sha=f470a8ae3b5b -->
