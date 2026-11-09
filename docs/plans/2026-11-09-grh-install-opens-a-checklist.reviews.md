# Reviews: grh install opens a checklist

## Full reviews (reference)

Round 1 (body ba1940e162f8): approve-with-changes from tui (25k/30 s), game-ux (29k/49 s), backend (25k/30 s), architect (33k/55 s), ux-research (31k/69 s), lang:rust (45k/89 s); backend, lang:rust, architect and ux-research first failed on a 429 rate limit and were relaunched once with no model. 33 findings: 31 accepted (22 by plan-apply, the overlaps and escaped patches merged by hand), 2 rejected (tui#2: the 0.3.0 listing heading already names `grh install --all`; tui#7: q prints `nothing changed`, ctrl-c prints `grh: interrupted`).

Round 2 (body f470a8ae3b5b): approve-with-changes from game-ux (30k/13 s), tui (30k/16 s), ux-research (31k/25 s), backend (34k/40 s), lang:rust (36k/54 s). Every round-1 finding resolved; no high or blocking finding.

Carried into implementation:
1. backend, tui: unregister the signal-hook handlers (their `SigId`s) before `install::run`, so SIGTERM or SIGHUP at `Proceed?` or during the install keeps its default effect; ignore restore errors after a hang-up (EIO); a pty step sends SIGTERM at `Proceed?` and expects 143 with the receipt unchanged.
2. lang:rust: 143 and 129 are new codes in the published exit table: `Exit::Terminated` and `Exit::HungUp` in `src/exit.rs`, `codes_match_the_published_table` extended, the module comment reworded, the README exit table updated.
3. lang:rust: the pty test signals grh's own pid (`pgrep -P <sh pid>`), never the process group, so `stty -a` still runs.
4. lang:rust: `assess` returns `Ok(None)` outside a git work tree; `run` maps it to today's usage error; the checklist reads `None` as no repository (not a match on `Exit::Usage`).
5. game-ux: an empty line at `Proceed?` means N (a late enter after the drain), stated in the docs and covered by the doubled-enter test.
6. game-ux (optional): label the printed command, e.g. `running: grh install …`, only if the line stays copy-pasteable as a command.
7. ux-research: under 8 rows or 40 columns, print `terminal too small for the checklist (needs 8 rows, 40 columns): enlarge it or name the parts` before the listing; the 6-row pty test asserts it.
8. backend: the "can never be pulled in" bullet sits at the level of its siblings in the landed plan.
9. game-ux: `q cancel` changing nothing is shown after exit only; no change.

Tokens and time are per seat above. codex: not installed.

