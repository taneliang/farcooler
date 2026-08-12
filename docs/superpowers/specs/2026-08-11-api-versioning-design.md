# Design: version drift between apps and daemons

Date: 2026-08-11
Status: APPROVED (design); implementation not started
Paired with `docs/superpowers/specs/2026-08-11-release-channels-design.md`.
Channels supply the boundary this design hangs its per-channel freeze rules on;
read that one first if you are reading both.

## Problem

An App Store review takes days. A TestFlight build takes hours. A Linux daemon
updated by `farcooler host install` takes one command, and a Mac app updates
itself. These four clocks do not tick together, and nothing in this repo lets
them run apart.

`docs/releasing.md` states the rule the whole system is built on — one version
for CLI, daemon, Mac app and iOS app, stamped from one commit — and gives the
reason: *"A Mac app talking to a daemon built from different source does not
fail to launch; it behaves like a bug you already fixed is still happening."*
That is right, and it is why `FARCOOLER_BUILD` exists. But it answers a
different question than the one drift asks. `BUILD` answers *are these the same
build*. Nothing answers *can these two work together when they are not*, and in
the field they will not be: a phone that has not opened the App Store in six
weeks is talking to a daemon someone updated this morning.

The machinery for the answer is half-built. `ClientHello.supported_protocol_versions`
and `ServerHello.selected_protocol_version` exist in the proto. `DaemonVersion`
carries a `capabilities` list. `docs/farcooler-design.md:1065` promises *"Daemon
protocol major version N supports clients on N and N-1. Newer clients hide
unsupported capabilities after negotiation."*

None of it works.

## What the audit found

Eight surfaces can drift. Five of them matter.

| # | Surface | Consumers | Risk |
| --- | --- | --- | --- |
| 1 | Daemon wire protocol (protobuf over Unix socket and SSH stdio) | CLI, Mac app via its bundled CLI, iOS/Android via the bundled Rust client core | **High** |
| 2 | Agent event JSON (`AgentEventFrame.payload_json`) | Swift `AgentKit`, Kotlin `model/AgentEvent.kt`; also persisted in SQLite | **High** |
| 3 | Relay HTTP `/v1/*` | iOS, Android, daemon | Low — already governed |
| 4 | Push payload (daemon to relay to app) | iOS, Android | Low |
| 5 | SQLite store schema | daemon only | Medium, on downgrade |
| 6 | tmux pane tags (`@farcooler_*`) | daemon to daemon across versions | Medium, unenforced |
| 7 | `dist/<arch>-linux/` layout for `host install` | Mac app to remote Linux | Low |
| 8 | **Remote invocation contract** — `~/.local/bin/farcoolerd --stdio` and `--stream <id>` | iOS/Android/Mac over SSH | **Medium** |
| 9 | ACP / Claude / Codex adapter protocols | third-party CLIs | Separate problem |

The Rust-core-to-Swift/Kotlin FFI and the Mac-app-to-bundled-CLI boundary ship
inside one bundle and cannot drift. They are not versioning surfaces; they are
where a versioning answer has to arrive.

### What already works

The discipline is real, and this design keeps all of it.

Retired proto tags are documented as retired and never reused — `// 30 was
WorktreeImport` in `proto/farcooler.proto:66`. An unknown method is rejected
rather than defaulted (`rpc.rs:172`, `rpc.rs:226`). prost's enum accessors fall
back to `Unspecified`, and the hand-written label functions have explicit
fallback arms with the reasoning written down: `pane_mode_label` at
`crates/client/src/session.rs:1258` says *"Unspecified from an older daemon is
terminal: the mode that needs no adapter and always works."* The agent event
JSON already uses `#[serde(default)]` and `skip_serializing_if` for new fields —
`AgentEvent::SessionStarted.backend` at `crates/agent-core/src/event.rs:217`
carries the note *"every transcript already in SQLite was written before this
field existed and must still decode"* — and both app decoders fall back on an
unknown variant name. The relay has a written additive-only policy with a `/v2/`
escape hatch. The store does forward-only migrations with checksummed backups.

