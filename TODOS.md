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

**Depends on / blocked by:** Nothing. Reconnect behavior does not change in kind, since a
terminal exceeding its share emits the same honest `Gap` it already would.

**Where the full reasoning lives:** `docs/farcooler-design.md`, the protocol framing and
replay section, in the bullet stating the per-terminal cost and naming the global budget as
the known answer.

---

## Repository-scoped client authorization

**Status:** Deferred, trigger-based
**Surfaced by:** `/plan-eng-review` 2026-08-05, Review design, scope finding

**What:** Extend `Scope` beyond the host-wide `READ` / `CONTROL` / `HOST_ADMIN` so a client can be
authorized for specific repositories rather than the whole machine.

**Why:** The Review design wanted "this phone may read the code in one repository." That is not
expressible today, so review methods that return file content were placed at `CONTROL` — the same
scope that can drive every terminal on the machine. A device that should only be able to read one
project's diffs must instead be trusted with the whole host. It also forced `review.inbox` to
return bare counts at `READ` rather than the scoped view it was designed for.

**Why it was deferred:** It touches enrollment, the granted-scope wire field, every method
contract in the table, and the scope checks in all three clients. The Review feature has a correct
and safe answer without it — `CONTROL`, which exposes nothing a terminal screen did not already
expose — so this buys granularity, not safety.

**Trigger:** A second person or a shared machine. The moment a host is used by anyone other than
its owner, or a device is enrolled that should not be able to drive terminals, host-wide scope
stops being adequate.

**Pros of doing it:** Makes least privilege expressible. Enables an untrusted-ish device (a work
phone, a borrowed laptop) to review code without the ability to run commands.

**Cons:** A protocol-wide change with a migration for existing enrollments, and a scope model that
every future method has to reason about in two dimensions instead of one.

**Depends on / blocked by:** Nothing technically. Best done before, not after, a second scoped
surface exists, because retrofitting two is harder than one.

**Where the full reasoning lives:** the Review design doc, "Review reads source, so review
requires `CONTROL`".

---

## A real read-only execution mode for agents

**Status:** Deferred, trigger-based
**Surfaced by:** `/plan-eng-review` 2026-08-05, Review design, ask-dispatch finding

**What:** A daemon-enforced mode in which an agent session provably cannot modify the worktree.

**Why:** Review's "ask a question, change no code" path has no enforcement worth the name.
`AgentSetMode` carries an adapter-defined string and enforces nothing. `fs_guard::confine` is
**path confinement, not a sandbox** — it stops a write escaping the worktree, it does not stop a
write. And an agent that shells out to `sed`, a formatter, or `git checkout` bypasses the daemon's
filesystem service entirely. The design settled for detection: compare the worktree digest before
and after, and report loudly if it moved.

**Why it was deferred:** Real enforcement means either adapter cooperation that does not exist
across Claude Code, Codex and Cursor CLI, or an OS-level sandbox per agent process — seatbelt on
macOS, a mount namespace or landlock on Linux — which is a substantial platform-specific surface
on a product that currently launches agents as ordinary tmux panes.

**Trigger:** An ask session that silently edited code and cost real work, or a user asking for a
"look but don't touch" agent as a product feature rather than a review detail.

**Pros of doing it:** Turns a promise into a guarantee. Also useful well beyond review — a
read-only agent is the safe default for exploring an unfamiliar repository.

**Cons:** Per-platform sandboxing is its own maintenance burden, and a sandbox that breaks a
legitimate tool invocation is worse than an audit that reports honestly.

**Depends on / blocked by:** Nothing. The audit path ships first either way and remains the
fallback for adapters that shell out.

**Where the full reasoning lives:** the Review design doc, the Dispatch section's Ask subsection.

---

## Measure filesystem-watcher cost on a large monorepo

**Status:** Deferred, measurement-first
**Surfaced by:** `/plan-eng-review` 2026-08-05, Review design, stage 1

