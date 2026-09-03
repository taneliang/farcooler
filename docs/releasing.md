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
install's fleet, which is what makes it safe to run one on the computer your real
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
and relay, so all four coexist on one host and none can see another's state.
See `docs/superpowers/specs/2026-08-11-release-channels-design.md`.

Both apps stamp it into their `Info.plist` at build time (`FarCoolerChannel`,
`FarCoolerDisplayVersion`), `AgentKit.AppVersion` reads it back, Settings shows
it under the account row, and it is what each device reports to its relay. A tag
containing `-preview.` is published as a GitHub prerelease, so it never becomes
the download someone lands on.

An unstamped bundle reports `local`, deliberately: defaulting the other way
would let a hand-made build pass itself off as a release.

## A channel's app is its own app

| | stable | canary |
| --- | --- | --- |
| macOS bundle id | `com.farcooler.FarCooler` | `com.farcooler.FarCooler.canary` |
| macOS app | `Far Cooler.app` | `Far Cooler Canary.app` |
| iOS home screen | Far Cooler | FC Canary |
| login agent | `com.farcooler.daemon` | `com.farcooler.daemon.canary` |
| icon | the bear | the bear, amber CANARY banner |

All of it derives from `scripts/version.sh` — `app-suffix`, `app-name`,
`app-name-short` — for the reason the CLI's name and the URL scheme do: a rule
written once cannot disagree with itself. Stable keeps every bare name, because
that is what existing installs answer to.

The icons are drawn at build time by `scripts/icon-label.swift` from the one
source asset, into `build/` directories only. A generated icon written into the
repository would dirty the tree, and a dirty tree makes `version.sh channel`
answer `local` — so the act of labeling a canary would make every later step
build local instead, silently.

**One-time migration.** A canary Mac app installed before this change carries
the STABLE bundle identifier and is called `Far Cooler.app`. The next canary
installs beside it as `Far Cooler Canary.app`, leaving the old one looking
exactly like your stable app. Delete the old one — and if it ever registered the
daemon, switch that off first, or a launchd job under the shared label outlives
the app that created it.

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

## The Linux binaries every shipped app carries

The Mac app is the distribution unit for the Linux daemon too. Someone
installing Far Cooler onto a machine they ssh to has the app and nothing else —
no checkout, no Rust, no `scripts/build-linux.sh` — so `host_install.rs` looks
for `Resources/dist/<arch>-linux` inside the bundle before it looks anywhere
else, and there is no download fallback behind it. If the bundle has none, the
Install button cannot work at all, and the only way out is `--from <dir>` with
binaries built by hand.

So `.github/workflows/linux-binaries.yml` builds them once and **every workflow
that ships an installable app calls it**: `release.yml` for preview and stable,
`canary.yml` for canary. It is a file of its own precisely because it was
inline in `release.yml`, and canary — added later — therefore shipped without
it, dmg after dmg, with nothing failing:
`build-app.sh` only warns when `dist/` is empty, by design, so that a laptop
without musl can still build the app, and a warning in a job that passes is a
warning nobody reads. Both callers assert the binaries are present after
unpacking rather than trusting that warning.

`ci.yml` deliberately does not call it. That job builds the app to prove it
compiles and throws the bundle away; nothing installable comes out of it, and
paying for two musl builds on every pull request to check a `cp` loop is not
the trade.

The channel is stamped into those binaries like any other build, and for a
canary it has to be **said** rather than derived: a commit on `main` carries no
tag. `linux-binaries.yml` resolves the channel on the runner and passes it in
as `FARCOOLER_CHANNEL`, and `Cross.toml` is what carries it across the
container boundary — `cross` forwards `CARGO_*`, `CROSS_*` and `RUSTFLAGS` and
nothing else, so a variable not named there never reaches the build script.
Both halves are needed and the failure is silent in the usual direction: an
unstamped binary calls itself `local` and writes the Local channel's runtime
directory on every host it reaches, so a stable and a canary install on one
machine would quietly share a database.