What is missing is not care. It is a mechanism.

### G1 — `PROTOCOL_VERSION` is a wall, not a window

`crates/transport/src/client.rs:99` rejects unless `selected_protocol_version`
equals the client's own. `crates/transport/src/connection.rs:192` rejects unless
the client's offered list contains the daemon's own. The client offers exactly
`[PROTOCOL_VERSION]` (`client.rs:88`). The N/N-1 window `farcooler-design.md:1065`
promises is not implemented anywhere.

Bumping to 2 hard-fails every shipped app at the handshake. The practical
consequence is worse than the break: nobody will ever dare bump it, so real
changes get made under version 1 anyway and the number stays decorative while
the compatibility it claims to describe erodes silently.

### G2 — no capability negotiation

`DaemonVersion.capabilities` (proto field 3) is populated at
`crates/daemon/src/rpc.rs:428` with two hardcoded placeholders, `"workspaces"`
and `"terminals"`. Nothing reads them; the only assertion is
`crates/daemon/tests/rpc_over_socket.rs:291`, which checks the list is not
empty. `ServerHello` carries no capabilities at all, so learning them costs a
second round trip that no client makes.

A client therefore cannot ask whether a machine does stacks. It can only try and
read the error.

### G3 — `NOT_FOUND` is overloaded

An unknown method and a missing workspace return the same code. A newer app
calling `pr.refresh` against an older daemon cannot distinguish *this machine is
too old for that* from *no such PR*, so it can neither degrade nor say the right
thing.

### G4 — silent semantic drift on request payloads

The worst one, and the only one no existing mechanism touches.

Add a field to an existing payload — a base branch on `WorkspaceCreate`, say —
and an older daemon drops it as an unknown proto3 field and does the old thing.
The client believes it asked. The user sees a workspace created from the wrong
base and has no reason to suspect a version. There is no error, no log, and
nothing in CI that would notice.

### G5 — agent events tolerate unknown variants, not unknown enum values

`AgentEvent.swift:378` falls back to `.gap(.unparsed)` on an unknown variant
name, which is correct. But inside a known variant, `Role` (`AgentEvent.swift:3`)
and `ToolStatus` (`AgentEvent.swift:9`) are bare `String, Decodable` raw-value
enums with no fallback. A future `ToolStatus::Canceled` from a newer daemon makes
`try outer.decode(ToolCallPayload.self, ...)` throw, which propagates out of
`AgentEvent.decode`, and `apps/ios/FarCooler/AgentStream.swift:118` does
`try?` inside a `compactMap` — so the tool call vanishes from the transcript
with no gap marker and no trace.

Because these events are persisted in SQLite, this is also an at-rest format:
an older app reading a transcript a newer daemon wrote hits the same path.
Kotlin is more tolerant (`else ->` fallbacks at `AgentEvent.kt:336` and
`Markdown.kt:287`) but has not been audited variant by variant.

### G6 — the remote invocation contract is undeclared

Streaming a terminal is not a wire method. `crates/client/src/session.rs:162`
spawns `~/.local/bin/farcoolerd --stream <terminal>` as a second SSH exec,
deliberately, so bytes do not sit behind a fleet refresh on the one serialized
control connection. The control channel itself is `~/.local/bin/farcoolerd
--stdio` (`session.rs:130`).

Both are hardcoded in an app bundle that ships to the App Store. The absolute
path, the two flag spellings, and the argument shape of `--stream` are a public
API of the daemon binary held together by nothing but nobody having touched
them. `crates/daemon/src/main.rs:58` and `:77` parse them by raw string compare
against `std::env::args()`.

Adding a flag is safe — an older app does not pass it. Renaming `--stream`,
moving the install path, or changing what `--stream` expects breaks every
installed app at once, and no test would notice.

### G7 — nothing in CI runs an old client against a new daemon

Every rule above is a convention held by a comment.

### G8 — smaller things the audit turned up

