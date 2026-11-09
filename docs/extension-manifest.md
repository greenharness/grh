# Extension manifest (v1, and v2 from harness 0.2)

An extension is a directory with a `greenharness.toml` at its root. `grh install --extension <dir>` reads it (ADR-0048, ADR-0050). Everything it places is recorded in grh's receipt under the part `ext:<name>`, so `grh uninstall` removes it again.

Unknown keys, at any level, are refused (exit 2). So is a `harness` requirement grh does not satisfy: grh's harness version is 0.2. A v1 manifest (harness `>=0.1`) still loads; a harness that only matches 0.1, such as `^0.1`, is refused.

```toml
[extension]
name = "duro"            # required: letters, digits, - _ .
version = "1.0.0"        # required
harness = ">=0.1"        # optional: ">=0.1", "^0.1", "0.1", ">=0.1, <1"

[[skill]]
path = "skills/hello"    # a directory with SKILL.md (or a single .md file)

[[agent]]
path = "agents/helper.md"

[[mcp]]
name = "duro"
command = "duro-mcp"
args = ["--stdio"]

[[session_context]]
command = "duro context"

[[doctor]]
command = "duro doctor"

[conform]
eslint = "duro lint"
```

## Sections

- `[extension]`: `name`, `version`, optional `harness`. Operators: `>=`, `>`, `<=`, `<`, `=`, `^`, `~`; a bare version means `^`.
- `[[skill]]` `path`: relative to the manifest directory, never absolute and never `..`. A directory is copied to `<claude>/skills/<dir name>/`; a file becomes `<claude>/skills/<stem>/SKILL.md`.
- `[[agent]]` `path`: a file, copied to `<claude>/agents/<file name>`.
- `[[mcp]]`: registered with `claude mcp add --scope user <name> -- <command> <args…>`; the undo is `claude mcp remove --scope user <name>`. Both are recorded as a tool-install.
- `[[session_context]]` `command`: grh prints the `SessionStart` hooks block to add to `settings.json` and does not write that file.
- `[[doctor]]` `command`: run with `sh -c` by `grh status`, which shows the first line of its output.
- `[conform]` `eslint`: grh prints the line; it changes nothing.

`<claude>` is `$CLAUDE_CONFIG_DIR`, else `~/.claude`. A file already there that grh did not place is left alone unless `--replace`.

## v2 (harness 0.2)

The catalog's `[[part]]` entries are manifests of this same type. v2 adds the following; any of them requires `harness = ">=0.2"` (a requirement that 0.1 satisfies is refused).

- `[extension]`: `summary`, `needs`, `enhances`, `aliases` (part names), `hint` (printed, never run) and `published` (default true).
- `[tool]`: `bin`; `[tool.release]`: `repo`, `version`, `sha256` per target triple.
- `[tool.machine]` and `[tool.repo_level]`: `install`, `uninstall` (with `{bin}`), `requires` (host commands that must be on PATH) and `status`, either `{ args, exit = { present, absent } }` or `{ args, json = ["installed", "state"] }`.
- `[tool.keep_binary]`: `args`, `state` (JSON path), `absent`, `shims` (JSON path), `binary`, `warning`: when a placed binary must stay.
- `[[repo_file]]`: `dirs` (the first that exists in the repository, else the last), `path`, `content`; relative and inside the repository.
- `[bundle]`: `skills`, `agents` taken from grh's pinned skills bundle.

## What `--extension` shows and records

A manifest the catalog does not pin installs through the same path as a catalog part, so it may carry a `[tool]`. Before asking for consent, grh prints on stderr, even under `--quiet`, the binary a `[tool.release]` would download (its repository and version, which the same manifest supplies with its sha256), every command it would run (doctor commands included), (install, uninstall, status, keep-binary) one per line with shell-quoted arguments, every `[[repo_file]]` path (not its content) and every `requires` entry. Off a terminal without `--yes` it then exits 2 and changes nothing. `--repo` allows `[[repo_file]]` and `[tool.repo_level]`; a repo file is written only inside the repository (no `..`, no absolute path, no link that leaves it).

The level's `status` and the `keep_binary` spec are recorded in the receipt at install time, so uninstall undoes the tool and confirms it absent after the extension directory is gone.
