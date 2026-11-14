# ov-1138: test processes survive their runner

## Evidence and scope

The ticket records PID 7789, `ov-1114-script-hooks/target/debug/deps/farcooler_daemon-d58f083da146b7ac`,
reparented to launchd and consuming about 75–80% CPU after 15 hours. PID 7789 was killed on November 13 at the owner’s request.
On November 14,
`ps -p 7789 -o pid,ppid,etime,pcpu,command` returned no process, and `sample 7789 1`
reported that it no longer appeared to be running. The original test name and busy-loop
stack cannot be recovered from that PID; this change does not claim to identify them.

The surviving local log `/tmp/ov-1114-tests.log` names that exact binary at line 26,
with 3,056 tests. It has no final summary for the top-level daemon unit-test run.
Its final lines report failures of the two `session_projectors::follow_tests` follower
notification tests after 60 seconds. Other tests, including
`fanout::socket_tests::a_watcher_gets_everything_read_before_the_pipe_ended`, had also
exceeded 60 seconds. These are historical observations, not proof of which test spun.

**The reproduced spawn paths are `lane-gate.py:run_step` and
`tmux-leak-check.py:run`.** Both used an unguarded `subprocess.run`: killing the
Python owner left its runner and test children running. The latter is the wrapper
used around `cargo nextest run` in CI. Its existing post-run cleanup discovers tmux
servers and `farcoolerd`, not daemon unit-test binaries; it also cannot execute at
all after its owner receives SIGKILL.

Nextest's installed source (`nextest-runner-0.126.0/src/runner/unix.rs:32` and
`runner/executor.rs:828`) calls `process_group(0)` for tests. Killing only the runner's
process group therefore misses tests in their own groups. A second red experiment
confirmed this with a stand-in using `os.setpgrp` for its test child.

## Change

`process_guard.run` starts a separate supervisor in a private session. Only the
calling gate holds the write end of a lifetime pipe; the supervisor receives the
read end, and no executed child inherits the writer. EOF detects parent death,
including SIGKILL. Completion of the runner also triggers cleanup.

The supervisor enumerates exact PIDs in its own session, sends TERM, allows a
one-second grace period, then sends KILL to survivors. It rechecks each PID's session
immediately before signaling. The supervisor remains the session leader throughout
cleanup, pinning its ID against reuse. It waits for its direct child. There are no
name-based signals, `pkill`, or `killall` calls.

Both gate steps and the nextest wrapper use this helper, so nested wrappers also
clean up when an outer gate dies. Daemons and tmux servers that create their own
sessions remain under the existing private `TMUX_TMPDIR` leak checks; the checker
self-test now explicitly creates a new session for its detached fake daemon.

This protects these supported spawn paths. A bare test binary or bare nextest run
started outside the wrappers does not gain supervision. This is containment of the
confirmed orphaning path, not a speculative change to an unknown Rust busy-loop.

## Red and green verification

Before wiring the helper, `python3 scripts/process_guard_test.py` failed all four
cancellation cases:

| Case | Evidence from the red run |
| --- | --- |
| Gate receives TERM | runner/test PIDs 45216, 45217 remained alive |
| Gate receives KILL | runner/test PIDs 45367, 45368 remained alive |
| Runner receives KILL | test from runner PID 45509 remained alive |
| Wrapper receives KILL | runner/test PIDs 45746, 45747 remained alive |

`Ran 3 tests in 20.781s — FAILED (failures=4)` (the gate test has two subcases).
After adding separate test process groups, the group-only implementation also
failed all four cancellation cases (`Ran 3 tests in 20.818s`). Session cleanup
passed them. Each cancellation case keeps an unrelated bystander alive and asserts
that it survives; teardown signals only PIDs created by that case.

Final checks:

- `HEAVY_LOCK_HELD=1 python3 scripts/lane-gate.py --self-test`: passes, including the
  six real-process regression tests. These cover TERM/KILL of a gate, KILL of the
  nextest wrapper and runner, nested gate/wrapper cancellation, a successful runner
  leaving a child, and preservation of exit status, output, cwd, and environment.
- `python3 scripts/tmux-leak-check.py --self-test`: passes, including detached
  daemon detection and the existing unrelated-server/bystander checks.
- `python3 scripts/test-leak-check.py --self-test`: passes.
- `python3 scripts/file-size-budget.py` and `git diff --check`: pass.

An additional real-nextest check created a dependency-free Rust test project under
`/tmp`, with a test that published its PID and slept indefinitely. It ran
`tmux-leak-check.py -- cargo nextest run --offline`, waited for the actual test to
start, then SIGKILLed only the wrapper it had started. Result:

```
PASS: SIGKILL of wrapper 50319 left no real nextest test 50360
```

