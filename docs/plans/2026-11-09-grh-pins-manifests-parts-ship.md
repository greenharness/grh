---
status: active
branch: feat/pinned-manifests
repos: [grh, decisions]
adrs: [ADR-0064]
---
# grh pins manifests its parts ship

## Review panel

📄 Full reviews: [2026-11-09-grh-pins-manifests-parts-ship.reviews.md](2026-11-09-grh-pins-manifests-parts-ship.reviews.md)

👉 **Decide:** none — approve if pinned manifests built into grh, with a `pins` example and a CI re-check, is the right close of build step 2.
📍 grh, decisions · nothing built · next: Phase 1, the pinned form. Panel: backend, lang:rust, platform, architect, unix (opus-5-5).
**Changed by review:** pins resolved before `checked()`; a pin also checks `[tool.release]` version and repo; the shell scripts became a `pins` example on grh's fetch code; typed download failures so unreachable (3) is never a mismatch (1).
**Verdicts:** round 1 approve-with-changes 5; round 2 approve-with-changes 5; final-body check approve 1, approve-with-changes 2; 0 rework; 16 findings carried into implementation (listed in the full reviews).

## Picture

```
 today                         after this plan
 -----                         ---------------
 catalog.toml                  catalog.toml
   [[part]] every part,          [[part]] a part with no manifest of
            inline                        its own: inline, as today
                                 [[pinned]] name, repo, tag, asset,
                                            sha256
                               manifests/<name>.toml: the pinned copy,
                                 fetched once by the pins example,
                                 built into grh by build.rs

 grh starts: parse the         grh starts: for each pin, check
   inline parts                  sha256(copy) = pin, parse the copy,
                                 check its name and version, add it
                                 to the parts; no network, as today
                               CI: the pins example re-downloads every
                                 pin and fails on a mismatch
```

A part's repository can ship its own `greenharness.toml`; grh pins it by
release and sha256 and builds the checked copy in, so grh stays offline and
each grh release is one tested set.

## Goal

Build step 2d of ADR-0064 (programme `decisions/docs/programmes/gate-guardrails`):
"grh's catalog is data: for each part, the pinned location, version and
sha256 of its manifest… For a part whose repository ships no manifest yet,
the catalog holds one in its stead." Today every part is inline in
`catalog.toml`. This plan adds the pinned form and the tools around it, and
proves them on a fixture release. No real part moves yet: the person chose
that greengate and greenpiste ship their manifests in build step 3, in their
first releases under the new names, so this plan changes nothing a person
sees. It closes build step 2 (grh goes generic).

## Non-goals

- Moving any real part to a pin (step 3 does greengate and greenpiste; the
  others wait for their own programme).
- Fetching manifests at run time (the person chose the built-in copy).
- A grh release: with no pin in the shipped catalog the binary behaves as
  0.4.0; the first release with pins is step 3's.
- The tool repositories' release workflows (step 3 adds the asset there;
  this plan documents what they must ship).

## Code review
- grh: backend, lang:rust, platform, unix, architect (tui left out: no output a person sees changes)
- decisions: none — markdown only (the proposal counted grh's Phase 3 files)

## Behaviour

- **A pin in `catalog.toml`.** A `[[pinned]]` entry names a part and where
  its manifest lives: `name`, `repo` (`owner/name` on GitHub), `tag` (the
  release tag, which must start with `v`), `asset` (`greenharness.toml`
  unless given) and `sha256` of the asset's bytes. A part is either inline
  (`[[part]]`) or pinned, never both; the catalog refuses a name in both,
  and two pins with one name, naming it. Pinned parts take part in
  `order`, `needs`, `enhances`, aliases, retired names and every other
  catalog check exactly as inline ones do.
- **Resolved before the checks.** `RawCatalog` gains `pinned: Vec<Pin>`
  (`serde(default)`, `deny_unknown_fields`). `Catalog::parse_with(source,
  copies)` parses the raw catalog, resolves every pin into a part from its
  copy, and only then runs `checked()`, so no path builds a `Catalog`
  without the checks and a pinned part is checked like an inline one.
  `Catalog::load` passes the embedded copies, or with `GRH_CATALOG` the
  `manifests/` directory beside that file.
- **The pinned copy.** `manifests/<name>.toml` holds the asset's bytes.
  `build.rs` (no new dependency) writes a sorted `OUT_DIR/manifests.rs` of
  `(name, include_bytes!(absolute path))` for every `manifests/*.toml`,
  ignores other files, treats a missing directory as zero copies (git keeps
  no empty directory, and the shipped catalog has no pin yet), and emits
  `rerun-if-changed=manifests`; `cargo package` carries both (no `include`
  list). `.gitattributes` marks `manifests/*.toml` `-text`, and a copy is
  hashed as bytes before it is parsed.
- **Checked at load, no network.** A pin is refused (exit 1, nothing done)
  when: it has no copy; `sha256(copy)` differs from the pin (both sums
  named); its tag does not start with `v`; the copy does not parse or check
  as a manifest; its `[extension] name` is not the pin's `name`; its
  `[extension] version` is not the tag without `v`; it has a tool whose
  `[tool.release] version` is not that version or whose `[tool.release]
  repo` is not the pin's `repo` (so a pin never vouches for tarballs of
  another release); it says `published = false`. A copy with no pin is
  refused too, so a stale file cannot ride along.
