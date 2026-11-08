# Changelog

What an upgrader gets, in sentences. The release workflow refuses a tag whose
section is missing here.

## Unreleased

## v0.2.0

### Changed

- **Every part is a manifest.** `greenharness.toml` v2 (harness 0.2) is the
  catalog's part type and what `grh install --extension <dir>` reads. It adds
  `[tool]`, `[tool.release]`, `[tool.machine]` and `[tool.repo_level]` (with
  `install`, `uninstall`, `requires` and a `status` query), `[tool.keep_binary]`,
  `[[repo_file]]` and `[bundle]`, and `needs`, `enhances`, `aliases`, `hint`
  and `published` in `[extension]`. A v1 manifest (Duro's) still installs;
  v2 sections need a `harness` that 0.1 does not satisfy. See
  `docs/extension-manifest.md`.
- **grh's install path names no tool.** Status queries, the keep-binary rule
  and attest's CI workflow come from the manifests; a level whose `requires`
  command is missing is skipped with a gap that names it. The catalog refuses
  unknown `needs`/`enhances`, cycles, alias collisions, parts missing from its
  `order` and edges that point up it.
- **Extensions use the same path.** An extension may carry a tool; its status
  and keep-binary commands are recorded at install, so uninstall works after
  its directory is gone. A manifest the catalog does not pin lists, on stderr
  and before asking, the binary it would download and every command and repo
  file path it would use.
- The part `gate` is now `greengate`, and `guardrails` is now `greenpiste`. `grh install gate`, `grh uninstall gate` and `grh gate ...` still work: each resolves to `greengate` through its alias and prints one warning on stderr naming the command to use instead, e.g. `gate is now greengate: run grh uninstall greengate`. For `grh gate ...` the warning comes before the part runs, so stdout stays the tool's alone.
- The receipt is version 2. The first command that holds the receipt lock (install, uninstall) copies `receipt.json` to `receipt.json.pre-0.2` (rewritten from the current file whenever that file is older than version 2, so it always holds the receipt a downgrade needs), rewrites entries recorded under `gate` or `guardrails` to the new names, and says so on stderr, even under `--quiet`. `grh status` and `grh audit` never write the receipt: they read an alias in memory. A version 1 receipt still loads. A save that changes nothing no longer touches the file.
- The keep-binary check reads a tool's state with the same words as every status query (`installed` counts as present, like `present` and `partial`), so it keeps a binary in more cases, never fewer.
- Uninstalling a tool whose binary is already gone keeps its receipt entry and says so, instead of running an undo whose result grh cannot check. Reinstall the part, then `grh uninstall` it again.
- `grh audit` still names the parts `gate`, `guardrails` and so on in its rows; they move to the new names with the profiles.

### Downgrading to 0.1.19

grh 0.1.19 refuses a version 2 receipt. To go back: `grh uninstall` each part that `receipt.json.pre-0.2` does not list (installed under 0.2.0), install grh 0.1.19, then restore `receipt.json.pre-0.2` over `receipt.json` (both in `$XDG_STATE_HOME/grh`, else `~/.local/state/grh`).

## v0.1.19

### Changed

- gate and guardrails download from their new homes, `greenharness/greengate`
  and `greenharness/greenpiste` (Green Harness ADR-0064), and the catalog pins
  their first releases from there. That is gate's amont 2.3.1 (from 1.64.0;
  amont 2.0 changed how a push runs its gates, see its CHANGELOG) and
  guardrails' amont-agent 3.23.1 (from 3.22.0). The binaries keep their names,
  `amont` and `amont-agent`. Run `grh install gate guardrails` to upgrade.

## v0.1.18

### Changed

- Pins skills 0.5.0: the workflow part gains the `plan-picture` skill. A plan carries a `## Picture` section: an ASCII drawing of the mechanism it builds, or an SVG page beside the plan with a short-hash link when ASCII is too small, or `none: <why>`. `plan-review` runs its checker before seating the panel, the backend reviewer holds the picture to the phases, and `worktree-task` copies a linked picture beside the landed plan; each step is skipped when the skill is not installed. Run `grh install workflow` once after upgrading.

### Fixed

