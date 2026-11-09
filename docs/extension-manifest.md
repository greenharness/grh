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

## Audit rules (harness 0.3)

A manifest that uses `[[recommend]]` or `[[conflict]]` declares
`harness = ">=0.3"`; a requirement that 0.2 satisfies is refused. In the
catalog they are `[[part.recommend]]` and `[[part.conflict]]`.

```toml
[[recommend]]
when = ["adr_folder", "!ci_present"]   # all must hold; "!" negates; [] always holds
verdict = "recommend"                  # "already here" | "recommend" | "not useful"
reason = "decisions live in {adr_folder}"

[[conflict]]
paths = [".husky"]                     # or git_config = "key", or foreign_hooks = true
label = "husky (.husky/)"              # optional: added to the audit's `hooks`
warning = "{manager} already manages git hooks here"   # optional
```

The first rule of a part whose `when` holds gives its audit row; no rule that
holds means `not useful`. Signals: `machine_present`, `repo_present` (the
part's declared status at that level; the part must declare one),
`adr_folder`, `claude_set_up`, `ci_present` (path lists in the catalog's
`[audit]` table), `language_supported`, `repo_file_here` (the part's first
`[[repo_file]]` exists in a CI directory) and `dep:<prefix>` (`package.json`
declares a dependency starting with the prefix). Any other signal is refused.

Every template (`reason`, `warning`, `label`) may use `{adr_folder}`, `{ci}`,
`{claude_files}` and `{part}`. A `git_config` conflict's `label` and
`warning` may also use `{hooks_path}` (the key's value), and a `paths`
conflict's `warning` may use `{manager}` (its rendered `label`). Any other
placeholder, or one used outside the context that gives it a value, is
refused when the manifest loads.

A `foreign_hooks` conflict takes no `warning` or `label`: grh adds each hook
it did not install to `hooks` itself, as `hook <name>`.

A rule using `adr_folder`, `claude_set_up` or `ci_present` needs the
catalog's `[audit]` list for it to name at least one path.

In `grh audit --json`, `signals` holds the repository facts only
(`adr_folder`, `claude_set_up`, `ci_present`, `language_supported` and each
`dep:<prefix>` a rule names). `machine_present`, `repo_present` and
`repo_file_here` belong to one part and are read in that part's row.

## v4 (harness 0.4)

- `[extension]` `experimental` (default false): the part is an experiment, opt-in everywhere. `grh install --all` and the `install` line of `grh audit` leave it out (audit still gives its row a verdict, names the part in the `experimental` list of `--json`, and says how to try it when it would recommend it); it installs by name. A part that is not experimental may not `needs` an experimental one. Requires `harness = ">=0.4"` (a requirement that 0.3 satisfies is refused).

## Shipping a manifest (pinned parts)

A part's repository can ship its own `greenharness.toml`. grh's catalog then
pins it instead of holding it inline:

```toml
[[pinned]]
name = "greengate"
repo = "greenharness/greengate"
tag = "v2.4.0"
sha256 = "…"            # of the asset's bytes
# asset = "greenharness.toml" (the default)
```

grh builds the pinned copy (`manifests/<name>.toml`) into its binary and
checks it when the catalog loads, with no network: the bytes must match the
`sha256`, the tag must start with `v`, `[extension] name` must be the pin's
name and `[extension] version` the tag without `v`, a `[tool.release]` must
name the pin's `repo` and that version, and `published` must not be `false`.
A copy with no pin, or a part both inline and pinned, is refused.

What the tool's repository ships:

- `greenharness.toml` at its root, with no sums: the tarballs it would
  describe do not exist yet when the file is committed.
- Its release job writes `[tool.release]` (`repo`, `version`, and `sha256`
  of each unix target's tarball) into the manifest after the tarballs are
  built, puts it in `dist/` before `SHA256SUMS` is computed, and uploads it
  with the other assets, so it is checksummed (and attested where the
  release attests `SHA256SUMS`).
- Its release guard checks `[extension] version` against the tag, beside the
  `Cargo.toml` check.
- A released manifest is never uploaded again under the same tag
  (`--clobber`): a fix ships under a new tag, so grh's pin stays valid.

In grh's repository: `cargo run --locked --example pins -- pin <name> <repo>
<tag>` downloads the asset, checks it as the catalog load does, writes the
copy atomically and prints the `[[pinned]]` entry; `make pins-check` (and the
`pinned manifests` CI job) re-downloads every pin and compares it with the
pin and the copy. The example's exit codes are its own: 0 done, 1 mismatch,
missing asset or refusal, 2 usage, 3 every failing host unreachable after
the retries (1 wins when both happen).
