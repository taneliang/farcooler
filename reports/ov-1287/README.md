# ov-1287: cancelling a gate can be refused

## Summary

Cancelling a native step could end as an error instead of a safe failure. This happened when the agent's process exited by itself during the cancel, before the runner had stopped it. Fixed in `quiesce_native` (`crates/daemon/src/workflow_runtime/coding_native.rs:239`): whichever is seen first, the owner's own exit or the runner's stop, the proof is the same (nothing is left on the owner's tty) and so is the answer.

## Reproduction on the old code

I looped the single test 300 times, 4 at a time, on the unchanged code (commit `69021450e`):

```
workflow_runtime::gate_tests::cancelling_parallel_review_ends_the_owned_gate_before_returning
runs=300 failures=10
   7 Refused("native owner exited without observed process quiescence")
   3 Refused("native live tty has no process facts")
```

All 10 panicked at `gate_tests.rs:456`, the same place as main's CI run 43774779075. The second message is the same race seen a moment earlier: the process had already left its tty, but tmux had not yet marked the pane dead.

## The interleaving

1. The test cancels as soon as the reviewer's receipt exists. The fake reviewer writes its receipt and then calls `hook('Stop')`, which waits for the runner's `{}` reply (`gate_tests.rs:17`).
2. The cancel makes `controlled` return (`native_control.rs:89`). That drops the turn future, and with it the accepted hook connection. The reviewer gets EOF instead of `{}`, its assertion fails, and Python exits. When the shell running in the pane exits, the pane ends.
3. `run_native` then calls `quiesce_native` (`coding_native.rs:513`). On the old code, this:
   - refused if the pane was no longer `Live` (old line 231), or
   - refused if the pane was `Live` but nothing was left on its tty (old line 246).

   Only when the runner got there before the exit did it stop and kill the processes and return `Ok`, which gave the expected `SafeFailure`.

The outcome therefore depended on which happened first: the reviewer's exit, or the runner's look at the pane and its processes. A busier machine shifts the timing, but a slow machine was not the cause.

## The fix

`quiesce_native` now reads the pane's life and the processes on its tty together, and decides from both:

| Pane | Processes on the tty | Before | Now |
|---|---|---|---|
| Live | some | stop, kill, close | same |
| Live | none | refused | an exit tmux hasn't noticed yet: read again (with a reap nudge) until tmux reports the pane dead, for about 1 s, then refuse |
| Dead | none | refused | close the window, `Ok` |
| Dead | some | refused | the tty is still tmux's, so they are the owner's: stop, kill, close |
| Gone | none | refused | `Ok`, as `cancel_interrupted_native` already decided for this case |
| Gone | some | refused | refused ("native vanished pane still has processes") |

A pane that disappears between the life read and the inventory is read again instead of refused. A pane whose tags or tty differ from the retained owner is still refused. Closing a window that tmux has already closed counts as closed (`close_native`, line 82).

## Deterministic tests

These are in `coding_native.rs`, `mod quiesce_order_tests`. They use a real tmux pane and call `quiesce_native` directly.

- `the_runner_stopping_a_live_owner_first_ends_and_closes_it`: quiescence goes first. Passed on the old code too.
- `an_owner_that_exited_before_quiescence_looked_is_quiescent`: the owner exits, and tmux marks the pane ended before quiescence runs. **Failed on the old code** with "native owner exited without observed process quiescence".
- `an_owner_that_exits_after_its_pane_was_seen_live_is_quiescent`: a test seam (`AFTER_LIVE`, `#[cfg(test)]` only) kills the owner after the pane was read as live and before its processes are read. **Failed on the old code** with "native live tty has no process facts".

On the fix, all three pass.

## Verification on the fix

- The named test, looped 300 times at `-j4`: **300 passed, 0 failed**. Every log shows `1 passed`, so the filter matched each time.
- `a_verdict_survives_the_daemon_stopping_before_the_turn_ends`, looped 100 times on the fix: **100 passed**. It isn't exposed to this race. Its `nostop` reviewer sleeps after writing its receipt, so no hook is waiting for a reply when the cancel comes, and the agent doesn't exit. I didn't loop it on the old code.
- `cargo nextest run -p farcooler-daemon -j4 workflow_runtime`, in the foreground: 455 of 457 passed. The two failures need the `farcooler` CLI binary (`cargo build -p farcooler-cli --bin farcooler`, which their assertions name). After building it, both passed.

## Was production cancel affected?

Yes, though more rarely than the test shows.

- **Trigger:** a native agent (builder or reviewer) whose process exits on its own during a cancel or Stop, between the moment the turn is abandoned and the moment the runner stops it. In production, the hook CLI losing its reply doesn't make Claude or Codex exit, so the test's exact trigger is mostly a fixture effect. An agent that crashes, quits, or finishes and exits at that moment hits the same refusal.
- **Effect on the old code:**
  - `execute_coding` returned `Err`, which the engine settles as `Outcome::Uncertain` (`workflow_runtime.rs:645`).
  - The Stop that follows a cancel goes through `cancel_interrupted_native`. If the pane was `Dead` (kept after exit), that called the same `quiesce_native` (`coding_native.rs:573`) and refused again.
  - So a cancelled gate — the orchestrator's main lever on a stuck landing — could end Uncertain and need a person to clear it, instead of ending cleanly.
- **Behavior change to note for review:** a native session lost mid-turn (the pane died with no receipt, `coding_native.rs:349`) now also quiesces, as long as nothing is left on its tty. It is recorded as a turn with no completion, as a timed-out turn is, instead of refusing into Uncertain. I didn't trace that path to its final outcome; the `workflow_runtime` suite has no test that expects the old refusal. A dead pane with processes still on it is still killed under proof, and a vanished pane with processes on its tty is still refused.
