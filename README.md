# grh

One command to install, audit and remove the Green Harness parts.

```sh
curl -fsSL https://raw.githubusercontent.com/greenharness/grh/main/install/install.sh | sh
grh audit                          # what fits this repository
grh install                        # pick the parts in a checklist
grh install greengate greenpiste   # or name them
grh install --all --repo           # or every part but the experiments
grh uninstall                      # take every part back out
```

`install.sh` installs the `grh` binary only, after checking its SHA-256. It
refuses to install when it cannot check. Nothing else changes until you run
`grh install`, which prints its plan and asks before it acts (pass `--yes`
when stdin is not a terminal).

On a terminal, a bare `grh install` opens a checklist pre-ticked with what
`grh audit` recommends here (`j/k` move, `space` ticks, `enter` shows the
plan, `q` cancels). Ticking a part ticks what it needs. Off a terminal it
lists the parts and exits 2.

## Parts

| Part | Tool | What it decides |
|---|---|---|
| decisions | aval | what is decided: architecture decision records, resolved |
| greengate (was gate) | amont | what a commit or a push may carry: git hooks |
| greenpiste (was guardrails) | amont-agent | what an agent's tool call may do: a Claude Code hook |
| router | relais | which model runs a bounded task, verified and accounted (an experiment: never ticked for you, left out of `--all`) |
| conform | adr-lint | which decided code rules a tree breaks, by lint (needs decisions) |
| attest | git-attest | CI skips what your laptop already checked (needs greengate) |
| workflow | skills and agents | plans reviewed by a panel, one worktree per task (needs greenpiste and greengate) |
| design | Duro | the Duro design system, themed by your tokens (extension, not published yet) |

There are no profiles (removed in 0.3.0): use the checklist, name the parts,
or pass `--all`. An experimental part is installed only when you name or tick
it.
`grh audit` prints the `grh install` line that fits this repository. The old
names `gate` and `guardrails` still work, with a warning.

Every part is described by a manifest, `greenharness.toml` v2
([docs/extension-manifest.md](docs/extension-manifest.md)): the catalog's
parts and `grh install --extension <dir>` read the same type, and grh's
code names no tool.

Each tool still installs and runs on its own. grh calls each tool's own
install and uninstall commands. It never edits a tool's configuration itself.

## What grh changes, and how it takes it back

- **Binaries.** grh downloads each binary from the tool's GitHub release, at a
  version and SHA-256 pinned in `catalog.toml`, into `~/.local/bin` (or
  `$GRH_BIN_DIR`). A binary already on your PATH is *adopted*: grh uses it and
  never removes it.
- **Integrations.** grh runs the tool's own command, for example
  `amont-agent install --write`. It records the command and its undo, and
  checks the result through the tool's own query (`doctor --json`,
  `list --json`). When that query cannot answer, grh changes nothing.
- **Skills and agents.** grh copies them from
  [greenharness/skills](https://github.com/greenharness/skills) into
  `~/.claude`. A file you changed is kept.
- **The receipt** at `~/.local/state/grh/receipt.json` lists all of it.
  `grh uninstall` removes only what the receipt says grh placed.

## Exit codes

| Code | Meaning |
|---|---|
| 0 | done |
| 1 | a step failed; the receipt keeps the failed entries |
| 2 | usage, or a confirmation was needed and not given |
| 3 | a part is not published yet (design today); the other parts installed |
| 4 | the part is not installed (`grh <part> …`) |
| 5 | another grh holds the receipt lock |
| 129 | the terminal hung up while the checklist was open |
| 130 | interrupted |
| 143 | terminated while the checklist was open |

## Licence

MIT. See `LICENSE`.