- Release downloads survive a failing GitHub route: a release URL is retried up to 3 times (about 1 s, then 4 s apart) on a 5xx, 429 or network error, then fetched from the other route (the web URL or the API asset endpoint). With `GH_TOKEN` or `GITHUB_TOKEN` set the API route goes first and the token is sent to the API host only, never to the host an asset redirects to; without one the web URL goes first. `GRH_API_BASE` points the API route at another host. Each URL is downloaded once per process, and the failure message names the URL once with the last status and the routes tried.

## v0.1.17

### Changed

- Pins skills 0.4.0: the workflow part gains generic `plan-review-ui-design` and `plan-review-react` seats, and `plan-review` fills their design-system lookup slot from an installed `design-system-lookup` agent (the `design` extension ships one). Run `grh install workflow` once after upgrading.
- Pins adr-lint 0.17.0 for `conform`: its TypeScript sync targets `@aval-adr/eslint-config` (0.16.0 or later), which now also ships the `base`, `effect` and `tests` presets.

## v0.1.16

### Changed

- Pins skills 0.3.11: the plan-review skill binds a seat that a file type earned to its slice of the plan. It runs `plan-sha --block` with the `--slice <role>` that `plan-panel` prints, pastes both stdout lines, re-runs every seat the hook names stale, and knows amont-agent 3.23.0's slice refusals. With an older amont-agent it runs the plain block. Run `grh install workflow` once after upgrading.

## v0.1.15

### Changed

- Pins skills 0.3.10: the backend plan reviewer checks a plan's declarations first. When the panel's classes (`areas=`) have no `contract` and the plan adds a module, crate or repository, or changes a contract another repository consumes, it raises a blocking "add `adds=contract`"; for a new screen or flow without `screen`, "add `adds=screen`". It names only values amont-agent accepts. Run `grh install workflow` once after upgrading.

## v0.1.14

### Changed

- Pins amont-agent 3.22.0 (from 3.16.1): a plan's review panel is seated from the file types of the files each phase names, when every phase names one, and from the repositories' areas otherwise, with advice to name them; `adds=` takes `screen` and `contract`. A fresh install with the earlier pin got an amont-agent that refuses those values.
- Pins skills 0.3.9 (and 0.3.8, never pinned): the plan template asks each phase for the files it touches and for `adds=screen` / `adds=contract` where no file shows them; `plan-review` seats by file type, with the area table as the fallback and the case of an older amont-agent first; the backend reviewer holds a plan seated from its files to both declarations; `worktree-task` builds a Delta brief from the reviewed head (0.3.8). Run `grh install guardrails workflow` once after upgrading.

## v0.1.13

### Changed

- Pins skills 0.3.7: `relais-verified-push`, `worktree-task` (F6) and `tag-release` push with `amont land` from amont 2.0, which refuses a bare `git push` whose push gates have not run ("push refused: … have not run on") and runs them before git connects. A `v*` tag lands with `amont land --tag`. With an older amont, or none, the skills keep their earlier paths. Run `grh install gate workflow` once after upgrading.

## v0.1.12

### Fixed

- The workflow part now lists the `agent-model-fallback` skill that skills 0.3.6 added. Without it `grh install workflow` placed the reviewer agents on `model: inherit` but left out the policy and helper their launchers cite. Run `grh install workflow` once after upgrading.

## v0.1.11

### Changed

- Pins skills 0.3.6: every first-party reviewer agent now inherits the root session model, and the plan-review and worktree-task skills pass `model: "opus"` and retry a reviewer once on the root model when the preferred one is unavailable (new `agent-model-fallback` skill). Run `grh install workflow` once after upgrading.

## v0.1.10

### Changed

- Pins amont-agent 3.16.1: a plan-review reviewer resumed with `SendMessage` counts once its message starts with the new `<<<PLAN …>>>` block, so round 2 and a delta can resume a reviewer instead of launching a fresh one.
- Pins skills 0.3.5: the plan-review skill says so. Run `grh install workflow` once after upgrading.

## v0.1.9

### Changed

- Pins amont-agent 3.16.0: `amont-agent open <url|path>` opens a page through the machine's opener (`amont.agent.opener`), and the `ask-unopened-page` rule refuses a question that names a local page nobody opened (ADR-0058).
- Pins skills 0.3.4: worktree-task F7 opens the app with `amont-agent open`. Run `grh install workflow` once after upgrading.

## v0.1.8

### Changed

