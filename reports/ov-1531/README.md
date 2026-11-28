# ov-1531: the compose refusal test no longer depends on how fast `ps` is

## Cause

`a_draft_or_a_dialog_is_refused_untyped` sends three times and expects `draft`, `prompt` and `unfamiliar`.
Before the runner looks at the box, it reads which process is in front of the pane (`foreground`). On macOS that
is two `ps` calls, each with a 500 ms deadline. If either one misses it, the send is refused as
`process_unreadable` before the box is read. On Linux, one of the reads is a `readlink` with its own 100 ms
deadline.

I reproduced the CI failure by setting the deadline to 1 ns (`repro-before.txt`). It panics at
`compose_tests.rs:230:5` with `left: "process_unreadable"` and `right: "draft"`, the same as CI.

## Fix

A test-only task-local, `FAKE_FRONT`, names the agent in front of the pane. `foreground` returns it without
reading any process. Only this test sets it. The pane, the screen reads and the refusals are still real.

No timeout changed, nothing is retried or ignored, and release builds don't contain the fake.

- **Can't time out:** with the fake in place and the deadline still at 1 ns, the test passes
  (`fake-with-1ns-deadline.txt`). So no `ps` read with that deadline is left on its path. The fake also returns
  before the Linux `readlink`; I didn't run it on Linux.
- **Still catches a missing refusal** (each removed once, then restored):

| Removed | Result | Output |
| --- | --- | --- |
| The gate's draft refusal | fails: `"unconfirmable"`, expected `"draft"` | `mutation-no-draft-refusal.txt` |
| The gate's dialog refusal | fails: `"unfamiliar"`, expected `"prompt"` | `mutation-no-dialog-refusal.txt` |

The fake's pid is 0, which is no process's. A send that gets past the gate therefore stops at `unconfirmable`
when it looks for claude's session. These runs show that the test notices a missing gate refusal. They don't
exercise the second draft check just before the first paste (`box_still_empty`).

The fake skips real process identification in this test only. `in_front` and `agent_of_process` have their own
tests, and the other compose tests still run real `ps`.

## 200 runs under load

**200 of 200 passed** with the fixed build on macOS (10 CPUs): four batches of 50, with 8 load processes (six
busy loops, plus one shell and the child it keeps forking and execing). The 1-minute load average was 13 to 53,
median 19. The high values are from the start, while the earlier heavy load was still leaving the average. Each
run took 0.8 to 1.7 s. All output is in `runs-8-load.txt`; the script is `run_batch.py`.

### An earlier, heavier run (not the proof)

I first ran the old and fixed builds in turn, 200 times each, under 60 busy loops and 16 fork/exec loops
(median load 102). That was too much for a shared machine: the orchestrator told me it was slowing a landing
check and three other builders, and set the limit of 8. The loop had finished by the time the message reached
me. Its output is kept in `heavy-load/`. The script there is a record of what ran; it expects `load.py` beside it.

- Fixed build: 200 of 200 passed.
- Old build: 199 of 200 passed. The one failure was in test setup (`tests.rs:174`, no pane found for the
  terminal), **not** `process_unreadable`.

So load never reproduced the CI flake here. The evidence that the flake is fixed is the 1 ns pair above. The
pass counts show only that the fixed test holds up under load. The setup failure is a separate, rare problem in
the shared test harness, and this change doesn't touch it.

## Is 500 ms right for the product?

**Not on a heavily loaded machine, given what happens when it expires.**

### Measured `ps` time

300 runs of each read, timed from spawn to exit (`time_ps.py`, `ps-timing.txt`). The machine was already busy
with other work. My processes, like this session, ran at `nice 10`.