`Cross.toml` earns its keep on a laptop too: `FARCOOLER_CHANNEL=preview
./scripts/build-linux.sh` reaches the build script the same way it does in CI,
where before it stopped at the container.

## Only a shipping build links the tunnel, and only the Mac's can

`farcooler-tailcat` compiles two ways. By default it is `stub.rs`, which fails
every tunnel with the code `no_tailcat`; that default is what lets `cargo
build` and `cargo test --workspace` work on a checkout with no Go toolchain,
and it is not meant to ship. The real backend is a Go archive built by
`scripts/build-tailcat.sh`, turned on with `--features tailcat` and linked with
a `-l static=tailcat` **scoped to a single `cargo rustc --bin` invocation** —
never a global `RUSTFLAGS`, which attaches the archive to every crate rustc
compiles and once turned a 47.6 MB archive into a 6.4 GB artifact.

Every platform that *serves* a tunnel has to do that at release time, and for a
while none of them did: `build-app.sh` built the daemon in the same plain
`cargo build` as the CLI, and `linux-binaries.yml` restated its build inline
rather than calling `scripts/build-linux.sh`. Both shipped a stub daemon on
every channel, while iOS — which *dials* a tunnel rather than serving one — was
wired correctly and made the feature look present.

**The Mac is fixed. Linux is not, and cannot be while its binaries are musl.**

`build-app.sh` now builds `farcoolerd` through its own `cargo rustc` with the
`darwin-arm64` archive linked, unconditionally, and `ci.yml` runs the bundled
binary to prove it (`scripts/tunnel-smoke.sh`). Unconditionally is the point: a
build that quietly fell back to the stub when Go was missing is precisely how
this went unnoticed for as long as it did, with every job green. It also needs
`-l framework=Security`, because Go's `crypto/x509` reaches the system roots
through it on darwin and nothing in the Rust graph links that framework.

### Why Linux still ships the stub

Not for want of trying, and not a toolchain problem — that part works. A Linux
daemon built with the archive linked **segfaults before it logs a line**:

```
dist/x86_64-linux/farcoolerd: ELF 64-bit LSB pie executable, x86-64,
  static-pie linked, Go BuildID=6BImF5uB-…
Segmentation fault (core dumped)
```

The cause is not this repository's Go code, and not the Rust link. A five-line
Go program with one exported function, built `-buildmode=c-archive` and linked
into a C program, crashes identically. Measured on linux/arm64 with a native
Go 1.25.1, one `main.go` and one `main.c`:

| archive built with | linked with | result |
| --- | --- | --- |
| glibc `gcc` | `gcc`, dynamic | works |
| glibc `gcc` | `gcc -static` | works |
| `musl-gcc` | `musl-gcc -static` | SIGSEGV |
| `musl-gcc` | `musl-gcc`, dynamic | SIGSEGV |

So it is musl, not static linking, and not PIE — which is worth stating plainly
because `file` says `static-pie` right next to the Go build id and that is the
obvious thing to blame. `strace` puts the fault inside the Go runtime's own
startup, immediately after it maps its profiler hash buckets, as a null
dereference:

```
mmap(NULL, 1439992, …) = …   prctl(… " Go: profiler hash buckets")
--- SIGSEGV {si_signo=SIGSEGV, si_code=SEGV_MAPERR, si_addr=NULL} ---
```

Go prints nothing, even under `GOTRACEBACK=system`: it dies before the runtime
can report anything. The same crash reproduces on x86_64 with an archive built
natively on a Linux runner, so it is neither architecture-specific nor an
artifact of cross-compiling the archive from a Mac.

musl is not incidental to this release — it is the reason a self-installed
daemon runs on Debian, Alpine and a NAS alike, and "GLIBC_2.38 not found" is
the failure it exists to prevent. So the choice is a real one and has not been
made: keep musl and leave Linux tunnels unavailable, or move the Linux daemon
to glibc (the table says static glibc does work with Go) and pay for it in
NSS — `getaddrinfo` and `getpwnam` in a static glibc binary need the matching
`libnss_*` at runtime, which is precisely the "works on my distribution" class
of failure musl was chosen to end.

