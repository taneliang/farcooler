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

The channel is derived from a tag, never from a flag someone remembers to pass:

| Tag | Channel | Shown as |
| --- | --- | --- |
| clean tag `v0.2.0` | `release` | `0.2.0` |
| clean tag `v0.2.0-beta.3` | `beta` | `0.2.0 (beta 3)` |
| untagged, or a dirty tree | `dev` | `0.2.0 (dev a1b2c3)` |

**Which tag, though.** Promotion puts two on one commit — beta and release have
different bundle identifiers, so a release is a *rebuild* of the beta's source
rather than a re-upload of its binary. `git tag --points-at HEAD` then returns
both, sorted, and `v0.2.0` comes before `v0.2.0-beta.3`. So `version.sh channel`
takes an optional tag, and the promotion workflow passes what it just created
via `FARCOOLER_TAG`. Without it a promoted commit would report `release`
forever, and anyone rebuilding the beta from it would get a binary that installs
at the release path and writes the release runtime directory.

**A channel is a separate installation**, not a label: its own runtime
directory, database, tmux server, binary name and service unit, so all three can
coexist on one machine and none can see another's state. See
`docs/superpowers/specs/2026-08-11-release-channels-design.md`.

Both apps stamp it into their `Info.plist` at build time (`FarCoolerChannel`,
`FarCoolerDisplayVersion`), `AgentKit.AppVersion` reads it back, Settings shows
it under the account row, and it is what each device reports to the relay. A
tag containing `-beta.` is published as a GitHub prerelease, so it never becomes
the download someone lands on.

An unstamped bundle reports `dev`, deliberately: defaulting the other way would
let a hand-made build pass itself off as a release.

## Cutting one

Dev is automatic. Beta and release are buttons — the **Promote** workflow, run
from the Actions tab, choosing a channel and typing the version you believe you
are shipping.

There are no channel branches. Everything happens on `main`, and a tag names
what a build is.

| | What runs it | What it tags |
| --- | --- | --- |
| dev | every push to `main` | nothing — dev has no compatibility promise to record |
| beta | Promote → `beta` | `v<version>-beta.<highest + 1>` |
| release | Promote → `release` | `v<version>` on the beta's commit |

**The version is confirmed, never chosen.** The button reads
`[workspace.package] version` from `Cargo.toml` and refuses unless what you
typed matches. Moving the version is an ordinary pull request, because
`crates/protocol/build.rs` stamps `FARCOOLER_BUILD` from that literal — a tag
naming a version `Cargo.toml` disagreed with would have the binary and the plist
reporting different releases to each other, which is the exact failure the one
number rule exists to prevent.

Nothing else needs incrementing. `build-app.sh` stamps the bundled plist,
`generate-project.py` fills in `MARKETING_VERSION`, and `build.gradle.kts` reads
`versionCode`/`versionName` — all from `version.sh`, all at build time.

**Renaming a version in flight** costs one pull request and the next beta press:
bump `Cargo.toml`, promote, and the counter restarts at `beta.1` because that
build genuinely is the first beta of the new version. The abandoned tags stay
where they are — they name builds that really did reach TestFlight. A rename
must go *forward*; the button refuses a version sorting below one already
tagged, because App Store Connect will not accept a build number below one it
has, and that is unfixable afterwards.

One workflow does both the tagging and the build, and that is load-bearing: a
tag pushed with the default `GITHUB_TOKEN` does **not** trigger another
workflow. GitHub suppresses those events to prevent recursion, so a button that
tagged and stopped would silently never build anything.

`.github/workflows/release.yml` builds every component **from the single commit
the tag points at** and attaches the results to a GitHub release:

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

One relay serves every channel. It already has to be compatible with app builds
months old, so a second deployment would double that obligation to buy isolation
the `channel` column already provides.

## The wire has the same rule

The relay is not the only thing that must serve something older than itself. An
App Store review takes days and `farcooler host install` takes one command, so a
phone weeks behind talking to a daemon updated this morning is the ordinary
case.

**Additive only, same as the relay.** Tags are never reused, fields are reserved
rather than removed, meanings never change, and no existing message gains a
required field.

**Anything a client must know exists, it asks for by name.** Capabilities ride
in the `ServerHello`, so every client has the answer before its first request.
Never by comparing version strings.

**`PROTOCOL_VERSION` is reserved for a framing break** that additivity cannot
express — the envelope, the length prefix, the handshake itself. Bumping it is a
MAJOR release. Expected frequency: never. It is the fire alarm, not the
mechanism, and a number that is never pulled can be trusted when it is.

**A new field on an existing payload names its capability.** An older daemon
drops one as an unknown proto3 field and does the old thing, so the client
believes it asked for something it did not get — silently. Naming the capability
in `Request.required_capabilities` turns that into a refusal.

`./scripts/proto-lint.py` enforces the first rule against
`proto/baseline/<channel>.proto`, which the promotion workflow commits. How long
a field is frozen depends on the channel: **permanent** once it ships in a
release, **one beta** in beta — that is where the protocol's shape is still
being discovered, and carrying every exploratory field to 1.0 is worse than a
tester having to update. Dev freezes nothing.

## CI

`.github/workflows/ci.yml` runs on every push and pull request:

| Job | What it protects |
| --- | --- |
| `wire` | that a client already in the field can still talk to this — the proto lint, its own self-tests, and what `version.sh` answers |
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
