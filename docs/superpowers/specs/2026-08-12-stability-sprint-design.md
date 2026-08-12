# Stability sprint: surviving a pane that stops answering

Audit of Far Cooler's crash and degradation paths, and the design for fixing
them. Written 2026-08-12, from three real crash reports, the live machine's
process table, and measurements against the running fleet.

## What prompted it

Two reports from the user:

- Switching panes and layouts on the Mac was slow — blank, then a few flashes at
  the wrong size. Fixed separately in `perf: open a pane without waiting for
  anything that was not happening`, which this branch is stacked on.
- "If a particular terminal hangs — e.g. it's running a process that OOMs —
  farcooler itself starts crashing. The farcooler daemon doesn't seem resilient
  to failure modes like this."

The second is the subject here.

## What the audit actually found

The codebase is not full of landmines, and it is worth saying so plainly before
listing what is wrong. The daemon has **three** non-test `unwrap`/`expect` calls
in the whole crate. There is no `try!` and no force-cast anywhere in either app.
The fanout is bounded, hangs up watchers that fall behind, and reclaims itself
after an idle grace. That discipline is why the failures below are all *design*
failures rather than sloppiness, and why each has a single specific fix.

### F1 — `UNUserNotificationCenter.current()` aborts every unbundled build

`apps/macos/Sources/FarCooler/Notifications.swift:29`, reached from
`ContentView.swift:171` on appear.

`UNUserNotificationCenter.current()` raises `NSInternalInconsistencyException`
when the running executable has no valid bundle identifier. Swift cannot catch
an Objective-C exception, so the process aborts.

**This is 2 of the 3 crash reports on this machine** (2026-08-06 00:29 and
2026-08-11 23:42, both `SIGABRT` with an identical stack through
`Notifier.requestAuthorization()`). It is also exactly what the project's own
README tells you to run at line 134:

```sh
FARCOOLER_BIN=$PWD/target/release/farcooler ./apps/macos/.build/debug/Far Cooler
```

A `swift build` product is not an app bundle, so the documented way to run the
app crashes it on launch.

### F2 — one failed `list-panes` reports every terminal on the machine as `Lost`

`crates/core/src/derive.rs:57`:

```rust
if !snapshot.inventory_healthy {
    return DerivedTerminal { state: TerminalState::Lost, orphan_candidates: Vec::new() };
}
```

This is the reported symptom, and the mechanism is already written down in the
codebase. `crates/tmux/src/server.rs:176`:

> `send-keys` writes to a pane's pty. A program that never reads its input …
> eventually lets that buffer fill, and then the write blocks. tmux blocks with
> it, this call blocks with tmux, and because a client connection answers one
> request at a time, every other terminal's requests queue behind a pane nobody
> is even looking at.

The 1 s `TMUX_COMMAND_TIMEOUT` bounds each command but does not prevent the
cascade — it converts it. A wedged pane makes `list-panes` time out; a timed-out
`list-panes` yields `RuntimeSnapshot::unavailable()`; an unavailable snapshot
makes **every terminal on that machine** derive `Lost`. The whole fleet goes red
because one pane stopped reading its stdin.

The fix has to respect the invariant the product is built on. The README is
emphatic that runtime state is derived and that `LOST` is preferred to a guess,
and that is correct — but it answers a different question than the one being
asked here. `Lost` means *we looked, and nothing proves this is alive*. An
unavailable inventory means *we could not look*. Reporting the second as the
first is not conservatism; it is a false negative asserted with confidence, and
it is the one thing the derivation rule exists to prevent.

The protocol already carries the distinction — `runtime_healthy` is on the wire
(`crates/daemon/src/wire.rs:60`) and decoded by the Mac (`Model.swift:24`) — it
is simply not acted on.

### F3 — the inventory gives up after a single timed-out read

`LiveInventory::refresh` (`crates/tmux/src/inventory.rs:42`) calls
`list_tagged_panes()` once and declares the inventory unavailable if it fails.

A read that lost a race with one blocked write is not a broken tmux server. One
retry costs a few milliseconds in the failure case and nothing at all in the
common one, and it converts most instances of F2 into a hiccup nobody sees.

### F4 — a panic anywhere in the client core aborts the phone apps

`crates/client/src/ffi.rs` exposes 12 `extern "C"` entry points and guards none
of them with `catch_unwind`. Since Rust 1.71 a panic unwinding out of an
`extern "C"` function aborts the process, so any panic in the core is a `SIGABRT`
in the iOS and Android apps with no diagnostic beyond the signal.

The reachable panic surface is small but it is a genuine cascade: every shared
structure is taken with `.lock().expect("queue")` (lines 250, 403, 418, 497,
1126, 1134, 1160, 1165). A `std::sync::Mutex` is poisoned when a thread panics
while holding it, so **one** panic in a spawned task — which tokio would
otherwise absorb harmlessly — permanently poisons the queue, and every
subsequent `farcooler_client_poll` from Swift then aborts the app. A recoverable
fault is converted into an unrecoverable one, and then into a crash.