Until that is decided, the Linux release keeps the stub deliberately, and a
tunneled runner on Linux answers `no_tailcat` — one named error, at one call
site, rather than a daemon that dies on boot.

### What a machine needs to build the Mac's daemon

- **Go**, which every workflow that runs `build-app.sh` installs with
  `actions/setup-go` pinned by `go-version-file: crates/tailcat/go/go.mod`
  rather than trusting the runner image — an image that drops Go fails with an
  error that never mentions Go.
- Nothing else. `darwin-arm64` is host-native: no cross-compiler, no
  `-isysroot`.

`scripts/build-linux.sh` additionally wants `x86_64-linux-musl-gcc` or
`aarch64-linux-musl-gcc` — both names load-bearing twice over, since
`build-tailcat.sh` hands them to cgo as `CC` and `.cargo/config.toml` names the
same binaries as Rust's linkers — from `brew install
FiloSottile/musl-cross/musl-cross`. What it produces is the segfaulting binary
above, so do not ship it.

The check that matters is behavioural, and `scripts/tunnel-smoke.sh` is it:
start the packaged `farcoolerd` against a scratch `FARCOOLER_HOME` and
`authorized_keys`, and confirm the tunnel it tries to serve comes back as
anything other than `no_tailcat`. A size or `file` check is only a proxy — and
the Linux crash is exactly why: that binary passes every proxy there is.

## Updating a Mac that already has one

Cutting a release gets a build onto GitHub. It does nothing for the Mac that
installed last month — that Mac has to find out on its own, which is Sparkle,
wired in per channel.

**What a person sees.** Every shipping channel — stable, preview, canary —
checks once a day (`SUScheduledCheckInterval` is `86400`) and asks before
installing (`SUAutomaticallyUpdate` is false everywhere, canary included: Far
Cooler is a tool people work inside, and a build that replaced itself unasked
mid-session would be a worse failure than an update noticed a day late).
"Check for Updates…" sits in the app menu right after "About Far Cooler," for
checking on demand — greyed out rather than absent on a build with no feed, so
its presence never implies an update channel that isn't there.

**`local` never checks.** `version.sh feed-url` returns nothing for `local`,
so `build-app.sh` stamps no `SUFeedURL`, and `Updates.swift` never constructs
an `SPUStandardUpdaterController` without one — not merely disabled, never
created. A local build is somebody's working tree; an updater offering to
replace it with a build from CI would be losing work, not updating it.

**Where the feed is.** `https://updates.farcooler.com/<channel>/appcast.xml`,
from `version.sh feed-url`. `scripts/appcast.py` writes the XML Sparkle reads
there; `scripts/appcast-test.sh` checks that its `sparkle:` attributes resolve
to Sparkle's own namespace, not merely that the file parses; CI signs the
enclosure with Sparkle's own `sign_update`.

**Where the dmg is** differs by channel:

| Channel | The enclosure points at |
| --- | --- |
| stable, preview | the GitHub release asset itself — its `browser_download_url`, read back from `action-gh-release`'s own output rather than reconstructed |
| canary | an R2 object, keyed per build: `canary/Far Cooler-<build>.dmg` |

Stable and preview need no second copy: a GitHub release asset is immutable
once attached, and `sign_update` signs the dmg's *bytes*, not a URL, so the
signature stays valid pointed straight at it.

Canary has no release to point at — it ships from every push to `main` — so
its dmg lives in the R2 bucket instead, and **keyed per build rather than
overwritten at one fixed path**, which is the design a reader will otherwise
"simplify" away. Overwriting `canary/Far Cooler.dmg` means a stale appcast — an
edge cache, a client that polled an hour ago — still names the old version
while the object underneath it now holds the new build's bytes, so the
signature the appcast carries stops matching what Sparkle just downloaded.
Sparkle reports that as a signature verification failure, indistinguishable
from tampering — the one error nobody should be trained to click past. A key
that carries the build number turns the same situation into a plain 404
instead: try again later, which is what it actually is.

