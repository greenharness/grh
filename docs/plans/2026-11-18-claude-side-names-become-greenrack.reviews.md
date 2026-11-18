# Full reviews: The Claude-side names become greenrack

## Full reviews (reference)

**Round 1** (opus-5-5):
- tui, approve-with-changes (26k, 27 s): reason `greenrack-*` (accepted); 80-column check (accepted).
- lang:rust, approve-with-changes (30k, 33 s): fixture from npm pack with asserts (accepted); both-scopes golden (accepted); exact golden diff (accepted, later replaced by codex#2); keep `duro` in PARTS (accepted); published=false (rejected: duplicate of backend#4); fresh npm pack (accepted).
- backend, approve-with-changes (33k, 58 s): Phases heading (accepted); dotfiles in repos (accepted); rollback (accepted); published=false note (accepted); grep string (rejected: superseded by unix#1); duro-ds from an app's .mcp.json (accepted).
- unix, approve-with-changes (28k, 44 s): JSON-escaped guard (high, accepted); atomic write + jq (high, accepted); `claude mcp get` (accepted).

**Round 2:** every finding was medium or low, so all were carried; the contradictions were later fixed in the body.
- rust (35k, 14 s): step 5 path; receipt check after restore; `claude mcp get` on an unknown name.
- tui (30k, 15 s, approve): step 7 should use `claude mcp get`.
- unix (31k, 14 s): atomic rollback; `claude mcp get`; full template path.
- backend (40k, 22 s): step 5 contradiction; rollback trees; dotfiles reason; README row.

**Codex**, rework:
- uninstall cannot restore a backup into a pruned directory (blocking, verified at `src/uninstall.rs:393`, applied);
- goldens are legacy and skip signals (high, applied);
- rollback loses consumed backups and the template (high, applied);
- t09 closed too early (high, applied).

**Delta after Codex**: backend applied the step 5/7 contradictions, unix seat, two objectives and Verification 1. unix#8 (high) was merged into step 7, rust#1 (high) added the golden recorder note. Carried into implementation:
- tui: restore error names its next command; SHA256SUMS;
- unix: temp file + rename in the restore fix; directory trees restored by `mv dir dir.failed && mv dir.restore dir`; full template path;
- rust/backend/tui: the stray paren; step 8 via a follow-up PR; quote `"$P/package"`; check `claude mcp get` on an unknown name.

**Final bind**:
- rust approve (47k, 10 s);
- backend approve (59k, 17 s);
- tui approve (39k, 12 s);
- unix approve-with-changes, medium only, carried (42k, 13 s).
