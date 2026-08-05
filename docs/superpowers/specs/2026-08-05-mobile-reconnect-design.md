# Reconnecting on the phones

**Status:** approved
**Applies to:** `crates/client`, `apps/ios`, `apps/android`
**Does not apply to:** `apps/macos` — see "What the Mac already does".

## The complaint

> the apps keep disconnecting

They do not, exactly. They connect once and then never find out that they
stopped being connected. That is worse: a phone showing a fleet it can no
longer reach looks working and is not, and every tap on it quietly does
nothing.

## What the Mac already does

Worth stating, because it is the target and none of it needs building again.

`DaemonClient` reconnects on its own with exponential backoff (1s doubling to
a 30s ceiling, jittered so several machines recovering from one network event
do not retry in lockstep), and falls back to a 5-minute cadence for the two
conditions retrying cannot fix — a daemon that is not installed, and two sides
on different protocol versions. `Reachability` kicks every machine at the two
moments waiting out a backoff is the wrong thing to do: the lid opening, and
the network coming back. There are two manual affordances: "Reconnect all" in
Settings › Machines, and every host dot in the sidebar, which is a button.

The phones have none of it.

## What is actually broken

Four things, and the first two are in the Rust core both phones share, which
is why fixing them once fixes both.

**1. `farcooler_client_connected` reports history, not health.** It answers
whether a `Session` was ever put in the slot. Nothing ever takes one out. A
session whose SSH transport died an hour ago still reads as connected.

**2. Nothing distinguishes a dead link from a refused request.** Every error
crosses the FFI boundary as `e.to_string()`. On the far side, Swift and Kotlin
each have a `Failure` enum that recovers meaning by matching substrings —
which is the right call for the connect path, where the message is genuinely
all there is, and it is not something to extend to every call. Rust still has
the types at the moment the error is produced.

**3. A failed poll is not a disconnection.** Both `Connection.refresh()`
implementations say exactly that in a comment, and they are right about one
poll and wrong about the hundredth. Neither app has any path out of
`Connected` once it is in it. The 3-second poller keeps firing at a dead
socket until the app is force-quit.

**4. Nothing reacts to the app coming back.** The single most common way this
will be experienced is a phone in a pocket for two hours: iOS suspends the
process, the sockets die, and the first thing the user sees on unlock is a
stale screen. Android gates polling on the foreground already but does not
reconnect when it returns. iOS does not even do that — it polls while
suspended, which is one plausible way the session dies in the first place.

## The design

### Part 1 — Rust: a dropped link becomes a fact

`SessionError` gains one variant and one predicate.

```rust
/// The link underneath this session is gone. Distinct from `Protocol`,
/// which is the far side saying something we could not make sense of —
/// that is a session still worth talking on.
#[error("the connection to the host was lost: {0}")]
Disconnected(String),
```

`From<ClientError>` routes the transport-level failures there:
`ClientError::Connect(_)`, and `ClientError::Codec(CodecError::Io(_) |
CodecError::Truncated)`. `CodecError::Framing(_)` stays `Protocol`: garbage on
the wire is not a dead wire.

`ClientError::Closed` and `NoHello` keep their existing mapping to
`DaemonMissing`, unchanged. That mapping is load-bearing at handshake time —
it is what produces "connected, but `farcoolerd --stdio` did not answer", the
message the whole CLI-tools-on-PATH feature exists to resolve — and remapping
it would trade a diagnosis for a shrug.

```rust
/// Whether this means the link is gone rather than the request was refused.
///
/// Only meaningful for a call on an ESTABLISHED session. At connect time
/// these same variants answer a different question — what could not be
/// reached in the first place — and `DaemonMissing` in particular means
/// something specific and useful there. Once a session exists, a closed
/// pipe is a closed pipe.
pub fn is_disconnect(&self) -> bool
```

`dispatch` in `ffi.rs` changes its error type from `String` to `SessionError`.
This deletes about thirty `.map_err(|e| e.to_string())` calls and is the whole
reason the classification is available where it is needed: at the one place
that owns the session slot.

`farcooler_client_call` then does the thing that makes everything downstream
possible:

```rust
// A transport failure is not this request's problem, it is every future
// request's problem. Emptying the slot is what makes
// `farcooler_client_connected` honest, and it is what makes the next call
// fail in microseconds with "not connected" rather than waiting out
// another TCP timeout to learn the same thing.
if outcome.as_ref().err().is_some_and(SessionError::is_disconnect) {
    *guard = None;
}
```

and adds `"disconnected": true` to the pushed envelope so the client knows
which kind of failure it just had, without reading the message.

### Part 2 — a fourth phase, on both phones

`Phase` gains `Reconnecting(attempt)`, between `Connected` and `Failed`.

It is a separate phase rather than a flag because the two existing candidates
are both wrong. `Connecting` means "there has never been a fleet", and it
renders a spinner in place of the whole screen — showing that to someone
reading a terminal because their Wi-Fi blinked would throw away the thing they
were looking at. `Failed` means "stopped, waiting for you", and it is not.

**Reconnecting keeps rendering the last known fleet.** That is the point of
it. The Mac established this rule already — an unreachable machine still
contributes its last good rows, because that is what keeps your mental map of
the fleet stable while a laptop sleeps — and the phones should not disagree
about the same machine.

Transitions:

