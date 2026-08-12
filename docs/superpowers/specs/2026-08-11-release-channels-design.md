# Design: dev, beta and release as separate installs

Date: 2026-08-11
Status: APPROVED (design); implementation not started
Paired with `docs/superpowers/specs/2026-08-11-api-versioning-design.md`, which
depends on the channel boundary this document establishes.

## Problem

A channel is a label today and nothing more. `scripts/version.sh:69` derives it
from the tag on `HEAD` — `release`, `beta`, or `dev` — and it reaches an
`Info.plist`, a settings row, and the string a device reports to the relay. That
is the whole of it.

Everything underneath is single-instance. One bundle identifier per platform
(`com.farcooler.ios` at `apps/ios/generate-project.py:315`,
`com.farcooler.android` at `apps/android/app/build.gradle.kts:80`). One binary
path, `~/.local/bin/farcoolerd`, hardcoded in the shipped app
(`crates/client/src/session.rs:130`, `:162`) and in six places in
`crates/cli/src/host_install.rs`. One launchd label,
`com.farcooler.daemon.remote` (`host_install.rs:39`). One systemd unit,
`farcooler.service`. One runtime directory, one SQLite database, one socket, one
tmux server. One `APNS_TOPIC` in the relay (`services/relay/src/push.ts:67`).

`docs/releasing.md` states the position plainly: *"A beta of `0.2.0` **is**
`0.2.0` — same code, same marketing version, same App Store entry."* That was
right when a beta was something a handful of people ran for an afternoon. It is
wrong now that beta is the primary way this product is used, and it produces two
concrete failures.

**You cannot try a beta without risking the fleet your real work lives in.**
Installing a beta daemon replaces the release daemon, on the same database, the
same tmux server, and the same managed worktrees. The thing you would most want
to test on is the thing you cannot afford to test on.

**Compatibility has to hold across every combination.** Any app build can reach
any daemon build, so the compatibility matrix is every app against every daemon,
and the freeze rules have to be the strictest that any pairing demands. That is
what makes the versioning design's baseline question so awkward: a field that
shipped only in a beta is either wrongly frozen forever or wrongly unprotected,
and there is no channel boundary to hang a different answer on.

## What a channel becomes

A channel is a separate installation, top to bottom. Three of them coexist on
one machine and on one phone, and none of them can see another's state.

| | dev | beta | release |
| --- | --- | --- | --- |
| Daemon binary | `farcoolerd-dev` | `farcoolerd-beta` | `farcoolerd` |
| CLI binary | `farcooler-dev` | `farcooler-beta` | `farcooler` |
| Runtime dir | `…/FarCooler Dev` | `…/FarCooler Beta` | `…/FarCooler` |
| Database, socket, install-id | separate, inside the runtime dir |||
| tmux server | separate, `tmux -L farcooler-<install-id>` |||
| Managed worktrees | separate, inside the runtime dir |||
| launchd label | `com.farcooler.daemon.remote.dev` | `.beta` | `com.farcooler.daemon.remote` |
| systemd unit | `farcooler-dev.service` | `farcooler-beta.service` | `farcooler.service` |
| iOS bundle | `com.farcooler.ios.dev` | `com.farcooler.ios.beta` | `com.farcooler.ios` |
| Android app id | `com.farcooler.android.dev` | `.beta` | `com.farcooler.android` |
| Mac bundle | `com.farcooler.mac.dev` | `.beta` | `com.farcooler.mac` |
| APNS topic | the bundle id, per channel |||

The release column is unchanged from today. Everything already installed keeps
its paths, its database and its tmux server, and nothing has to be migrated.
Only the two new columns are new names.

### Where the channel comes from

Compile time, from `scripts/version.sh channel`, stamped by
`crates/protocol/build.rs` beside `FARCOOLER_BUILD` and exported as
`farcooler_protocol::CHANNEL`.

Never a flag and never an environment variable, for the reason `version.sh:54`
already gives about the channel itself: *"a flag is a thing to forget on the
build that mattered."* A daemon that could be told which channel it is could be
told wrong, and the failure — a beta daemon writing into the release database —
is the one this whole design exists to prevent.

`FARCOOLER_HOME` keeps overriding the runtime directory outright. It is how
tests and scratch daemons get an isolated home, and channel derivation happens
only when it is unset.

### What falls out for free

Deriving the runtime directory from the channel gets most of the isolation with
no further work, because so much already hangs off it:

- `socket_path()` and `database_path()` are inside it (`paths.rs:43`, `:47`).
- `install_id_path()` is inside it (`paths.rs:51`), and the tmux server is
  `tmux -L farcooler-<install-id>` (`crates/tmux/src/server.rs:8`) — so a
  separate runtime dir means a separate tmux server, and therefore separate
  panes, without naming tmux anywhere in this change.
- `worktrees_dir()` and the pastes directories are inside it (`paths.rs:57`,
  `:71`).

One function, and the daemon's entire runtime footprint separates.

### What does not, and has to be built

**1. Binary names, and the app asking for the right one.** Two daemons cannot
both live at `~/.local/bin/farcoolerd`. The app derives the name from its own
compile-time channel — a beta app asks for `farcoolerd-beta` — which is what
makes "a beta app never talks to a release daemon" true by construction rather
than by policy.

This revises rule 4 of the versioning contract. What is frozen is no longer the
literal path but the *scheme*: `~/.local/bin/farcoolerd[-<channel>]`, `--stdio`,
`--stream <terminal-uuid>`. The suffix is empty for release, so every installed
release client keeps working unchanged.

**2. Service unit names.** `LAUNCH_AGENT` (`host_install.rs:35`) and `UNIT`
(`host_install.rs:57`) are string constants with the label and `ExecStart` baked
in. Both become channel-aware, or installing a beta silently replaces the
release daemon's supervision — which is exactly the failure this design is meant
to remove, arriving through the back door.

**3. Bundle identifiers and push topics.** Separate bundle ids mean separate
APNS topics, and `push.ts:67` sends a single `env.APNS_TOPIC`. The relay routes
by the registering device's channel instead. The `devices` and `daemons` tables
already carry a `version` column from
`services/relay/migrations/0002_versions_and_expiry.sql`; this adds `channel`
beside it, nullable, additive, in keeping with that file's stated rule.

A separate iOS bundle id also means a separate App Store Connect record for the
beta, distributed through TestFlight on that record. A beta tester does not
auto-upgrade into the release app; they hold both, which is the point.

**One relay serves every channel.** Not a separate deployment per channel, and
that is a decision rather than an omission: `relay.yml` deploys continuously from
`main` precisely because it must keep serving app builds that are months old, so
it is already required to be compatible with everything in the field. A second
deployment would double that obligation to buy isolation the `channel` column
already provides. A beta app therefore writes to the production D1, and the
additive-only rule in `migrations/0002_versions_and_expiry.sql` is what keeps
that safe.

**4. Worktree ownership.** The one that would bite, described next.

## Worktree ownership

`crates/daemon/src/reconcile.rs:122` adopts every worktree that `git worktree
list` reports for a registered repository. There is no ownership marker and no
path filter. Adoption is deliberate and worth keeping — `proto/farcooler.proto:66`
records that client-side import was removed precisely because the reconciler
does this now — but it assumes there is only ever one Far Cooler.

With two, each channel adopts the other's worktrees. Both show them in their
fleet. Both can start agents in them, on separate tmux servers, in the same
directory, at the same time. Nothing detects it and nothing reports it.

**The fix.** A daemon writes its `install-id` into the git config of each
worktree it creates. Adoption then follows one rule:

- Unmarked worktree: adopt it, exactly as today. This is a worktree a person
  made by hand, and picking it up is the feature.
- Marked with this install's id: adopt it, as today.
- Marked with a different install's id: skip it, and say so at debug level.

This is the rule `@farcooler_daemon_id` already applies to tmux panes
(`crates/core/src/lib.rs:28`), extended to the thing that outlives them. A pane
is transient and already tagged; a worktree is durable and was not.

It also fixes a case that predates channels: two daemons on one machine under
different `FARCOOLER_HOME` values — which is what every scratch daemon is —
already had this hazard against the real one.

## Testing the isolation

Every claim in this document rests on one property: two channels on one machine
cannot see each other's state. Nothing currently asserts it, and the whole test
suite already quietly depends on it — `crates/daemon/tests/rpc_over_socket.rs:18`
gives each test *"a daemon on a private socket with a private database"* and
notes the service is opened at an explicit directory, never through
`FARCOOLER_HOME`, *"because the environment is process-global and these tests run
in parallel."*

That is the whole trick. A channel's only job is to choose a runtime directory,
so two services at two explicit roots **is** two channels, and the isolation test
needs no new harness — only the assertions nobody has written.

**The mapping is total and distinct** (unit). `runtime_dir_for(Dev|Beta|Release)`
returns three different paths; release's is byte-identical to today's, so nothing
already installed migrates; `FARCOOLER_HOME` still overrides all three.