### F5 — the test suite leaks tmux servers, and the leak degrades the machine

Measured on this machine right now:

| | count |
|---|---|
| tmux sockets under `/tmp/tmux-501/` | **3 069** |
| live `tmux -L farcooler-*` server processes | **362** |
| of those, using the `farcooler-test-` prefix | 28 |

Each leaked server holds a session, a pane, and an interactive `fish`. They are
test fixtures — the ones still running are named `demo-stdio`, `demo-agent-test`
and similar — but only 28 carry the `-test-` prefix, so most are indistinguishable
from a real install by name.

This is not only untidiness. It has two measurable consequences:

- **It slows the product being measured.** `capture-pane` against the live server
  was timed at 50 ms at rest and **740 ms** under this load. tmux is
  single-threaded per server, and 362 servers is real contention for CPU.
- **It makes a test fail deterministically.**
  `an_exited_command_is_observed_as_dead_not_silently_gone`
  (`crates/tmux/tests/live_tmux.rs:92`) sleeps a flat 400 ms and then asserts the
  pane is dead. On a loaded machine 400 ms is not enough; the test now fails on
  every run, on unmodified `main` as well as on this branch. It was passing
  earlier in the same session. A timing assumption that decays as the machine
  fills up is a test that will keep costing time and teaching people to ignore
  it.

### F7 — `farcoolerd` races itself, and the losers never exit

Found while chasing the last leaking test, by asking why the machine had **63
live daemons**. Their start times gave it away: they came in groups sharing a
timestamp to the second — `19:05:34` ×3, `19:29:49` ×3, `00:41:13` ×2 — which is
several clients auto-starting the daemon at the same moment.

`crates/daemon/src/main.rs` probed the socket at line 114 and bound it at line
150. In between: `Service::open` (SQLite plus a full tmux inventory),
`backfill_pane_tags` walking every pane, `resume_agent_listeners`, and spawning
the watcher. A hundred milliseconds or more of check-then-act.

So every racer probed, found nothing listening, did all that work, and bound.
`UnixListenerServer::bind` unlinks a stale socket — correct for a dead one — so
the last to arrive silently unlinked everyone else's. The losers were never told
and never exited. Each went on holding the database open and running a `Watcher`
that captures every live pane once a second, unreachable by any client.

This is very likely the largest contributor to the contention blamed on leaked
test servers above: 34 orphaned daemons on the default install, each sampling
the same single-threaded tmux server every second.

The fix is an advisory `flock` on `<runtime>/farcoolerd.lock`, taken before any
of the expensive setup. The kernel grants it to exactly one process and releases
it when that process exits — crash included — so there is no stale lock to
detect and no pid file to disbelieve. The socket probe stays as a cheap fast
path; the lock is what makes it correct.

Verified both directions: `crates/daemon/tests/one_daemon_per_home.rs` starts
eight daemons against one runtime directory at once and asserts one survives.
With the lock, one does. With the lock neutralized, **six of eight** survive.

### F6 — iOS input accessory view crash: already fixed, no action

The third crash report (2026-08-11 10:35, `FarCooler` on device) is an
Objective-C exception from
`-[UIView(Hierarchy) _associatedViewControllerForwardsAppearanceCallbacks:performHierarchyCheck:]`
while UIKit installed the composer as an input accessory view.

This was fixed in `17fd186` at 11:37 the same morning — one hour after the crash
— by deliberately not calling `addChild` on the hosting controller. The reasoning
is recorded at `apps/ios/FarCooler/DockedBar.swift:139`. Verified present; no
change needed.

## The design

Five changes, each addressing one finding, in the order they matter.

### 1. A client that cannot be crashed by asking for permission

`Notifier.requestAuthorization()` returns early when
`Bundle.main.bundleIdentifier` is nil, and every other entry point that touches
`UNUserNotificationCenter` is gated the same way. An unbundled build then runs
with no notifications instead of not running.

Nothing is silently swallowed: the reason is logged once, and it is the honest
one — an unbundled build cannot receive notifications, which is a property of how
it was launched, not a failure to be repaired at runtime.

### 2. `Unknown` is not `Lost`

Add `TerminalState::Unknown`, returned by `derive_terminal` when
`inventory_healthy` is false, in place of today's `Lost`.

This *strengthens* the derivation rule rather than weakening it. `Unknown` never
claims a terminal is running, so the invariant the README states — that no stale
`running` can ever be shown — is untouched. What it stops doing is claiming a
terminal is *gone* on the strength of a read that never completed.

Both clients render it as the machine being unreadable rather than the terminal
being dead, and the actions that only make sense for a genuinely lost terminal —
`dismiss-lost` above all — are not offered for it. A terminal must never be
dismissed as lost because tmux was busy for a second.

### 3. One retry before declaring the runtime unreadable

`LiveInventory::refresh` retries `list_tagged_panes` once, after a short pause,
before falling back to `unavailable()`. Reads only. A retry on a write would risk
performing it twice, and nothing here needs that.

