# Changelog

What an upgrader gets, in sentences. The release workflow refuses a tag whose
section is missing here.

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