- `Connected` → `Reconnecting(1)`: a call came back with `disconnected`.
  Detected in `refresh()` only. Every other call site swallows its errors, and
  chasing all twenty of them would buy nothing: the poller runs every three
  seconds, so the drop is noticed within one poll of whichever call first hit
  it, from one place.
- `Reconnecting(n)` → `Connected`: an attempt succeeded. Repositories are
  re-read, the poller restarts, and live terminal streams are relinked (below).
- `Reconnecting(n)` → `Reconnecting(n+1)`: an attempt failed for a reason
  worth retrying.
- `Reconnecting(n)` → `Failed(msg)`: the attempt failed for a reason retrying
  will never fix — the key was rejected, the host key changed, this device has
  no identity. The existing `Failure` classification already answers this and
  is reused rather than duplicated.

Backoff is `min(30, 2^attempt) * random(0.8...1.2)` seconds — the same numbers
as `DaemonClient.backoffSeconds`, deliberately, so "how long until it comes
back" has one answer across three apps.

### Part 3 — the three things that skip the backoff

A timer cannot know that you just walked back into Wi-Fi range. Three signals
say "now is a better moment than the one the timer picked", and all three land
on the same `reconnectNow()`:

**The app becoming active.** iOS: `scenePhase == .active`. Android:
`Lifecycle.State.RESUMED`, which already calls `setForeground(true)`. If the
connection is reconnecting or failed, retry at once; if it thinks it is
connected, poll immediately rather than waiting out the 3-second interval —
after two hours suspended, "connected" is a claim worth testing straight away.

**The network coming back.** A new `Reachability` on each phone, mirroring the
Mac's: `NWPathMonitor` on iOS, a `ConnectivityManager.NetworkCallback` on
Android. Only the transition into reachable fires; a path that was already
satisfied is not news.

**A person asking.** The chip, below.

iOS also gains the foreground gate Android already has: the poller does not
fire while the app is not active. A poll is an SSH round trip and a radio
wake-up, and nobody is reading the answer.

### Part 4 — the manual reconnect

**iOS:** a status chip at the trailing edge of `HostSwitcherBar`, which is
already the strip that says which machine you are looking at and is already
present under every phase including the ones you cannot otherwise escape.

```
┌──────────────────────────────┐
│  [worktree list]             │
├──────────────────────────────┤
│ 🖥  my-mac ⌃⌄   ● Reconnecting│
└──────────────────────────────┘
```

The dot is always there; the label appears only when there is something to
say. Connected is a green dot and no words — a permanent "Connected" on a
phone screen is noise, and the absence of amber is the same information.
Connecting and reconnecting are amber with a label. Failed is red with
"Disconnected". Tapping reconnects now, from any state, including green — that
is the "it's actually cooked" case, where the app believes it is fine and the
user knows better.

**Android:** `MachineStatusRow` already exists for exactly this and already
returns early when a machine is connected. It gains a `Reconnecting` branch
with the attempt-aware text and a "Reconnect now" button. Android shows every
machine at once (`FleetRepository`), so per-machine is the only shape that
works there and this row is already it.

### Part 5 — live terminal streams

A stream is a second SSH channel on the session that just died. `TerminalSession`
already survives this — its `onEnd` retries once and then settles for polling —
so the screen recovers on its own but stays on the slower path until the view
is rebuilt.

`Connection` publishes a `reconnectGeneration` counter, incremented on every
successful reconnection. `TerminalView` watches it and calls a new
`TerminalSession.relink()`: the same work `switchTo` does, minus the id change
and minus handing the old pane its size back — that pane is on the other side
of a link that no longer exists, and asking it anything is a request into the
void.

## What this deliberately does not do

**Reconnect the macOS app differently.** It already does all of this and has
for longer. Changing it to share code with the phones would be a refactor
justified by symmetry alone, and the two are not actually symmetric: the Mac
drives a CLI subprocess and an event stream, the phones drive an in-process
`russh`. The numbers are shared by being written down; the code is not.

**Retry forever at 30 seconds for conditions retrying cannot fix.** The Mac
drops to a 5-minute cadence for "not installed" and "version mismatch". The
phones already classify both — `Failure.DAEMON_MISSING`, and the version
message — and both go to `Failed` rather than into the backoff loop, because a
phone showing a failure screen with an actionable message is better than a
phone quietly retrying something that will not work. `Failed` is not a dead
end: the chip, the activation signal and the network signal all still reach it.

**Queue writes made while disconnected.** Sending a keystroke that lands four
minutes later, out of order, into an agent's prompt, is worse than not sending
it. Calls made while the link is down fail immediately, which is the honest
answer and is now fast.

## Testing

Rust changes are covered by unit tests in `crates/client`: the `From<ClientError>`
mapping and `is_disconnect()` are pure functions over an enum and get a table
test each. The session-slot clearing is covered in
`crates/client/tests/against_a_real_daemon.rs` by killing the daemon under a
live session and asserting the next call reports `disconnected` and that
`farcooler_client_connected` goes false.

Neither phone app has a unit-test target for view code, and neither is gaining
one for this. `Connection`'s backoff schedule is a pure function of the attempt
count and is testable on Android (`app/src/test/`), which has a JVM test
source set; on iOS it is verified by reading it against the Android one, which
is the same arithmetic. The rest is verified by building both apps and dropping
the link on purpose.
