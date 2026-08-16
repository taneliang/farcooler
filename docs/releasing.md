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
    ./scripts/version.sh channel  local|canary|preview|stable
    ./scripts/version.sh display  0.2.0 (preview 3)  # what a person is shown
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

## Channels: local, canary, preview, stable

A preview of `0.2.0` **is** `0.2.0` — same marketing version, different build.
What differs is which one someone has, and that is what the channel answers.
Without it a bug report says "0.2.0" and there is no way to know which.

The names are the ones a developer-tool audience already knows: Rust ships
`nightly`/`beta`/`stable`, Chrome ships a `canary`, Zed ships `preview`.

| Channel | Who has it | How it is built |
| --- | --- | --- |
| `local` | you, on your own machine | `cargo build`, no CI |
| `canary` | internal TestFlight | every push to `main`, automatically |
| `preview` | external TestFlight | Promote → `preview`, tagged `v0.2.0-preview.N` |
| `stable` | everyone | Promote → `stable`, tagged `v0.2.0` |

All four install side by side: separate bundle identifier, runtime directory,
database, tmux server, binary name and relay. A canary cannot see a stable
install's fleet, which is what makes it safe to run one on the machine your real
work lives on.

The channel is derived from a tag, never from a flag someone remembers to pass:

| Tag | Channel | Shown as |
| --- | --- | --- |
| clean tag `v0.2.0` | `stable` | `0.2.0` |
| clean tag `v0.2.0-preview.3` | `preview` | `0.2.0 (preview 3)` |
| `FARCOOLER_CHANNEL=canary`, set by CI | `canary` | `0.2.0 (canary a1b2c3)` |
| untagged, or a dirty tree | `local` | `0.2.0 (local a1b2c3)` |

Canary is the one that cannot be derived: a commit on `main` carries no tag, so
CI states it outright. Forgetting to yields `local`, which is the safe
direction — a build that cannot prove what it is stays isolated from every other
channel's data.

