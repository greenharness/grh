# grh's part model is data: full reviews

Round 1 (body 9c931c014b20): all six approve-with-changes; 31 of 32 findings accepted and merged by hand into Behaviour, Verification, Code review and Outcome (several seats edited the same lines); po#5 (phase order) needed no edit. Round 2 (body fd3a8cdcd0a2): approve tui, po, architect, unix; approve-with-changes backend, lang:rust.

Carried into implementation:
1. tui: the migration notice shows under `--quiet` (as `Ui::fail` does) and names `receipt.json.pre-0.2`.
2. unix: write `receipt.json.pre-0.2` through `write_atomic`; keep an existing copy, never overwrite it.
3. po, architect, backend: rollback drill: list what changed under 0.2.0 with `grh status`, `grh uninstall` the parts installed under 0.2.0, restore `receipt.json.pre-0.2`, check `grh status` under 0.1.19; the CHANGELOG says so.
4. architect: uninstall runs only the commands the consenting install recorded, never anything read afresh from a directory.
5. backend, lang:rust: the Phase 3 grep exempts `claude` in `src/extension.rs`'s `[[mcp]]` registration functions, the one host command kept.
6. backend: count the alias warnings from the session transcripts (`gate is now greengate` lines), or drop the count.
7. backend: the Picture's keep_binary line shows `{ args, state, shims }`, and the picture gains `requires` and the receipt migration.
8. backend: for a part the catalog pins, the catalog's status/keep-binary spec wins over the receipt's recorded copy.
9. lang:rust: the new `ToolInstall` fields are `#[serde(default)]`; an entry written by 0.1.x without them takes its spec from the catalog part by canonical name; Phase 5 uninstalls a seeded 0.1.19 entry.
10. lang:rust: v2 keys need a `harness` that 0.1.0 does not satisfy; a missing `harness` or one 0.1.0 satisfies is refused with them.
11. po: a numeric bar for the alias count, or none.
12. unix: SIGINT between the copy and the save, then a rerun: the backup is byte-identical and the receipt migrated once.
13. unix: `grh status --json | head -c1` on the migrating run exits 0 with no broken-pipe message.

Tokens and time: round 1 architect 30k/24 s, backend 33k/44 s, unix 31k/36 s, po 31k/32 s, lang:rust 44k/60 s, tui 32k/51 s; round 2 tui 38k/13 s, po 37k/10 s, architect 37k/13 s, unix 38k/21 s, backend 41k/32 s, lang:rust 57k/34 s. codex: not installed.