- `apps/macos/Sources/FarCooler/DaemonClient.swift:163` detects a version
  mismatch by substring-matching `"update the older side"` against a message
  produced in `crates/cli/src/remote.rs:142`. A compatibility mechanism that
  breaks when someone rewords an error string is itself a drift hazard.
- iOS and Android never see that condition at all: `crates/client/src/session.rs:38`
  words it differently and nothing downstream matches on it.
- `farcooler_core::tmux::SCHEMA_VERSION` (`crates/core/src/lib.rs:31`) names the
  tag `@farcooler_schema_version`. It is never written and never compared. The
  only `schema_version` values in the tree are a parse at
  `crates/tmux/src/windows.rs:383` and a test fixture at
  `crates/daemon/src/watch.rs:772`.
- `crates/store/src/store.rs:78` returns `Ok(())` when the database's
  `schema_version` is at or above `CURRENT_SCHEMA_VERSION`, so an older daemon
  opening a newer database proceeds to run against a schema it does not know.

## What we are building

Four decisions, taken in that order because each depends on the one before.

### 1. The contract

Three rules, written down beside the relay's, which already lives by the first
of them.

1. **The wire is additive-only.** Tags are never reused, fields are reserved
   rather than removed, meanings never change, and no existing message gains a
   required field.
2. **Anything a client must know exists, it asks for by name.** Never by
   comparing version strings, never by inferring from a daemon version.
3. **`PROTOCOL_VERSION` is reserved for a framing break** that additivity cannot
   express — a change to the envelope, the length prefix, or the handshake
   itself. Bumping it is a MAJOR release and requires implementing the N/N-1
   window in the same change. Expected frequency: never.

4. **The remote invocation contract is frozen under the same rule as the wire.**
   `~/.local/bin/farcoolerd[-<channel>]`, `--stdio`, and `--stream
   <terminal-uuid>` are as public as any proto message, because an App Store
   binary hardcodes all of them (`crates/client/src/session.rs:130`, `:162`).
   New flags may be added; the scheme may never be renamed, moved, or given a
   different argument shape.

   It is the *scheme* that is frozen, not the literal, because channels make the
   suffix vary — see the channels design. The suffix is empty for release, so
   every installed release client keeps working unchanged.

The point of rule 3 is that the version number stops being the mechanism and
becomes the fire alarm. A number that is never pulled is a number that can be
trusted when it is.

Rule 4 is stated because nothing about `main.rs:58` looks like an API. It parses
two strings out of `std::env::args()`, and it is the only reason a phone can
reach a machine at all.

### 2. Capabilities

**Where they live.** `ServerHello` gains `repeated string capabilities = 6`
(tags 1–5 are taken). In the handshake, so every client has the answer before
its first request, at zero additional round trips.

`DaemonVersion.capabilities` stays where it is for `farcooler daemon version`
and diagnostics, and is populated from the same source as `ServerHello`, so the
two cannot disagree.

**What the names are.** One per user-visible feature that a client can be built
to use before every daemon has it — not one per method. From the surface as it
stands today:

| Capability | Covers |
| --- | --- |
| `workspaces` | `repository.*`, `workspace.*`, `repository_root.*`, `branch.list`, `worktree.list` |
| `terminals` | `terminal.create/screen/write/resize/stop/seen/remove/restart/dismiss_lost`, and the `--stream` data plane |
| `agent` | `terminal.set_pane_mode`, `terminal.agent_*`, `worktree.file_search` |
| `changes` | `changes.change_set`, `changes.commit_files`, `changes.file_diff`, `changes.set_base`, `changes.mark_read`, `changes.inbox` |
| `stack` | `stack.get`, `stack.set_parent`, `pr.refresh` |
| `layout` | `layout.*` |
| `paste` | `terminal.paste_file` |
| `adapters` | `adapter.*` |
| `themes` | `theme.*`, `settings.set_branch_prefix` |