**Two daemons do not see each other** (integration). Two services at two explicit
roots, both registering the *same* git repository — which is the case that would
actually happen, and the one that fails today:

- Create a workspace in A, reconcile B, and assert B did **not** adopt it. This
  is the worktree-ownership rule, and it is the assertion that fails against the
  current code.
- A's workspace list contains it; B's does not.
- The two install-ids differ, so the two tmux server names
  (`tmux -L farcooler-<install-id>`) differ.
- Create a terminal in A; B's terminal list is empty and B's tmux server reports
  no pane for it.

**Binary names are distinct, and release's is unchanged** (unit).
`daemon_binary_name(channel)` gives three names, and release's is bare
`farcoolerd` — so every already-installed release client keeps resolving the
same path.

One trap worth stating because this codebase has already hit it: a runtime root
has to stay short enough that a per-terminal agent socket under it fits in
`sun_path`. `crates/daemon/src/agent_supervisor.rs:529` already tests exactly
that, and a test root nested a few directories deeper than the real one is how
you would find out the hard way.

## Promotion

The pipeline is dev → beta → release. Dev is automatic; the other two are
buttons. There are no `beta` or `release` branches: everything happens on `main`
and channels are named by tags.

**dev** is every push to `main`, and every local build. `version.sh` already
returns `dev` for an untagged or dirty tree, so this needs no new machinery, and
no tag is created — dev has no compatibility promise to record. A local `cargo
build` produces a dev-channel binary that installs beside your beta and your
release and cannot touch either.

**beta** is a `workflow_dispatch` button. It reads `version.sh marketing`, takes
the **highest** existing `v<version>-beta.N`, and creates `v<version>-beta.<N+1>`.
A dependent job checks that tag out and builds at channel beta.

Highest rather than a count: counting regenerates a number that already exists
the moment any beta tag is deleted or made by hand. That one fails loudly —
`fatal: tag 'v0.2.0-beta.3' already exists` — so it is a papercut rather than a
hazard, but there is no reason to write the operation that has the failure mode.

**release** is a second button, taking the beta tag to promote and defaulting to
the newest. It tags **the same commit** `v<version>`, and a dependent job builds
it at channel release.

Promotion therefore moves a commit, never a build. `release.yml`'s header states
the rule this preserves: the CLI, daemon, Mac app and phone app in a release are
built from one commit, because they check each other's build stamps and a
release assembled from four builds can disagree with itself.

**A tag pushed by CI does not trigger another workflow.** GitHub suppresses
events raised with the default `GITHUB_TOKEN` to prevent recursion, and
`release.yml` currently triggers on `push: tags: ['v*']`. So a button that
tagged and stopped would silently never build anything. Each button is therefore
one workflow whose build job `needs:` its tagging job — which also means no
second trigger has to fire, and no personal access token has to exist.

The version bump stays out of the buttons. `[workspace.package] version` moves
on `main` as an ordinary pull request, before promotion — a job that bumped and
then built would be shipping a commit nobody ran on beta. This dissolves most of
`scripts/release.sh`: what remains is the `Cargo.toml` edit, and tagging moves
into CI.

Nothing else needs incrementing. `Cargo.toml` holds the only literal in the
repo; `apps/macos/build-app.sh:58` stamps the bundled plist at build time,
`apps/ios/Info.plist:31` holds `$(MARKETING_VERSION)` which
`generate-project.py:309` fills in, and
`apps/android/app/build.gradle.kts:93` takes `versionCode` and `versionName`
from the same script. A release job never edits a plist, a gradle file, or a
project file.

Marketing version is unchanged in meaning: a beta of `0.2.0` is still `0.2.0`,
and the channel distinguishes them. What changes is that they are now two
installs rather than two builds of one.

### Confirming a version before it exists

A tag is not undoable in the way a commit is. Once `v1.0.0` has been pushed and
a build from it has reached TestFlight, that version number is spent: App Store
Connect will not take it again. So both buttons confirm twice, in two different
ways, because they catch two different mistakes.

**A typed confirmation.** The button takes the version you believe you are
shipping and refuses unless it matches `version.sh marketing` at the target
commit. This repository already reaches for that shape where being wrong is
expensive — `TypedConfirmation` at `proto/farcooler.proto:848`, with
`ERROR_CODE_CONFIRMATION_REQUIRED` behind it.

**A GitHub environment with required reviewers.** The tagging job targets an
`environment:`, so the run pauses with its inputs visible until someone
approves, before anything is created.

