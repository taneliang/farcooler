# TODOS

Deferred work with explicit triggers. An item here is not a plan to do something; it is
a decision to wait, plus the evidence that should reverse it.

---

## Move replay memory to a global daemon-wide budget

**Status:** Deferred, trigger-based
**Surfaced by:** `/plan-eng-review` 2026-07-26, Performance finding P2

**What:** Replace the fixed 8 MiB per-terminal replay buffer with a daemon-wide budget
allocated across terminals, keeping a guaranteed per-terminal floor so a noisy neighbour
cannot evict everyone else's history.

**Why:** The per-terminal cap multiplies by a concurrency the product is explicitly
designed to increase. Five workspaces with four tabs each is twenty terminals and roughly
160 MiB of daemon memory held solely for reconnect, most of it belonging to terminals
nobody is currently watching. That is the dominant term in the daemon's footprint, on a
background service running on the user's own laptop next to the coding agents it launched.

**Why it was deferred:** The fixed cap is predictable, reconnect depth never depends on
what other terminals are doing, and it needs no eviction or fairness policy that a noisy
terminal could exploit. A shared budget is more machinery, and there is no measured need yet.

**Trigger:** Observed daemon memory during the five-workspace acceptance run, or during
design-partner use, indicating the footprint is a real problem. The daemon already reports
total replay usage in host self-health, so the data arrives without extra instrumentation.

**Pros of doing it:** Bounds the quantity that actually matters, total daemon memory,
rather than a per-terminal figure. Busy terminals get depth where it helps; idle ones stop
reserving megabytes to hold nothing.

**Cons:** Requires an eviction policy and a fairness rule. Success criterion 6's fixture,
which is built around a five-minute disconnection fitting inside the buffer, must be
rechecked against whatever floor the allocation guarantees.

**Depends on / blocked by:** Nothing. Reconnect behaviour does not change in kind, since a
terminal exceeding its share emits the same honest `Gap` it already would.

**Where the full reasoning lives:** `docs/farcooler-design.md`, the protocol framing and
replay section, in the bullet stating the per-terminal cost and naming the global budget as
the known answer.