`workspaces` and `terminals` are the floor: a daemon without them is not a
daemon. The two placeholders at `rpc.rs:428` become these, and stop being
decorative.

**Where the list is defined.** One table in `crates/protocol`, beside
`PROTOCOL_VERSION`. Each entry is a name, the methods it covers, and the payload
fields it covers. The daemon builds `ServerHello.capabilities` and
`DaemonVersion.capabilities` from it, the proto lint reads it to decide whether a
new method or field is accounted for, and the CLI reports it.

One definition with three consumers, for the same reason `scripts/version.sh` is
the only implementation of "what version is this": a second copy is the drift the
thing exists to prevent.

### 3. Making G4 loud

`Request` gains `repeated string required_capabilities = 7`, sitting with the
other generic envelope preconditions — `target_resource_id`,
`expected_resource_version`, `expected_lease_generation`, `idempotency_key`. It
is validated in `Rpc::handle` (`crates/daemon/src/rpc.rs:223`) in the same place
scope already is, before any domain logic runs.

This follows the rule the envelope was designed around, stated at
`proto/farcooler.proto:10`: concurrency, lease and idempotency metadata are
canonical only in the envelope, and *"the generic dispatcher validates these
preconditions before invoking domain logic."* Capability is another such
precondition, and belongs in exactly the same place.

It covers two cases:

- **A method that belongs to a capability.** The client names it; an older
  daemon refuses with a code that says why.
- **A new field on an existing payload.** The client that fills in the new field
  also names the capability that field belongs to. This is the reason the check
  is on the envelope rather than voluntary at each call site: there are roughly
  thirty call sites, and a check that can be forgotten at any one of them leaves
  G4 exactly as silent as it is now.

**New error code.** `ERROR_CODE_CAPABILITY_UNSUPPORTED = 28`, carrying the
missing capability name in its message. Per the house rule that every code has a
sentence a person reads and never a raw Rust error:

> This machine is running an older Far Cooler that can't do this yet. Update it.

This also fixes G3: `required_scope()` returning `None` for an unknown method
maps here instead of to `DomainError::NotFound`, so *this machine is too old*
stops presenting as *no such workspace*.

`retryable` is false. No amount of retrying updates the other side.

### 4. Availability reaching the UI

A control the machine cannot serve is shown, dimmed, with the reason — not
hidden. `farcooler-design.md:1065` currently promises hiding; that is the wrong
call and gets rewritten. The same app showing different tabs for two machines,
with no stated reason, reads as a bug. It is also inconsistent with how this app
already treats an uninstalled host: it says so rather than omitting the row.

The Rust client core normalizes everything for both apps, so this is one place.
`crates/client/src/session.rs` holds the negotiated set from the handshake;
`crates/client/src/ffi.rs` puts it in the connection JSON both apps already
decode. iOS and Android each gate their controls on it.

The Mac app drives remote hosts through its bundled CLI subprocess rather than
the FFI, so the CLI's connection JSON carries the same field.

Copy, per the house conventions: *"machine"*, not *"host"*. Title case on any
button.

> Needs a newer Far Cooler on this machine.

### 5. The agent event channel

This gets its own answer because it is also an at-rest format: transcripts
written by a newer daemon are read by an older app out of SQLite, so tolerance
has to work in both directions and cannot be negotiated at connect time.

- Swift `Role` and `ToolStatus` gain an `unknown` case with a custom
  `init(from:)`, exactly as `GapReason` already does at `AgentEvent.swift:50`.
  Every other raw-value enum in that file gets the same audit, and so does the
  Kotlin side.
- `AgentStream.swift:118` stops silently dropping. A decode that fails becomes
  `.gap(.unparsed)` — the marker the format already has for precisely this
  condition — so a person sees *something happened here I can't show* rather
  than a hole with no explanation.
- The Rust-side rule gets written down where the enum lives: new variants are
  fine, new fields need `#[serde(default)]`, and existing variant and field
  names never change. This is already the practice; it is stated nowhere.

### 6. Enforcement

#### The baseline is recorded, not discovered