**What:** Determine whether a `notify`-style watcher over a worktree is viable at monorepo scale,
or whether the two-stage signature poll should be the only invalidation source.

**Why:** The change-set cache needs to know when the working tree moved, and `watch.rs` cannot
tell it — that loop samples tmux panes for agent activity and observes no file content and no ref
movement. The design adds a watcher subscribed only while a review surface is open. On a worktree
with 200 000 files that may exhaust inotify watches on Linux or coalesce unhelpfully under
FSEvents.

**Why it was deferred:** The design already carries three other invalidation sources — agent writes
through the daemon's own filesystem service, a two-syscall signature check on the existing
one-second loop, and an explicit Refresh. The watcher is a latency improvement, not a correctness
requirement, so it can be measured and dropped without changing what the user can trust.

**Trigger:** Stage 1 implementation. Measure watch-descriptor count and CPU on the largest
repository available before committing to the dependency; the workspace currently has no `notify`
and adds dependencies reluctantly.

**Pros of doing it:** Sub-second reflection of an editor save with no polling cost.

**Cons:** A new dependency with well-known platform-specific failure modes, in a workspace whose
`Cargo.toml` justifies a single HTTPS client in a five-line comment.

**Depends on / blocked by:** Nothing. Falls back to signature polling with no user-visible change
beyond latency.

**Where the full reasoning lives:** the Review design doc, the `ChangeSet` cache-invalidation
bullets, and Open Question 5.

---

## Bridge review entries to GitHub PR comments, both directions

**Status:** Deferred, trigger-based
**Surfaced by:** `/office-hours` and `/plan-eng-review` 2026-08-05, Review design, Open Question 3

**What:** Publish review entries as PR comments, and pull human reviewers' PR comments back into
the buffer so a teammate's comment dispatches to an agent exactly the way your own does.

**Why:** This is the one thing the Review design makes possible that nothing else does. The
insight underneath review-as-capture-and-dispatch is that a comment aimed at an agent is an
instruction, not a message. A human reviewer's comment on a PR is the same instruction wearing
different clothes, and today it gets retyped by hand into an agent. Closing that loop turns
Far Cooler from a personal tool into something that changes how a team reviews.

**Why it was deferred:** The whole Review design is deliberately read-only toward GitHub. Writing
means comment identity, threading, resolution state, edit and delete, permission handling, and a
sync model where the same comment exists in two systems that can both change it. That is a
product, not a feature.

**Trigger:** Stages 1 through 6 shipped and in daily use, plus at least one other person reviewing
Far Cooler branches on GitHub. Without a second reviewer there is nothing to bridge.

**Pros of doing it:** The team story. Also the strongest reason to believe this product is more
than one person's workflow.

**Cons:** Bidirectional sync with a system you do not control, and the first place Far Cooler would
write to a third party rather than read from one.

**Depends on / blocked by:** Stage 6 (PR state, read-only) must exist first.

**Where the full reasoning lives:** the Review design doc, Open Questions.

---
## Schedule the live agent suite, once someone wants it unattended

**Status:** Deferred, trigger-based
**Surfaced by:** Stage 1 of the agent-activity work, 2026-08-16

**What:** Run `cargo test -p farcooler-core --test live_agents -- --ignored` on a
schedule, rather than only when a person or an agent asks for it.

**Why it is not done:** The suite itself exists — see the Tests section of the README —
and it is deliberately on-demand. Everything that made it worth writing is in the test
file: which binary is the real one rather than a wrapper, how each trust gate is
dismissed, the cheapest model per agent, and why assertions are on furniture rather than
on model output. What is genuinely deferred is only the scheduling.

**Trigger:** Wanting to learn about a third-party release without anyone running
anything — most likely once several people depend on this, or once a drift has gone
unnoticed long enough to matter.

**Cons:** It costs a few cents per run and needs the CLIs signed in, so it belongs on a
machine with real logins rather than in CI, and a scheduler that fires while nobody is
watching needs somewhere to report a failure that someone actually reads.

**Depends on / blocked by:** Nothing.
