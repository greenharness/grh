# Changelog

What an upgrader gets, in sentences. The release workflow refuses a tag whose
section is missing here.

## Unreleased

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