The obvious design — diff the proto against "the last release tag" — does not
survive contact with this repository.

**There are no tags.** `git tag --list` is empty; nothing has ever been
released. A lint resolving its baseline through `git describe` would pass
vacuously on day one, which is how a check dies before it runs once.

**The rust CI job is a depth-1 checkout.** `.github/workflows/ci.yml:46` sets no
`fetch-depth`, so tags are not even fetched there. `scripts/version.sh:40`
already carries the scar tissue for this exact trap — *"A shallow clone is the
trap here, and it fails SILENTLY"* — and a lint that read git history in CI
would reproduce the bug that comment exists to prevent.

**Version sort is a third trap.** `v0.10.0` against `v0.9.0` needs
`--sort=-v:refname`, plus `versionsort.suffix` configuration to order a
`-beta.3` ahead of its release. Three subtleties in a shell line nobody reads
again.

So the baseline is a checked-in file, committed by the promotion button that
created the tag:

- `proto/baseline/release.proto` — the proto as of the last release. Permanent
  freeze.
- `proto/baseline/beta.proto` — the proto as of the last beta. One-beta freeze.
- Dev has no baseline and no lint.

There are no `beta` or `release` branches to diff against — channels are named
by tags on `main`, so there is no moving ref, and finding the newest matching
tag would drag `--sort=-v:refname` and `versionsort.suffix` back in. A file
committed by CI needs none of that. The objection to a checked-in baseline is
that whoever cuts a release forgets to refresh it; a button does not forget.

The comparison is then a plain diff between two files in one checkout: no tags,
no network, no shallow-clone trap, no version sort. And the baseline moving is
visible in the pull request that moves it, which is where a person would want to
see it. Same instinct as `version.sh` being the only implementation of "what
version is this": record once, do not re-derive.

The baseline files live in `proto/baseline/` rather than beside
`proto/farcooler.proto`, so they stay off the include path
`crates/protocol/build.rs:47` passes to protoc.

**Proto lint** — a new CI job comparing `proto/farcooler.proto` against the
baseline for the channel being built. Fails on a removed field, a reused tag
number, a changed field type, a renamed field, or a new method or payload field
that no capability table entry covers.

Before the first release the baseline file is absent, and the lint exits zero
saying so — nothing has shipped, so nothing is owed compatibility, and the
`TerminalAttach` tag reservation below is still legal. To keep it from being
dormant and untested until then, the lint's own logic gets unit tests against
fixture protos: a removed field, a reused tag, a method no capability covers.

**Old-client compatibility test** — a new CI job that builds `crates/client`
from the oldest supported tag, runs it against a HEAD daemon over a real socket,
and asserts that a handshake, a fleet read, a `terminal.screen`, an `--stream`
open and a `terminal.agent_subscribe` all succeed. The `--stream` case is what
holds rule 4: it is the only check that would fail if someone renamed the flag.

This is the only part of the design that proves compatibility rather than
asserting it. It is also the one thing here that genuinely needs a git ref,
because it has to build old source — so it carries `fetch-depth: 0` on that job
alone, and skips with a printed message, never a silent pass, until a first
release exists.

**Agent-event tolerance test.** The `unknown` cases in section 5 are a one-time
manual fix, and nothing stops the next raw-value enum someone adds from
reintroducing G5 silently. Both decoders already have test files —
`apps/shared/AgentKit/Tests/AgentKitTests/AgentEventTests.swift` and
`apps/android/app/src/test/java/com/farcooler/model/AgentEventTest.kt` — so this
is new cases rather than a new harness. Each decoder is fed:

- an event whose variant name it has never heard of, and
- a known variant carrying an unknown value in a nested enum — the case that
  throws today.

Both must yield a gap marker. Neither may throw, and neither may return
something the caller will silently drop.

**Oldest supported** is a single constant per channel naming a single tag, so
raising it is a deliberate one-line decision with a CI failure standing behind
it — not something that happens by accident when someone removes a field.