Nothing prunes those objects from a workflow: `wrangler r2 object` can put,
get and delete by key but cannot enumerate a bucket, so pruning from CI would
mean guessing at old keys. Retention is a bucket rule instead — an R2 object
lifecycle rule deletes everything under the `canary/Far Cooler-` prefix after
14 days, no workflow code involved. That prefix must not reach
`canary/appcast.xml`: the feed lives under `canary/` too, and deleting it
would take down every canary install's update check with a silent 404 until
the next push, rather than pruning an old dmg. At roughly twenty builds a day
the dmgs bound storage around 3 GB, comfortably inside R2's free tier, and a
stale appcast still inside that window resolves to a real, correctly signed,
slightly older build rather than failing outright.

**The keys are committed on one side only.** `apps/macos/sparkle-public-keys.txt`
carries one EdDSA public key per shipping channel — `local` gets none, for the
same reason it gets no feed — and `build-app.sh` stamps it into `SUPublicEDKey`
straight from that file. It is committed on purpose, unlike the WorkOS client
id: kept in a repository variable, a swapped key would be silent, with no diff
anywhere; committed, changing whose code can replace someone else's Mac is a
line in a commit somebody reviews. The private half of each pair never leaves
the secret it was generated into (see `CANARY_SPARKLE_KEY`,
`PREVIEW_SPARKLE_KEY`, `STABLE_SPARKLE_KEY` above), so a canary key
structurally cannot sign a stable update, even pointed at the wrong feed by
mistake.

**A known gap, recorded rather than hidden.** `version.sh channel` has no
`beta` case: its pattern match only carves `-preview.` out of everything else
starting with `v`, so a hand-pushed `v0.2.0-beta.1` tag falls through to the
plain `v*` case and resolves to `stable` — signed with `STABLE_SPARKLE_KEY`
and published to the stable appcast. `promote.yml` only ever creates
`-preview.` tags, so nothing reaches this today; it would only bite someone
pushing a version tag by hand.

