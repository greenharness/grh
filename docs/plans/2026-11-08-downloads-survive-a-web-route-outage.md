---
status: active
branch: fix/download-fallback
repos: [grh]
adrs: []
---
# Downloads survive a web-route outage

## Picture

```
 today                    after this plan
 -----                    ---------------
 grh --> web route        grh --> 1st route --504--> retry x3
           |                        |                   |
           `--504--> fail           `--200--> sha256    `--> 2nd route
                                                              |
                          token: API first                    `--> fail
                          none:  web first                         once
```

A release download now retries, then tries the other GitHub route, so one
failing route no longer fails `grh install`; the sha256 pin still decides.

## Goal

On 2026-11-08 GitHub's web download route
(`github.com/<owner>/<repo>/releases/download/<tag>/<file>`) answered 504
for every asset, even a missing one, while the API asset endpoint answered
200 for the same files. grh used only the web route, once, so every install
failed, CI e2e on `main` included. Downloads become token-aware, retried,
and fall back between the two routes.

## Non-goals

- No change to what is pinned or how it is verified: the sha256 pin decides.
- `install.sh` (curl) is not changed here.

## Code review

- grh: lang:rust, backend (unix left out: only one error message changes)

## Behaviour

- A GitHub release URL is fetched by route order: with `GH_TOKEN` or
  `GITHUB_TOKEN` set, the API asset endpoint first (release by tag, asset
  by name, `Accept: application/octet-stream`), then the web URL; without
  a token, the web URL first, then the API anonymously.
- Each route is tried up to 3 times on a 5xx, 429 or transport error, with
  backoff (about 1 s, then 4 s) and a per-request timeout; a 404 or other
  4xx does not retry on that route.
- The token is sent only to the API host, never to the host an asset
  redirects to. A 401 or 403 with a token retries the API without it.
- Each URL is downloaded once per process; later callers reuse the bytes.
- The error names the URL once, the last status and the routes tried.
- `GRH_API_BASE` points the API route at a test server; without it, the API
  route exists only when the release base is GitHub.

## Phases

- [x] Phase 1 — routes, retries, cache, message, tests: `src/fetch.rs`,
  `CHANGELOG.md` (`src/skills.rs` needs no change: it downloads through
  `download_verified`)
- [ ] Phase 2 — release grh, then rebase the skills 0.5.0 pin PR (#24)

## Decision log

- 2026-11-08 — API first only with a token, because anonymous API calls
  are limited to 60 an hour per IP and an install makes about 12; web
  first otherwise. Chosen by the person over web-first-always.

## Verification

- Unit tests in `src/fetch.rs` against local servers, covering: web 504 ×3
  then API 200; token → API first, the redirect target never sees
  `Authorization`; 401 and 403 → anonymous retry; 404 and oversized →
  one request per route; 429 and a dropped connection → retried; a
  relative `Location`; the rejected-token message; the same URL twice →
  one request → observed: `cargo test` "44 passed" (lib) and "30 passed"
  (cli), 0 failed.
- The process cache: `downloads_over_http_from_a_local_server` with
  `download` building a cache per call (the regression the review-fix run
  introduced) → red "Connect error after 3 attempts"; with
  `process_cache()` → green.
- `make check` → exit 0 → observed: exit 0 (27.1 s).
- From the laptop during the live 2026-11-08 web-route outage, curl of the
  aval asset → 504; `grh install full workflow --yes` into a scratch home
  with only `git` on PATH:
  - first patch, no token → exit 0, 117 s, 8 ✓ 0 ✗; with a token →
    exit 0, 30 s, 8 ✓ 0 ✗, the token in 0 output lines;
  - final patch, no token → exit 0, 103 s, 8 ✓ 0 ✗, 8 stderr retry lines
    and 4 route-switch lines.
