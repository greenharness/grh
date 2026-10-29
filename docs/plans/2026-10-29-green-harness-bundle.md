---
canonical: github:fredericrous/decisions/docs/plans/2026-10-29-green-harness-bundle.md
phases: [5]
status: active
branch: feat/initial
---
# Green Harness bundle

Pointer. The plan and its review panel live in decisions, with ADR-0050
(harness.installer). This repository carries phase 5: `grh` itself (catalog,
install, uninstall, audit, status, dispatch, the receipt and the extension
manifest v1), its install script and its release workflow.
| e2e, real releases, fake claude | `scripts/e2e.sh target/debug/grh` (local, macOS x86_64) | install.sh from this build; `install full workflow --repo` places every part; status shows decisions, gate, guardrails, router, conform installed and placed; `grh decisions --version` runs aval; bare uninstall disarms the repo, removes mcp servers and leaves HOME as before | rc 0: "every part installed and placed", "nothing left behind" (router's machine integration stays a reported gap until relais ships `doctor --json` integration) |
| adopted integration | `amont-agent install --write`, then grh install and uninstall guardrails | hook entry still in settings.json | as expected |
| adopted binary | `amont` symlink in a second PATH dir, grh install and uninstall gate | none placed in GRH_BIN_DIR; symlink still there | as expected |
| unit and integration tests | `cargo test` | pass | 30 + 29 passed |
| lint | `cargo clippy --all-targets -D warnings`, `cargo fmt --check` | clean | clean |

## Implementation review

approve after three deltas (104k + 132k + 143k + 150k). Fixed: Partial never turns an adopted integration into grh's; an existing mcp server is adopted, and a failed add leaves no undo; the gate guard keeps amont while the template dir bakes it, or when the answer is unreadable or empty; a foreign attest.yml is never claimed; install talk goes to stderr; cargo deny; tests for failing uninstalls, unreachable repos and audit fixtures; the e2e with a fake claude and the adopted cases.
deliberate: `is_tty` and `color` (src/ops.rs) are facts about the terminal, and `required` (src/manifest.rs) only picks a field reader's error text; `json_out` in audit carries a holds-until for v0.1.1.
deliberate: the e2e allows an emptied `{}` settings.json left by amont-agent's own uninstall (holds-until in scripts/e2e.sh; an amont-agent follow-up).