**Setting one up**, beyond what Secrets and Variables above already cover
(the `*_SPARKLE_KEY` secrets, `SPARKLE_PUBLISH`, the `farcooler-updates`
bucket and its custom domain, the token's R2 permission): add a lifecycle rule
on that bucket expiring objects under the `canary/Far Cooler-` prefix — the
dmgs, not `canary/appcast.xml` — after 14 days. It is a bucket setting with no
`wrangler` equivalent — there is nothing to script, only a dashboard toggle to
remember.

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
| `wire` | that a client already in the field can still talk to this — the proto lint, its own self-tests, what `version.sh` answers, that the four channels stay four WorkOS projects, and that `appcast.py` emits a feed Sparkle can actually read |
| `rust` (Linux + macOS) | clippy at -D warnings and tests, against a real tmux — a fake one would agree with whatever this code believed |
| `swift` | the full `build-app.sh`, a check that the bundle's stamp matches the workspace, a run of the bundled `farcoolerd` to prove the tunnel is linked into it, and that each channel's icon renders (byte-identical for stable) |
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
| `APP_STORE_KEY_ID`, `APP_STORE_ISSUER_ID`, `APP_STORE_KEY_P8` | TestFlight, from both the canary and the release workflow | base64 `.p8`, and the key must be **Admin** — see below |
| `IOS_CERTIFICATE`, `IOS_CERTIFICATE_PASSWORD` | the iOS archive, in both workflows | base64 `.p12` of an Apple Development identity. Not optional, and the reason is below |
| `CLOUDFLARE_API_TOKEN`, `CLOUDFLARE_ACCOUNT_ID` | relay deploy, and the Sparkle appcast publish | Four permissions for the relay, plus **R2 Object Read & Write** for the appcast — the last two relay permissions are the ones nobody expects, see below |
| `CANARY_SPARKLE_KEY`, `PREVIEW_SPARKLE_KEY`, `STABLE_SPARKLE_KEY` | `sign_update`, in canary.yml and release.yml's `macos` job | base64 EdDSA private key, one per channel, from Sparkle's `generate_keys -x`. The matching public half is committed at `apps/macos/sparkle-public-keys.txt` — a fork can only sign its own updates, never impersonate another channel's |

### Credentials that need more than the obvious setup

**The App Store Connect key must be Admin, not App Manager.** App Manager is
enough to upload a build, which is what it looks like the key is for, and enough
for cloud signing to create a DEVELOPMENT certificate — so the archive succeeds
and the log even says `Apple Development: Created via API`. The export then asks
for distribution assets and gets `Cloud signing permission error` followed by
`No profiles for 'com.farcooler.ios.canary' were found`, because creating a
DISTRIBUTION certificate is Admin-only. A key's role cannot be changed after
creation; generate a new one and replace all three secrets.

**The iOS certificate has to be supplied, because otherwise CI makes its own.**
The archive signs with automatic signing and `-allowProvisioningUpdates`, which
is what lets a runner have the provisioning profiles it cannot have been born
with. Profiles are free. Certificates are not: Apple caps how many an account
may hold, and a runner with an empty keychain has no Apple Development identity,
so cloud signing creates one — signs a perfectly good build with it, uploads it,
and is then destroyed along with the private key. Nothing looks wrong for about
eleven runs. Then every archive fails with `Choose a certificate to revoke. Your
account has reached the maximum number of certificates.` and, underneath it, `No
profiles for 'com.farcooler.ios.canary' were found` — which is not a second
problem but the same one, since no certificate means no profile either.

`scripts/import-ios-certificate.sh` is the fix, and it runs before the archive in
both workflows: automatic signing reuses an identity the keychain already has and
only creates one when it finds none. To make the secret, open Keychain Access,
find your **Apple Development** certificate, expand the arrow so the private key
underneath it is selected too, and export both as a `.p12` — a certificate
exported alone imports without complaint and yields no identity at all, which the
script refuses rather than pass on. Then `base64 -i certificate.p12 | pbcopy`.
The certificates already stranded in the portal by earlier runs are safe to
revoke: their private keys went with the runners, so nothing can sign with them.

**The Cloudflare token needs zone permissions as well as account ones.** Account
→ Workers Scripts → Edit and Account → D1 → Edit get the script uploaded and the
migrations applied, which is far enough to look like it works. Every relay
declares `custom_domain = true`, and attaching that route is a ZONE call:
without Zone → Workers Routes → Edit and Zone → Zone → Read, scoped to
`farcooler.com`, the deploy fails on its last API call with
`Authentication error [code: 10000]`. Unlike the Apple key, a Cloudflare token
can be edited in place, so no secret has to change.

**Analytics Engine has to be switched on for the account** before any relay can
deploy, because every environment binds a dataset. It is not a wrangler setting
and the error names no fix beyond a dashboard link: `You need to enable
Analytics Engine ... [code: 10089]`. Creating one dataset in the dashboard
provisions it; the datasets themselves are still created implicitly on first
write, so there is nothing to keep in step with `wrangler.toml`.

**The R2 bucket for the appcast must be named `farcooler-updates`.** Neither
workflow reads that name from anywhere — it is a literal in the `wrangler r2
object put` call in both canary.yml and release.yml — so a bucket created
under any other name fails every upload with `The specified bucket does not
exist`, which says nothing about the name being wrong. Give it
`updates.farcooler.com` as a custom domain: that is the URL `SUFeedURL` and
`scripts/appcast.py`'s `--url` both expect, and it is a property of the
bucket, not of the token.

### Variables, which are the on switches

Repository **variables**, not secrets, for the reason two sections down. Neither
is set by default, so a fresh fork builds everything and ships nothing.

| Variable | Set to | Without it |
| --- | --- | --- |
| `RELAY_DEPLOY` | `true` | the relay job skips, and every channel's relay stays where it is |
| `CANARY_TESTFLIGHT` | `true` | the canary iOS job skips, and main reaches nobody's phone |
| `SPARKLE_PUBLISH` | `true` | the signing and appcast-publish steps skip in both canary.yml and release.yml — a dmg still builds and (for stable/preview) still reaches the GitHub release, but no channel's app is ever told a newer build exists |
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
