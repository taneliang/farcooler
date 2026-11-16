# ov-1299: a detached caller isn't a person

Carries ov-1296, whose build was cut off by the app restart on Nov 16 at
18:10. Its saved patch (`interrupted.patch`, against 3cdb5b23e) was applied and
finished in da2e15a4b.

## What changed

- **`pane_of_ours`** (`crates/daemon/src/caller_seat.rs`) now refuses
  `Seat::Unreadable` with `scope-denied`. That is the seat an agent pane's
  command reads as once it is detached from the pane: `(cmd &)`, `nohup`,
  `setsid`, a double fork, or a launchd, at or cron job. Before, it counted as
  a person, so a pane could remove another lane's dirty tree or move it with
  `worktree.assign`. `Agent(None)` (an agent outside this daemon's panes, such
  as the owner's claude in Terminal) is still a person. No other refusal
  changed.
- **Tests:** `people_tests.rs` keeps the under-an-agent-outside case as a
  person (`a_caller_under_an_agent_outside_the_runner_removes_as_a_person`)
  and flips the left-behind case into two refusal tests:
  - `a_detached_caller_does_not_remove_a_worktree`
  - `a_detached_caller_does_not_assign_a_worktree`, covering both a claimed
    tree and a loose one

  Both tests cover a caller orphaned to pid 1 and a caller with no pid.
- **Real-CLI harnesses** (`crates/cli/tests/removing_asks_for_its_name.rs` and
  the Mac `RealCLIConfirmationTests`) now start their scratch daemon with
  `FARCOOLER_TEST_STUB_AGENTS=1`. In that mode every ancestor vouches and
  `stubbed` maps `Agent(None)` to a person, so the test counts as the person it
  stands in for. That holds under an agent's pane and with no terminal.

## Evidence

- **The new tests fail without the fix.** With the old arm
  (`Seat::Person | Seat::Agent(None) | Seat::Unreadable => Ok(None)`) put back
  for a moment, both `a_detached_caller_*` tests fail at their first
  assertion (`left behind`). They pass with the fix.
- **`cargo nextest run -p farcooler-daemon -j 4`:** the whole crate ran:
  3854 tests, 3851 passed, 3 failed.
  - Two of the failures, `accepted_native_claude_delivers_…` and
    `workflow_native_steering_continues_…`, need the `farcooler` CLI binary,
    which the fresh worktree hadn't built yet. They pass once it is built.
  - `workflow_coding_cancel_running_owned_process_…` failed under load
    (average about 60) and passed on rerun.
  - All three pass when run together with the new tests.
  - The tool moved the 21-minute run to the background at its 10-minute
    limit. It was the only cargo job running.
- **`cargo nextest run -p farcooler-cli -j 4 --no-fail-fast`:** 703 of 703
  pass. Run from inside an engine-launched agent session, it first had 8 hook
  test failures. The session's `FARCOOLER_NATIVE_HOOK_SOCKET` and the other
  engine `FARCOOLER_*` variables are inherited, so the hooks under test report
  to the session's own socket instead of the test's. With those variables
  unset, everything passes. This is a harness hazard and predates this change.
- **Scripts:**
  - `./scripts/daemon-layers.py`: ok.
  - `./scripts/file-size-budget.py`: ok.
  - `./scripts/lane-gate.py --self-test`: ok. The first try failed its
    wall-clock check, "a light job does not wait for the lock" (under 0.8 s),
    while cargo was compiling beside it. It passed on rerun.

## Commands for the real-CLI tests under an agent and with no terminal

Run these from inside an agent pane's process tree (here, a claude session the
engine launched, where `tty` prints `not a tty`):

```sh
# Unset the engine session's variables first, or the CLI's hook tests talk to it.
for v in $(env | grep -o '^FARCOOLER_[A-Z_]*' | grep -v WORKOS); do unset $v; done
export PATH="$HOME/.cargo/bin:$PATH" CARGO_BUILD_JOBS=4

# Under the pane, no terminal: passes.
cargo nextest run -p farcooler-cli --test removing_asks_for_its_name

# Detached from the pane and orphaned to launchd, no terminal: passes.
( nohup sh -c 'cargo nextest run -p farcooler-cli --test removing_asks_for_its_name > /tmp/detached.txt 2>&1' </dev/null >/dev/null 2>&1 & )

# Mac: passes. The swift-test-guard floor message is expected with --filter.
apps/macos/test.sh --filter RealCLIConfirmationTests
```

## Known and out of scope

`script`, `tmux new -d` and `ssh localhost` still read as a person. Each one
gives the command a fresh terminal or an `sshd` that vouches for it, so the
walk can't tell it from a person. This is the limit the module docs already
state.
