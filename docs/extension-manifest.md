# Extension manifest v1

An extension is a directory with a `greenharness.toml` at its root. `grh install --extension <dir>` reads it (ADR-0048, ADR-0050). Everything it places is recorded in grh's receipt under the part `ext:<name>`, so `grh uninstall` removes it again.

Unknown keys, at any level, are refused (exit 2). So is a `harness` requirement grh does not satisfy: grh's harness version is 0.1.

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