**Which tag, though.** Promotion puts two on one commit — preview and stable have
different bundle identifiers, so a stable release is a *rebuild* of the
preview's source rather than a re-upload of its binary. `git tag --points-at
HEAD` then returns both, sorted, and `v0.2.0` comes before `v0.2.0-preview.3`.
So `version.sh channel` takes an optional tag, and the promotion workflow passes
what it just created via `FARCOOLER_TAG`. Without it a promoted commit would
report `stable` forever, and anyone rebuilding the preview from it would get a
binary that installs at the stable path and writes the stable runtime directory.

**A channel is a separate installation**, not a label: its own bundle
identifier, runtime directory, database, tmux server, binary name, service unit
and relay, so all four coexist on one machine and none can see another's state.
See `docs/superpowers/specs/2026-08-11-release-channels-design.md`.

Both apps stamp it into their `Info.plist` at build time (`FarCoolerChannel`,
`FarCoolerDisplayVersion`), `AgentKit.AppVersion` reads it back, Settings shows
it under the account row, and it is what each device reports to its relay. A tag
containing `-preview.` is published as a GitHub prerelease, so it never becomes
the download someone lands on.

An unstamped bundle reports `local`, deliberately: defaulting the other way
would let a hand-made build pass itself off as a release.

## Cutting one

Local and canary are automatic. Preview and stable are buttons — the **Promote**
workflow, run from the Actions tab, choosing a channel and typing the version you
believe you are shipping.

There are no channel branches. Everything happens on `main`, and a tag names
what a build is.

| | What runs it | What it tags |
| --- | --- | --- |
| local | a build on your machine | nothing |
| canary | every push to `main` | nothing — canary has no compatibility promise to record |
| preview | Promote → `preview` | `v<version>-preview.<highest + 1>` |
| stable | Promote → `stable` | `v<version>` on the preview's commit |

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

**Renaming a version in flight** costs one pull request and the next preview
press: bump `Cargo.toml`, promote, and the counter restarts at `preview.1`
because that build genuinely is the first preview of the new version. The abandoned tags stay
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

## The relay: one per channel

There are three, and each gets its code the way its channel does
(`.github/workflows/relay.yml`):

| Trigger | Target |
| --- | --- |
| push to `main` touching `services/relay/**` | canary, continuously |
| Promote → `preview` | preview, at the tagged commit |
| Promote → `stable` | stable, at the tagged commit |
| Actions → Relay → Run workflow | any of them, from any ref |

Four deployments rather than one channel-aware relay, because a preview lives in
its own WorkOS environment: `accounts.id` **is** the WorkOS user id, so the
accounts do not carry across anyway, and one relay serving two environments
would have to choose which credentials to verify a token against before it knows
who the caller is. Every per-channel value was already a var or a secret, so
`services/relay/src` needs no idea any of this exists.

`APNS_TOPIC` is the one that must differ by more than value: it has to equal the
**receiving app's bundle id**, and each channel has its own. A preview relay holding
the stable topic means APNS rejects every push and nothing says so.

**Additive only.** New routes and new optional fields; never a removed route,
never a changed meaning, never a new required field on an existing request. A
route that genuinely must change shape becomes `/v2/…`, and `/v1/…` keeps
working until the analytics say nobody is calling it.

That rule is what lets a months-old app keep working — not the deploy cadence.
Production used to deploy on every push to `main` for that reason, which
confused the two: additivity is what serves old apps, and shipping every commit
straight to the people running them is a separate decision. Canary is the
continuous one now, and `workflow_dispatch` still puts any commit on the release
relay in one press when something is actually broken.

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
stable release, **one preview** in preview — that is where the protocol's shape is still
being discovered, and carrying every exploratory field to 1.0 is worse than a
tester having to update. Canary and local freeze nothing.

## CI

`.github/workflows/ci.yml` runs on every push and pull request:

| Job | What it protects |
| --- | --- |
| `wire` | that a client already in the field can still talk to this — the proto lint, its own self-tests, what `version.sh` answers, and that the four channels stay four WorkOS projects |
| `rust` (Linux + macOS) | clippy at -D warnings and tests, against a real tmux — a fake one would agree with whatever this code believed |
| `swift` | the full `build-app.sh`, plus a check that the bundle's stamp matches the workspace |
| `ios` | project generation then build, so a file added to AgentKit cannot compile locally and be missing from the app |
| `relay` | typecheck, `vitest` inside workerd against a real D1, and a `wrangler deploy --dry-run` |

GitHub Actions specifically because this repo is public: public repos get
unlimited minutes on standard runners, macOS included. A private repo would burn
its allowance on macOS runners in days — they bill at 10× Linux — and that is
the point at which CircleCI's open-source plan would be worth the move.

## Setting a repository up

Nothing below is in the repo, and none of it can be. It is set once in the
GitHub repository settings, except the relay's own secrets, which live only in
Cloudflare.

### Secrets

| Secret | Used by | Notes |
| --- | --- | --- |
| `MACOS_CERTIFICATE`, `MACOS_CERTIFICATE_PASSWORD`, `MACOS_SIGN_IDENTITY` | Mac release | Developer ID, base64 `.p12` |
| `APPLE_ID`, `APPLE_APP_PASSWORD`, `APPLE_TEAM_ID` | notarisation, and the iOS archive's `DEVELOPMENT_TEAM` | app-specific password |
| `APP_STORE_KEY_ID`, `APP_STORE_ISSUER_ID`, `APP_STORE_KEY_P8` | TestFlight, from both the canary and the release workflow | base64 `.p8` |
| `CLOUDFLARE_API_TOKEN`, `CLOUDFLARE_ACCOUNT_ID` | relay deploy | Workers Scripts edit, plus D1 edit for the migrations |

### Variables, which are the on switches

Repository **variables**, not secrets, for the reason two sections down. Neither
is set by default, so a fresh fork builds everything and ships nothing.

| Variable | Set to | Without it |
| --- | --- | --- |
| `RELAY_DEPLOY` | `true` | the relay job skips, and every channel's relay stays where it is |
| `CANARY_TESTFLIGHT` | `true` | the canary iOS job skips, and main reaches nobody's phone |
| `LOCAL_`/`CANARY_`/`PREVIEW_`/`STABLE_WORKOS_CLIENT_ID` | that channel's `client_…` | the app for that channel builds without a sign-in button, and warns |

The client ids are variables rather than secrets deliberately. One NAMES the app
rather than authenticating as it — the same value is committed in
`services/relay/wrangler.toml`, because the relay needs it too. Made a secret it
would be masked in the log, so a wrong one would print as `***` and read exactly
like a right one, on the axis whose failures are already the hardest to see.

### The `promote` environment

`promote.yml` runs its tag job in a GitHub environment named `promote`, which
does not exist until someone creates it. Give it required reviewers: it is the
pause that shows you the channel and the version you typed before a tag exists.
A tag cannot be taken back once a build from it reaches App Store Connect, which
will not accept that version again.

### Four WorkOS projects, one per channel

The same partition as the bundle identifiers, the runtime directories and the
relays, and it has to be, because `accounts.id` in the relay **is** the WorkOS
user id. Two channels sharing a project are not two environments that look
alike — they are the same accounts, and a preview build would be signing people
into the stable environment's identities.

Both halves have to agree per channel: the app's client id comes from the secret
above, and the relay it talks to holds the same value as `WORKOS_CLIENT_ID` in
its `[env.*]` block in `services/relay/wrangler.toml`.

No build ever names one of those secrets directly. `scripts/workos-client-id.sh`
derives which to read from `version.sh channel` — the same answer the bundle
identifier comes from — so a build's identity and the project it authenticates
against cannot disagree. **There is no fallback.** A repository holding only
`STABLE_WORKOS_CLIENT_ID` builds a preview with no client id at all rather than
with the stable one, and `scripts/workos-test.sh` exists to keep it that way.

A missing id is a warning, not a failure: the app still builds and works, minus
the sign-in button, which is what a fork gets. For a channel people install, the
warning says so in the run summary — sign-in is what buys notifications.

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