- **The `pins` example** (`cargo run --locked --example pins -- …`, grh's
  own code, not a shell script, so it reuses `src/fetch.rs`: the web route
  then the API route with `GH_TOKEN`/`GITHUB_TOKEN` sent as a header, 3
  attempts with 5xx/429 retried, `GRH_RELEASE_BASE` for `file://`
  fixtures, and `write_atomic`). The asset URL is
  `{base}/{repo}/releases/download/{tag}/{asset}`. Standard output carries
  data only, every diagnostic goes to stderr, and `-h` documents the exit
  codes: 0 done, 1 mismatch, missing asset or refusal, 2 usage, 3 host
  unreachable after the retries.
  - `pin [--replace] <name> <repo> <tag> [asset]` downloads the asset,
    checks it as a manifest (name, versions, repo, as the load does),
    writes `manifests/<name>.toml` atomically, and prints the `[[pinned]]`
    block. It refuses (exit 1) to overwrite a copy with other bytes unless
    `--replace` is given.
  - `check` downloads every pinned asset, compares its sha256 with the
    pin and with the copy, prints one line per pin, checks them all, then
    exits 1 if any mismatched (or 3 if any host was unreachable, never
    reported as a mismatch). With no pin it prints `0 pinned manifests`
    and exits 0.
- **CI** runs `cargo run --locked --example pins -- check` once, in its own
  `pinned manifests` job on ubuntu-latest, with `GH_TOKEN` in its env as
  the e2e job has; not in the `rust` OS matrix.