Nextest logged `Cancelling due to signal` and `SIGTERM` for that test. The scratch
project and its private `TMUX_TMPDIR` were removed. Rust production sources are
unchanged; validation targets the changed Python spawn paths and a real Rust test
under nextest rather than rebuilding the unrelated workspace suites.

## Follow-up: runner process-group cancellation

The initial supervisor shared its process group with the runner. Killing the
runner's group therefore killed the supervisor too, bypassing session cleanup
and leaving tests in nextest-style separate groups alive. This is a second
confirmed orphaning path in `process_guard.supervise`.

Added `test_killed_nextest_group_reaps_its_test`, which sends SIGKILL only to the
group created for its own runner. Before the fix it failed in 5.294 seconds:

```
AssertionError: False is not true : wrapper left test processes alive after runner-group: [57864, 57865]
Ran 1 test in 5.294s
FAILED (failures=1)
```

The test's teardown killed its surviving test PID. The supervisor now puts the
runner in its own process group while retaining the supervisor's session. Group
cancellation leaves the supervisor alive to reap separate test groups, with the
same session boundary and unrelated-bystander checks. `preexec_fn=os.setpgrp` is
safe in this standalone, single-threaded supervisor and works on the system
Python 3.9; `Popen(process_group=...)` requires Python 3.11.

Fresh verification: all seven process regressions pass (9.429 seconds), and
`HEAVY_LOCK_HELD=1 python3 scripts/lane-gate.py --self-test` passes with all seven
regressions (9.235 seconds for those regressions). The tmux leak-check and test
leak-check self-tests, file-size budget, and `git diff --check` also pass.
Process tests require running outside the macOS sandbox because it denies `ps`;
the sandboxed attempt failed during process inspection. Only exact child PIDs
started by that attempt were killed during cleanup, and the successful checks
ran with process inspection allowed.

## Follow-up: cancellation of the outer CI leak wrapper

The CI Rust command (`.github/workflows/ci.yml`) nests
`test-leak-check.py -- tmux-leak-check.py -- cargo nextest run`. The outer
`test-leak-check.run` still used `subprocess.call`. Killing it left the tmux
checker alive, holding its own supervisor's lifetime pipe open, so the runner
and test continued. This is another reproduced spawn path; it does not identify
the vanished PID's original Rust test.

`test_killed_outer_leak_wrapper_reaps_its_tests` exercises that exact wrapper
nesting with a runner and a TERM-ignoring test in separate process groups.
Before the fix it failed for both cancellation signals:

```
leak-wrapper left test processes alive after 15: [65680, 65681]
leak-wrapper left test processes alive after 9: [65801, 65803]
Ran 1 test in 10.392s
FAILED (failures=2)
```

Teardown killed only the test-owned surviving PIDs. The outer checker now uses
`process_guard.run` too. Its supervisor ends the inner checker on owner death;
that closes the inner lease and triggers cleanup of the runner and its tests.
The regression also verifies an unrelated bystander survives and removes the
private temporary directories recorded by its own runner.

Fresh validation after this fix:

- `python3 scripts/process_guard_test.py`: eight tests pass in 11.950s.
- `HEAVY_LOCK_HELD=1 python3 scripts/lane-gate.py --self-test`: passes, including
  all eight process regressions (11.735s).
- `python3 scripts/test-leak-check.py --self-test`: passes.
- `python3 scripts/tmux-leak-check.py --self-test`: passes.
- `python3 scripts/file-size-budget.py` and `git diff --check`: pass.

Process checks again ran with sandbox escalation for `ps` access. No production
Rust source changed; the affected Python wrapper suites were run.

## ov-1143: real Cargo and nextest cancellation

A dependency-free Rust fixture publishes its test PID and waits. The regression
runs it under both `cargo test --offline` and `cargo nextest run --offline`,
through both the gate and the outer leak wrapper. It sends TERM and KILL to the
owner after the test starts, and checks that the published test PID exits. The
fixture and cleanup use only processes started by the case. The existing
stand-in tests additionally check runner cleanup, separate process groups,
nested wrappers, and an unrelated bystander.

With each entry point temporarily restored to its unguarded subprocess call,
all four gate cases and all four outer-wrapper cases failed: the real test PID
remained alive for both runners and signals. The test teardown killed those
fixture PIDs. With the supervisor restored, all eight cases pass. No additional
production change was needed: plain Cargo remains in the same supervised session.
The exact test behind PID 7789 remains unrecoverable because the process was
killed and the surviving log records only the binary and incomplete run, not
which test was spinning.

Fix round 1 separates the real Cargo and nextest regressions, so a machine
without nextest still runs the plain Cargo cases and reports the nextest cases
as skipped. The startup assertion also requires a nonempty PID file even if the
owner exits during startup. On this machine both real runners were available;
all ten process tests passed through the gate self-test, as did the two leak
checker self-tests and the file-size budget.