| Added load (load average during sample) | Read | p50 | p95 | p99 | max | over 500 ms |
| --- | --- | --- | --- | --- | --- | --- |
| none (about 23) | list, `ps -t` | 9 ms | 63 ms | 125 ms | 128 ms | 0 |
| 20 busy + 4 fork/exec (38 to 40) | list | 20 ms | 35 ms | 82 ms | 111 ms | 0 |
| 60 busy + 16 fork/exec (71 to 102) | list | 293 ms | 686 ms | 1013 ms | 1180 ms | **41 of 300 (14%)** |
| 60 busy + 16 fork/exec | arguments, `ps -p` | 190 ms | 297 ms | 341 ms | 366 ms | 0 |
| 60 busy + 16 fork/exec | environment, `ps -E` | 212 ms | 342 ms | 414 ms | 475 ms | 0 |

At the two lower levels, no read of any kind took over 186 ms.

One send makes several of these reads, including more than one list read, and any miss refuses it. Don't turn
the 14% into a failure rate for sends: the old test ran 200 times at a similar load and never got
`process_unreadable`. I don't know why the two differ. The timing sample was taken while the load was still
climbing. These numbers are from one 10-CPU Mac; I didn't measure a CI runner.

### What a person sees when it expires

From the code (I didn't drive the apps):

| When | What they see | What happens to the message |
| --- | --- | --- |
| Sending from a composer | "The runner couldn’t read the agent pane’s processes, so the message wasn’t sent. Check the terminal." | Refused. It isn't held or retried. |
| CLI send or tell | "the runner couldn’t read the agent pane’s processes, so the message wasn’t sent. check the terminal" | Refused. |
| A message already shown as Queued, on any later try | "Not sent: “…”. The runner couldn’t read the agent pane’s processes. Check the terminal." | **Dropped**, and its attached files are deleted. `process_unreadable` isn't in `STILL_HELD`. |
| A task answer or message waking an agent | Task note: "Not delivered: the runner couldn’t read the agent pane’s processes." If text was pasted, it adds "Check the terminal for text left in the pane." | **Settled for good.** It's never tried again. |
| A message waiting for its Enter, resumed after a runner restart | The "Not sent" line above | Given up. |
| After a first paste | The "wasn’t sent. Check the terminal." wording | Text may be left in the agent's box. |

Stop, Send Now and Bring Here also refuse with `process_unreadable`; I didn't check their wording. While a
message waits for its Enter, the check for whether the agent has gone ignores a missed read.

A held message is the worst case. It can wait up to four hours, with a try every 500 ms while nothing is ahead
of it, and one slow read on any try that reaches the process check discards it. A draft or a busy agent keeps it
waiting, but a slow `ps` doesn't.

### Recommendation (not done here; it needs its own ticket)

1. **Retry when nothing has been typed.** Keep held messages and wakes waiting after `process_unreadable`, and
   give up only if reads keep failing for a set period, as `TORN_FOR` (10 s) does for a half-written registry.
   This matters more than the number. After a paste, keep today's behavior.
2. **Raise the deadline to about 2 s.** A deadline is still needed: ov-1430 added it because `ps` can wait on a
   process that is still in `exec`. 2 s is above the slowest read measured here (1.18 s), from a small sample on
   one machine. The cost is that a stuck read takes longer to report. The two ov-1430 stall tests also expect an
   answer within 2 s, so they would need a new bound.

Other compose tests still run real `ps` and can still fail this way; ov-1526 saw
`compose_pending_tests.rs:139` do so. This ticket changes only the one test it names. `FAKE_FRONT` alone won't
be enough for a test whose send gets past the gate, because later steps also read the process environment.

## Other checks

- `watch::answer_wake::` tests: 467 of 467 passed (`answer-wake-tests.txt`).
- `cargo clippy -p farcooler-daemon --all-targets` exited 0.
- `file-size-budget.py` and `doc-comment-check.py` passed.

## Files

`load.py` makes the load and stops only what it started. Its first cleanup crashed on macOS (`killpg` raised
`EPERM` for a group that had already exited). I fixed it, checked that no load process was left, and removed
that traceback from `ps-timing.txt`. I also reworded that file's first heading to say the machine was already
busy.
