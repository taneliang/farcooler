# Versions and releases

Far Cooler is five things that have to agree, and one that deliberately does not.

## One number

`[workspace.package] version` in `Cargo.toml` is the version of the whole
system. Nothing else holds a literal:

| Component | Where the version comes from |
| --- | --- |
| CLI, daemon | `CARGO_PKG_VERSION`, stamped with the commit sha into `FARCOOLER_BUILD` by `crates/protocol/build.rs` |
| Mac app | `build-app.sh` stamps the **bundled** `Info.plist` from `scripts/version.sh` |
| iOS app | `generate-project.py` sets `MARKETING_VERSION`; `Info.plist` says `$(MARKETING_VERSION)` |
| Relay | Not versioned with the rest — see below |

`scripts/version.sh` is the only implementation of "what version is this":

    ./scripts/version.sh          0.2.0
    ./scripts/version.sh build    1284             # commit count, for the stores
    ./scripts/version.sh channel  dev|beta|release
    ./scripts/version.sh display  0.2.0 (beta 3)   # what a person is shown
    farcooler --version           0.2.0+a1b2c3     # what components report to each other

The last one is not this script. The stamp components check against each other
is computed once, in `crates/protocol/build.rs`, and both binaries report it —
a second implementation would be the drift the script exists to prevent.

The build number is `git rev-list --count HEAD`. App Store build numbers must
increase forever, and the commit count is the one monotonic integer this repo
already has — no counter file, no state, no coordination between a laptop and
CI, and it names the commit it came from.

### Why not per-component versions

Because the failure they produce is not a build error. A Mac app talking to a
daemon built from different source does not fail to launch; it behaves like a
bug you already fixed is still happening. This project has hit that twice —
once when `build-app.sh` copied a stale binary instead of building one, and once
when a Mac drove a Linux host over ssh running older source. `HostProbe`'s build
comparison and the app's version handshake exist because of it.

So there is no such thing as an app-only patch release. A change anywhere is a
change to the system.

- **MAJOR** — the ssh protocol between an app and a daemon breaks. The only
  break a user has to do anything about.
- **MINOR** — features.
- **PATCH** — fixes.

## Channels: dev, beta, release

A beta of `0.2.0` **is** `0.2.0` — same code, same marketing version, same App
Store entry. What differs is which build someone has, and that is what the
channel answers. Without it, a bug report says "0.2.0" and there is no way to
know which one they had.

The channel is derived from the tag on `HEAD`, never from a flag someone
remembers to pass:

| HEAD | Channel | Shown as |
| --- | --- | --- |
| clean tag `v0.2.0` | `release` | `0.2.0` |
| clean tag `v0.2.0-beta.3` | `beta` | `0.2.0 (beta 3)` |
| untagged, or a dirty tree | `dev` | `0.2.0 (dev a1b2c3)` |

Both apps stamp it into their `Info.plist` at build time (`FarCoolerChannel`,
`FarCoolerDisplayVersion`), `AgentKit.AppVersion` reads it back, Settings shows
it under the account row, and it is what each device reports to the relay. A
tag containing `-beta.` is published as a GitHub prerelease, so it never becomes
the download someone lands on.

An unstamped bundle reports `dev`, deliberately: defaulting the other way would
let a hand-made build pass itself off as a release.

## Cutting one

    ./scripts/release.sh 0.2.0            # a release
    ./scripts/release.sh 0.2.0 --beta 1   # a TestFlight beta of the same code
    git push origin main v0.2.0

`--beta` only tags — it makes no commit, because the version has not moved. A
second beta of a version already in `Cargo.toml` is a tag and nothing else.

That bumps `Cargo.toml`, commits, and tags. `.github/workflows/release.yml`
builds every component **from the single commit the tag points at** and attaches
the results to a GitHub release:

- Linux `x86_64` and `aarch64`, static musl, in the `dist/<arch>-linux/` layout
  `farcooler host install` already reads
- `Far Cooler.app`, signed and notarised if the secrets are present
- iOS to TestFlight

Signing is conditional throughout. Without a Developer ID the Mac job still
produces a working ad-hoc-signed app, which is what a contributor gets and what
must keep working for them.

## The relay is different

`services/relay` deploys continuously from `main` (`.github/workflows/relay.yml`),
not with releases. It has to: an App Store release takes days to review and can
never be rolled forward on demand, so there will always be phones running an app
from months ago that the relay must still serve.

**Additive only.** New routes and new optional fields; never a removed route,
never a changed meaning, never a new required field on an existing request. A
route that genuinely must change shape becomes `/v2/…`, and `/v1/…` keeps
working until the analytics say nobody is calling it.

## CI

`.github/workflows/ci.yml` runs on every push and pull request:

| Job | What it protects |
| --- | --- |
| `rust` (Linux + macOS) | clippy at -D warnings and tests, against a real tmux — a fake one would agree with whatever this code believed |
| `swift` | the full `build-app.sh`, plus a check that the bundle's stamp matches the workspace |
| `ios` | project generation then build, so a file added to AgentKit cannot compile locally and be missing from the app |
| `relay` | typecheck, `vitest` inside workerd against a real D1, and a `wrangler deploy --dry-run` |

GitHub Actions specifically because this repo is public: public repos get
unlimited minutes on standard runners, macOS included. A private repo would burn
its allowance on macOS runners in days — they bill at 10× Linux — and that is
the point at which CircleCI's open-source plan would be worth the move.

## Secrets

None of these are in the repo, and none can be. They are set once in the GitHub
repository settings, except the relay's, which live only in Cloudflare.

| Secret | Used by | Notes |
| --- | --- | --- |
| `WORKOS_CLIENT_ID` | app builds | Public by design — it names the app, not the bearer. Kept out of the repo only so a fork can point at its own WorkOS project. |
| `MACOS_CERTIFICATE`, `MACOS_CERTIFICATE_PASSWORD`, `MACOS_SIGN_IDENTITY` | Mac release | Developer ID, base64 `.p12` |
| `APPLE_ID`, `APPLE_APP_PASSWORD`, `APPLE_TEAM_ID` | notarisation | app-specific password |
| `APP_STORE_KEY_ID`, `APP_STORE_ISSUER_ID`, `APP_STORE_KEY_P8` | TestFlight | base64 `.p8` |
| `CLOUDFLARE_API_TOKEN`, `CLOUDFLARE_ACCOUNT_ID` | relay deploy | Plus a repository **variable** `RELAY_DEPLOY=true` — see below |

### Why the gates use `vars` and `env`, never `secrets`

`if: ${{ secrets.X != '' }}` looks like it gates a job on having a credential.
It does not. The `secrets` context is not available to `if` at job or step
level, so the expression compares `''` against `''` and is **always false** —
silently, and failing closed, which is why it looks like it works. The relay
job simply never deploys, and a release publishes an unsigned, un-notarised app
with no failure signal.

So: the relay job gates on the repository variable `RELAY_DEPLOY`, and the
signing steps gate on job-level `env` that maps the secret to a boolean, which
`if` can read.

The relay's own secrets — the APNs key, the WorkOS API key, the FCM service
account — are set with `wrangler secret put` and are never visible to a
workflow. A deploy does not need them, and a workflow that could print a secret
is a workflow that eventually does.