The typed input catches being wrong; the environment gate catches being fast.

It matters more on the beta button than the release one. Beta is where a version
number first becomes real: `v1.0.0-beta.1` is the moment "1.0.0" comes into
existence, and by release time you are promoting something already named.

**The buttons confirm the version; they cannot change it.**
`crates/protocol/build.rs:39` stamps `FARCOOLER_BUILD` from `CARGO_PKG_VERSION`,
so if a tag said `v1.0.0` while `Cargo.toml` said `0.2.0`, `farcooler --version`
would report `0.2.0+a1b2c3` while the app's plist said `1.0.0` — two components
of one release reporting different versions to each other, which is the failure
`docs/releasing.md` says the one-number rule exists to prevent and records this
project having hit twice.

So a version change is a `Cargo.toml` bump on `main`, and the buttons only ever
read it.

### Renaming a version in flight

Deciding mid-beta that `0.3.2` is really `0.4.0` costs a `Cargo.toml` pull
request and the next beta press. The button reads `0.4.0`, finds no
`v0.4.0-beta.*`, and creates `v0.4.0-beta.1`. The abandoned `v0.3.2-beta.*` tags
stay where they are: they name builds that really did reach TestFlight, and they
remain the answer to which one a given tester has. The counter restarting at 1
is correct — that build is the first beta of `0.4.0`.

It is cheap because mid-beta you were going to cut another beta regardless, so
the rename rides on a press you would have made. Renaming at *release* time is
the expensive one: the commit has already been beta'd under the old name, and
naming it something else means cutting a beta you did not otherwise need. That
is the right cost to pay rather than avoid — a tag is the wrong place to relabel
code nobody has run under the new name.

**A rename must go forward.** `0.3.2` to `0.4.0` is fine; `0.4.0` back to
`0.3.2` is rejected at upload and unfixable afterwards, the same class of
mistake `version.sh:40` already warns about for build numbers — *"build numbers
can never be reused."* So the beta button refuses when `Cargo.toml`'s version
sorts below the highest already-tagged version.

Build numbers are unaffected by a rename: `git rev-list --count HEAD` only
increases, and beta is a single bundle identifier, so the sequence stays
monotonic across one.

### Promotion double-tags a commit, and that breaks `version.sh`

Because beta and release have different bundle identifiers, they are genuinely
different artifacts. Promotion is therefore a rebuild of the same source at a
different channel, not a re-upload of the beta binary — so the promoted commit
ends up carrying two tags.

`version.sh:69` derives the channel with `git tag --points-at HEAD | grep '^v' |
head -1`, and git returns tags lexicographically:

    v0.2.0
    v0.2.0-beta.3

`head -1` picks `v0.2.0`. That is right for the release build and wrong for the
beta from that moment on: anyone rebuilding the beta from that commit gets a
release-channel binary, which installs at the release binary path and writes into
the release runtime directory. That is exactly the cross-channel contamination
this design exists to prevent, arriving through the release process itself.

**The channel comes from the tag being built, not from the tags pointing at
HEAD.** `version.sh channel v0.2.0-beta.3` takes the tag as an argument; with no
argument it falls back to today's behaviour, so local builds are unaffected and
still default to dev.

This is an argument where `version.sh:54` argues against flags — *"a flag is a
thing to forget on the build that mattered"* — and the distinction is that no
person supplies it. The workflow created the tag one job earlier and threads it
through a `needs:`. It is a value computed and passed along, not one remembered.

## Versioning inside a channel

This is what the channel boundary buys, and it is why the two designs are
paired. Because a beta app can only ever reach a beta daemon, the compatibility
matrix loses its off-diagonal and each channel can set its own freeze rule.

**Release: permanent.** A field that ships in a release is frozen forever. The
baseline is `proto/baseline/release.proto`.

**Beta: one beta.** A field added in beta N must keep working through beta N+1,
and may change or disappear after that. The baseline is
`proto/baseline/beta.proto`. This is the aggressive end deliberately: beta is
where the protocol's shape is still being discovered, and carrying every
exploratory field to 1.0 is a worse outcome than a tester having to update.

**Dev: nothing is frozen.** No baseline, no lint. A dev build talks to a dev
daemon you built from the same tree.

Both baseline files are committed to `main` by the promotion button that created
the tag, as a follow-up commit. Nothing derives them from git history: with no
channel branches there is no moving ref to diff against, and finding the newest
matching tag would mean `--sort=-v:refname` plus `versionsort.suffix` to order a
prerelease against its release — subtle shell in CI that nobody reads twice.