### 7. Loose ends folded in

- Rewrite `docs/farcooler-design.md:1065`. It promises an N/N-1 window and
  feature hiding, and we are building neither.
- `docs/releasing.md` gets the wire contract next to the relay's. They are now
  one rule stated for two transports, and stating it once for one of them is how
  the wire ended up without it. The channels design rewrites this file anyway;
  the two edits land together.
- `ClientHello` and `ServerHello` each gain `build_number` — `scripts/version.sh
  build`, the commit count. Additive, and the beta channel's floor check needs a
  monotonic number to compare. See the channels design for the policy; this
  design only adds the field.
- Version mismatch reaches all three apps as a typed state rather than a string
  match. `DaemonClient.swift:163` stops matching on `"update the older side"`.
- `TerminalAttach` (`Request` tag 28) and `TerminalAttachResult` (`Result` tag
  12) are in the proto with no daemon handler, no `required_scope` entry, and no
  caller — streaming goes through `--stream` instead. The capability table has
  nowhere to put them and the lint will say so. Reserve both tags with a comment
  saying attach was superseded by the `--stream` data plane, the way tag 30
  already records `WorktreeImport`.
- Delete `farcooler_core::tmux::SCHEMA_VERSION` (`crates/core/src/lib.rs:31`).
  It is written nowhere and compared nowhere; the daemon id tag already
  establishes which daemon owns a pane. A constant that names a mechanism that
  does not exist is worse than no constant.
- `crates/store/src/store.rs:78` refuses to open a database whose
  `schema_version` exceeds `CURRENT_SCHEMA_VERSION`, with a sentence saying the
  daemon is older than its data, instead of running against a schema it does not
  know.

## Out of scope

- **ACP, Claude and Codex adapter protocols.** Drift against third-party CLIs on
  their own release cadence is a real problem and a different one. `crates/acp`
  pins `protocolVersion: 1` and fails the handshake on a mismatch
  (`crates/acp/src/handshake.rs:82`); `crates/agent-core/src/backend.rs:106`
  carries its own error for it. Left alone here.
- **The relay.** Already governed by an additive-only policy with a `/v2/`
  escape hatch, version columns on `devices` and `daemons`, and a stated reason
  in `migrations/0002_versions_and_expiry.sql`. Nothing to add.

## Sequence

Each step is useful on its own and none of them breaks a shipped client.

1. Proto changes and the capability table in `crates/protocol`.
2. Daemon: build the capability list from the table, put it in `ServerHello`,
   enforce `required_capabilities` in the dispatcher, add the error code.
3. Client core: hold the negotiated set, expose it through the FFI and the CLI's
   connection JSON, map the mismatch to a typed state.
4. iOS and Android: gate controls, add the copy. Mac app: same, and drop the
   string match.
5. Agent event tolerance in Swift and Kotlin, and the rule written down in Rust.
6. CI: proto lint, then the old-client test.
7. Docs: the contract in `releasing.md`, the correction in `farcooler-design.md`.

Step 6 is the one worth not deferring. Steps 1–5 are a mechanism; step 6 is the
only thing that keeps the mechanism true a year from now, which is the timescale
the problem is stated on.

## What this does not solve

A daemon **older** than the app is handled: the app asks by name and degrades.
A daemon **newer** than the app is handled by additivity: unknown fields, events
and result variants are ignored, which is the proto3 default and now a stated
rule rather than an accident.

What remains unhandled by design is a client so old that a capability it depends
on has been *removed* from the daemon. On the release channel rule 1 says that
does not happen — a capability name, once shipped in a release, is permanent —
and the old-client test is what holds us to it. If a feature must genuinely go
away, it goes away by the daemon declining to advertise its capability, which
older clients already handle correctly, rather than by deleting the method.

On the beta channel removals *are* permitted after one beta, so this case is
real there. It is covered by the floor rather than by tolerance: a beta daemon
refuses a beta client more than one beta behind, before dispatch, rather than
serving it badly. See the channels design.