- **What a tool repository ships** (documented in
  `docs/extension-manifest.md`, applied in step 3): a `greenharness.toml` at
  its root with no sums; its release job writes `[tool.release]` (`repo`,
  `version`, and `sha256` for each unix target's tarball) into the asset
  after the tarballs are built and before `SHA256SUMS`, uploads it with the
  other assets (so it is checksummed and, for greengate, attested), and its
  guard checks `[extension] version` against the tag beside `Cargo.toml`.
  A released manifest is never re-uploaded under the same tag: a fix ships
  under a new tag, so grh's pins and CI stay valid.

## Phases

- [x] Phase 1 — the pinned form: `[[pinned]]` in the raw catalog,
  `Catalog::parse_with` resolving pins before `checked()`, the embedded
  copies, the load-time checks, `GRH_CATALOG`'s sibling `manifests/`:
  `build.rs`, `Cargo.toml` (build script), `.gitattributes`,
  `src/catalog.rs`, `src/manifest.rs`, `tests/cli.rs`,
  `tests/fixtures/pinned/`.
- [x] Phase 2 — the `pins` example and CI: `examples/pins.rs`,
  `src/fetch.rs` (`download` returns a typed failure instead of one `Exit::Failed`
  string: not found = a route answered 404, the API listing has no such
  asset, or a `file://` read gave NotFound; unreachable = every route
  tried still failed with a transport error, 5xx or 429 after its
  retries; other = the rest. `check` maps not found and other to exit 1
  and unreachable to exit 3, and exits 1 if any pin mismatched or is
  missing even when another host was unreachable; no second download
  function),
  `tests/pins.rs`, `.github/workflows/ci.yaml`.
- [x] Phase 3 — docs and the programme map: `docs/extension-manifest.md`,
  `catalog.toml` (header comment), `CHANGELOG.md` (Unreleased),
  `decisions/docs/programmes/gate-guardrails/map.md` (build step 2 done,
  next step 3).

## Decision log

- 2026-11-09 — the person chose: a pinned manifest is fetched once by a grh
  script and built into the binary, checked against its sha256 at load and
  by CI, so grh keeps working offline; not fetched at run time.
- 2026-11-09 — the person chose: this plan builds the mechanism only, proven
  on a fixture release; greengate and greenpiste ship their manifests in
  build step 3, in their first releases under the new names.
- 2026-11-09 — no grh release for this plan: the shipped catalog has no pin,
  so 0.4.0's behaviour is unchanged; step 3's release carries the first
  pins.
- 2026-11-09 — the copies are embedded by a `build.rs` that lists
  `manifests/*.toml`, not by a new crate (`include_dir`) or a hand-kept list.
- 2026-11-09 — the `pinned manifests` CI job calls `cargo run --example pins
  -- check` directly, not `make pins-check`: make turns every failing exit
  into 2 and would hide 3 (an outage, a warning on a pull request); the make
  target is the same command for running it locally.
- 2026-11-09 — implementation went through relais; Phase 1's candidate was
  salvaged by hand (its reviewer could not be spawned, and it lacked the
  refusal unit tests, which were written by hand); Phase 2's first candidate
  was revised once (a retry-delay variable reachable in release builds, the
  pin checks duplicated outside the catalog).
- 2026-11-09 — the pin and check tools are a cargo example reusing
  `src/fetch.rs`, not shell scripts (panel: shell TOML parsing, the sha256
  tool on macOS, retries through a GitHub outage and atomic writes all come
  free from grh's own code).

## Verification

Observed on the finished tree (2026-11-09):

- Phase 1: `a_pinned_part_resolves_and_takes_part_in_order_and_needs`,
  `a_pinned_part_with_a_tool_of_this_release_resolves` and
  `each_pin_refusal_names_the_part_and_why` (every refusal: no copy on both
  paths, sha256 with both sums, tag without `v`, unparsable, name, version,
  `[tool.release]` version and repo, `published = false`, a copy with no
  pin, inline and pinned, two pins, an alias clash) → green; removing the
  `published` check and the `[tool.release] repo` check each turned it red
  ("the catalog was accepted"); `a_pinned_part_installs_from_a_fixture_release_beside_gh_catalog`
  → green; the build has no `manifests/` directory and the shipped catalog
  loads; the 13 audit goldens unchanged.
- Phase 2: `tests/pins.rs` (pin writes and prints a block the catalog
  accepts, refuses to overwrite without `--replace`, a missing asset leaves
  `manifests/` unchanged; check green, two byte-off pins both named,
  `http://127.0.0.1:1` unreachable, `file://` without the asset not found, a
  local server's listing without the asset not found, 503 on every attempt
  unreachable, a mismatch plus an unreachable pin gives 1; `pin` and the
  catalog refuse a wrong `[tool.release]` repo with the same reason; the
  built example: `-h` 0, an unknown flag 2, `check` on a catalog with no pin
  prints `0 pinned manifests`) → green; the existing `src/fetch.rs` tests
  pass with no assertion edited; `grep GRH_RETRY_DELAY_MS src examples`
  finds nothing.
- Phase 3: `docs/extension-manifest.md` "Shipping a manifest" names the root
  file without sums, the release job writing `[tool.release]`, the asset in
  `SHA256SUMS`, the version guard and the no-re-upload rule; the programme
  map says build step 2 is done and names step 3 (decisions, `relais
  programme check` exit 0); `scripts/e2e.sh` against the real releases
  installs every part and leaves HOME as it found it (exit 0); 201 tests,
  clippy `-D warnings` and fmt green.

- Phase 1: unit tests load a fixture catalog with one pin and its copy (the
  part is in `parts`, `order` names it, an inline part `needs` it and
  resolves); each refusal gives exit 1 with its message: no copy, sha256
  mismatch (both sums), a tag without `v`, unparsable copy, name mismatch,
  `[extension] version` not the tag, `[tool.release] version` or `repo` not
  the pin's, `published = false`, a copy with no pin (embedded and sibling
  paths), a name both inline and pinned, two pins with one name, an alias
  clash with an inline part; each refusal test fails when its check is
  removed; a build from a clone with no `manifests/` directory succeeds and
  the shipped catalog loads; the 13 audit goldens are byte-identical; a cli
  test installs a pinned part from a `file://` fixture release through
  `GRH_CATALOG` and its sibling `manifests/`; editing a copy rebuilds the
  binary.
- Phase 2: `tests/pins.rs` runs the example against a `file://` fixture
  release: `pin` writes the copy and prints only the block on stdout, which
  the catalog then accepts; it refuses to overwrite a differing copy
  without `--replace`; a missing asset exits 1 and leaves `manifests/`
  unchanged; `check` is green on a matching asset, exits 1 and names both
  parts when two assets are each one byte off, exits 3 (not 1) when the
  base is `http://127.0.0.1:1` (no listener; a `file://` read error cannot
  tell unreachable from missing) and 1 when a `file://` base lacks the
  asset, and prints `0 pinned manifests` on the shipped
  catalog; the tests pass on both CI OSes; the `pinned manifests` job runs
  on the PR.
- Phase 3: the docs section names the root file without sums, the release
  job writing `[tool.release]`, the asset in `SHA256SUMS`, the version
  guard and the no-re-upload rule; the map's Notes say build step 2 is done
  and name step 3 next; `scripts/e2e.sh` against the real releases installs
  every part and leaves the scratch HOME as it found it (exit 0).

## Outcome

The first pins land with step 3: `cargo run --example pins -- check` green on the
release that pins greengate and greenpiste, and `grh status` gives the same
per-part answers before and after the move (`scripts/outcome-diff.sh`, 0
verdict differences).

<!-- panel: repos=grh,decisions adds=contract reviewers=backend,lang:rust,platform,architect,unix body-sha=576c54b9f367 -->