### 4. An FFI boundary that cannot abort the app

Every `extern "C"` function in `crates/client/src/ffi.rs` runs its body inside
`catch_unwind` and returns that function's own failure value if it unwinds — 0
for a ticket, false for a predicate, null for a pointer. A panic becomes an error
the app can show instead of a signal it cannot.

Separately, every `.lock().expect(...)` becomes poison-tolerant
(`unwrap_or_else(PoisonError::into_inner)`). The data behind these locks is a
queue of finished results; a panic elsewhere does not make its bytes untrustworthy,
and refusing to look at them ever again is a strictly worse outcome than reading
them.

### 5. Tests that clean up after themselves, and do not assume a fast machine

- Every live tmux test kills its server on the way out, including on the failing
  path, so a failed assertion stops leaking a server for the rest of the machine's
  life.
- `an_exited_command_is_observed_as_dead_not_silently_gone` polls for the
  condition it is waiting for, up to a generous deadline, instead of sleeping a
  fixed 400 ms. The assertion is unchanged; only the waiting is.
- A `scripts/reap-stale-tmux.sh` to clear what has already accumulated, since
  3 069 sockets will not remove themselves.

## What shipped, and where it differed

All five are implemented in `fix: stop one stalled pane from speaking for the
whole machine`. Three things were decided during implementation rather than
here, and are worth recording because they are judgement calls:

- **`Unknown` counts as live for the workspace's own state.** While the
  inventory is unreadable every terminal derives `Unknown`, so the workspace has
  no evidence at all — and all three of `Error`, `Ready` and `Active` are
  claims. `Active` is the only one that leaves the sidebar where it was rather
  than flashing every workspace on the machine red and back. The honest signal
  lives on `runtime_healthy`, which is a per-machine fact and the right shape
  for it; a per-workspace state cannot say "ask the machine, not me".

- **Both apps keep a pane mounted while its state is `Unknown`.** This was not
  in the plan and matters more than the rest of the state work. The byte stream
  behind a terminal is its own channel, unaffected by a failed `list-panes`, so
  treating `Unknown` as not-live tore down a working stream and replaced a
  terminal somebody was reading with "no running session" — for a hiccup usually
  over before the view finished rebuilding. In a tiled layout it did that to
  every pane in the window at once.

- **The daemon and the stdio tests leaked servers too.** The plan named only the
  tmux crate's tests. `crates/daemon/src/test_support.rs` and
  `crates/daemon/tests/stdio_transport.rs` had the same hole and got the same
  guard. `rpc_over_socket.rs` already had one, with the same reasoning written
  out — so this was a known problem that had been fixed in one place out of
  four.

Measured after: full workspace suite green (49 binaries, 0 failures), both apps
build, and a full run leaks one live tmux server rather than four.

### Cleaned up on the machine itself

Both reapers were run against the development machine, with the live fleet
verified healthy afterwards (`runtime_healthy: true`, every terminal still
running):

| | before | after |
|---|---|---|
| tmux servers | 376 | 5 |
| tmux sockets | 3 364 | 11 |
| `farcoolerd` processes | 66 | 10 |
| `capture-pane` on the live fleet | 50–740 ms | under 5 ms |
| `terminal stream` time to first byte | 144 ms median | 20 ms median |

The last row is worth reading twice. The earlier measurement of this — 214 ms
down to 89 ms — was taken on a machine carrying 362 tmux servers and 34 orphaned
daemons, and was measuring the contention as much as the code. With that cleared,
the same change is 144 ms down to 20 ms.

`scripts/reap-stale-tmux.sh` grew two safety rules while being made fit to run:
it spares every install a *live* daemon is using rather than only the default
one — three daemons with their own `FARCOOLER_HOME` were running at the time,
and the first version would have killed their servers — and it keeps any server
whose panes sit outside a temp directory, on the grounds that a session nobody
is attached to is still somebody's work.

`scripts/reap-orphan-daemons.py` is Python rather than shell because it walks
paths containing spaces, which is exactly where the shell version of it went
quietly wrong.

### Left open

- **Two daemons on a runtime directory nothing is listening on.** The reaper
  refuses these deliberately: with no listener there is no owner, so there is no
  safe way to choose which to keep. They are another agent's scratch homes and
  will go when that job does.

## Out of scope, deliberately

- **Panes remounting on every switch.** Keeping every visited pane mounted, the
  way iOS's `PaneHost` already does, would make switching show stale content
  instantly instead of blank. It is the right next change and it is a structural
  one, not a stability fix.
- **The per-second `capture-pane` sample.** `watch.rs` captures every live pane
  every second against a single-threaded server. Worth revisiting — control-mode
  streaming is the real answer — but it is a redesign, not a repair.
- **The CLI-subprocess-per-call transport.** Every Mac action spawns a
  `farcooler` process; `layout show` measured 60–220 ms. The socket path already
  exists in `daemon_link.rs`. Migrating live-runtime calls onto it is its own
  project.