- Pins amont-agent 3.15.1: `push-preview` no longer diffs from a stale remote-tracking ref (merged, squash-merged, or on a rewritten branch), and an identical before/after pair is approved as `unchanged` with no question (ADR-0057).
- Pins skills 0.3.3: worktree-task F7 skips the question for an `unchanged` preview, with a fallback when the hook does not approve it. Run `grh install workflow` once after upgrading.

## v0.1.7

### Changed

- Pins amont-agent 3.14.0: a branch that changes only its plan needs no code-review profile (implementation-review still checks the record), and a pointer plan declares through its own `## Code review` line, so an approval from the session carries for cross-repository plans. It also carries pulldown-cmark 0.13.6 (RUSTSEC-2026-0433).
- Pins skills 0.3.2: the worktree-task pointer template carries this repository's `## Code review` line, copied from the canonical plan. Run `grh install workflow` once after upgrading.

## v0.1.6

### Changed

- Pins amont-agent 3.13.0: a code-review profile's `approve` given earlier in the session carries to a fix commit without a re-run, and implementation-review's review block is read from the prompt's header only.
- Pins skills 0.3.1: the code-review agents and worktree-task say the brief goes by path because it is long, and that the prompt's own lines come first. Run `grh install workflow` once after upgrading.

## v0.1.5

### Changed

- Pins amont-agent 3.12.0. `amont-agent board` reads a squash-merged branch as merged instead of in flight, `work-overlap` names the plan the other session's branch added, the hook costs less on every write, and a `git patch-id` that exits early no longer kills the hook. Run `grh install guardrails` after upgrading.

## v0.1.4

### Changed

- Pins skills 0.3.0 and amont-agent 3.11.0. A plan can name, in a `## Code review` section, the expert profiles that review its code before the push. The workflow part now installs their seven `code-review-<role>` agents, and amont-agent's push hook waits for each declared profile's review. Run `grh install workflow` once after upgrading, so the new agents are placed.

### Fixed

- `grh --bogus --help` prints the help and exits 0: help wins over an unknown
  flag (cli.help.help-and-version), and `grh install --bogus --help` prints
  install's help. An unknown flag alone still exits 2, and a part's own
  arguments (`grh decisions --help`) still reach the part.

## v0.1.3

### Changed

- **workflow places skills 0.2.0**: `worktree-task`, `diagnosing-bugs` and
  `tag-release` first check what other sessions are doing in the repository
  (`amont-agent board`, amont-agent 3.10.0 or later) and what just landed,
  so a session messages the session that already works a change instead of
  redoing it. Without the board, each runs its git check alone. Run
  `grh install workflow` once after upgrading.
- **guardrails pins amont-agent 3.10.0**, which adds `amont-agent board`,
  the session activity record and the advisory `work-overlap` rule.

## v0.1.2

### Changed

- **workflow places skills 0.1.1**, whose `worktree-sweep` stops what still
  runs in a deleted worktree, not only language servers: test browsers, dev
  servers, shells, and orphaned `chrome-devtools-mcp` servers. A live Claude
  Code session and its own tools are never touched, and a worktree a session
  works in is held. Run `grh install workflow` once after upgrading.

## v0.1.1

### Changed

- **router pins relais 0.39.0**, whose `doctor --json` reports its Claude
  Code integration, so grh adopts or installs it instead of reporting it
  unknown.
- relais 0.39.0 stops writing `relais-verified-push` and
  `relais-architecture-conflict`; grh now places them with gate and
  decisions. After upgrading relais, run `grh install gate decisions` once.
- `grh audit --json` and `grh status --json` share one output switch.

## v0.1.0

### Added

- **`grh install <part|profile>…`** installs any subset of the Green Harness
  parts (decisions, gate, guardrails, router, conform, attest, workflow) and
  the profiles solo, team and full, with what each part needs. It runs each
  tool's own install command, places the published skills and agents, adopts
  what is already installed, and arms a repository only with `--repo`.
- **`grh uninstall [part…]`** removes exactly what grh placed, through the
  tool that placed it, and never what it adopted.
- **`grh audit`** reads a repository and recommends parts and a profile.
- **`grh status`**, and **`grh <part> …`**, which runs the part's own command.
- **Extensions** (`greenharness.toml`, `docs/extension-manifest.md`) and
  `grh install --extension <dir>`.