Committing the file was the alternative considered and rejected earlier in this
design's history, on the grounds that a human running a release script would
forget to refresh it. A button in CI does not forget, which is what makes it the
right answer now.

### The floor under the beta rule

A one-beta window means a tester who skips an update can land on a daemon two
ahead. Capability negotiation handles most of that gracefully — a capability the
daemon no longer advertises is a control the app dims — but it does not cover
the dangerous case. A field *removed* from a payload the old app still sends is
dropped as an unknown proto3 field, and the daemon does the old thing silently.
That is G4 from the versioning design, arriving through the channel that
tolerates removals.

So the beta channel gets a floor. `ClientHello` and `ServerHello` each carry
`build_number` — `scripts/version.sh build`, the commit count, which is already
the one monotonic integer this repo has and is already what the app stores use.
A beta daemon refuses a beta client more than one beta behind, before dispatching
anything, with `ERROR_CODE_VERSION_INCOMPATIBLE` and a sentence:

> This Far Cooler beta is too old for this machine. Update it from TestFlight.

Loud and early beats quiet and wrong. The release channel does not apply this
check: there, capabilities and a permanent freeze carry it, and refusing a
six-month-old app that would have worked fine is the failure the whole versioning
design exists to avoid.

The two hellos gaining a field is additive, so adding it breaks nothing.

## Sequence

1. `CHANNEL` stamped in `crates/protocol/build.rs`; `runtime_dir()` derives from
   it. Dev and beta daemons now isolate on a machine, with nothing else changed.
2. Worktree ownership marking and the adoption rule, with the two-daemon
   isolation test written **first** — it fails against the current code, which
   is what makes it worth having. Independently correct: this fixes a hazard
   that exists today between scratch daemons and the real one.
3. Channel-aware binary names, `host_install.rs` paths, and the two service unit
   templates. The client derives the daemon name from its own channel.
4. Bundle identifiers for the three apps; relay `channel` column and per-channel
   APNS topic.
5. `build_number` in both hellos, and the beta floor check in the daemon.
6. `version.sh channel [tag]` takes the tag being built. Independently a bug
   fix: the double-tag ambiguity is wrong today for anyone who tags a commit
   twice, and promotion makes it certain.
7. The two promotion buttons, and the baseline commit each one makes.
   `release.sh` reduces to the `Cargo.toml` bump.
8. `docs/releasing.md` rewritten around channels and buttons rather than around
   one number and one script.

Steps 1 and 2 are worth doing regardless of whether the rest lands: the first
makes scratch daemons safe, the second fixes a bug that is reachable today.

## What this does not solve

**One machine still runs one daemon per channel, not per person.** Nothing here
addresses two people sharing a host.

**A dev build has no compatibility promise at all**, by construction. Two dev
builds from different commits will drift, and the `BUILD` stamp comparison at
`apps/macos/Sources/FarCooler/Hosts.swift:204` is the only thing that will say
so. That is the right trade for a channel whose whole purpose is that nothing
is settled.

**Channel separation does not reduce the need for capability negotiation.** It
removes the cross-channel matrix, which was never the hard part. The drift that
will actually happen is a phone on beta N talking to a daemon on beta N+1, and
that is squarely inside one channel. The versioning design still carries it.

**Android has no release path, in any channel.** `.github/workflows/release.yml`
ships Linux, macOS and iOS to TestFlight; there is no Android job, and there was
none before this design. The per-channel `applicationId` and build number in the
table above describe what a build *would* be stamped with, not a pipeline that
exists. Deliberately deferred.

**Downgrade is unhandled, and this design makes it harder.** The versioning
design has the daemon refuse to open a database newer than its schema, which is
correct on its own but leaves an older daemon with no way back once a migration
has run. Combined with no hotfix line, the answer to a bad release is to ship the
next one. Accepted for now.

**There is no hotfix line.** Tags on `main` and no channel branches means that
once `main` has moved to `0.4.0`, there is no way to ship a fix to someone on
`0.3.2` without either releasing everything that has landed since, or cutting a
branch from the `v0.3.2` tag after all — the thing this model deliberately does
not have.

That is the right trade today: a maintenance branch costs more to carry than a
solo project mid-beta gets back from it, and the answer to a bad release is
currently to ship the next one. It is recorded here so that the first time a
hotfix is genuinely needed, adding a branch is a decision being revisited rather
than a hole being discovered.
