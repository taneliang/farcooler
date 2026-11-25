# ov-1457 self-test process cleanup

## Failing check on main

Before changing either script, I reproduced both leaks on `origin/main`
(`ab1c6074dca16117dc0c49ecf5cfa517d8e555d2`), using copies of the scripts
from that ref. I paused the process-guard self-test using its existing Cargo
fixture readiness hook. For tmux, I paused at the first lookup of the
bystander's private socket and recorded the server and its pane from the
process table. I sent SIGKILL to each self-test and checked for live members
of the exact fixture process groups.

| Self-test | Exit status | Live fixture process groups after exit |
| --- | ---: | --- |
| tmux leak check | -9 | 82193, 82202 |
| Process guard | -9 | 82336 |

The absence check failed for both. The baseline harness ended its observed
fixture groups by group ID and confirmed those groups were empty.

## Cleanup

Both entry points now put the self-test worker under a separate supervisor
with a pipe held by the caller. A normal exit lets the supervisor finish
cleanup. SIGTERM, SIGHUP, SIGINT, and SIGQUIT close the pipe and wait for the
supervisor. When SIGKILL closes the pipe, the supervisor finishes cleanup
independently. The process-guard test's nested supervisors also receive EOF
when their respective fixture owners die.

The tmux self-test starts foreground servers with real Unix sockets and no
pane commands. Its fakes share the supervised test session. The test still
exercises stale and live sockets, daemon detection, the private-directory
boundary, and both report and kill behavior. Test teardown uses child PIDs
and the supervised session's membership.

## Ten consecutive rounds

On arm64 macOS, each round ran `tmux-leak-check.py --self-test` followed by
`process_guard_test.py` in the foreground at nice +10. Cargo used one build
job, with its target directory under `$FARCOOLER_SCRATCH`. The guard suite ran
all ten tests each time, including the real Cargo and nextest fixtures.

The host had **91 live orphaned fixtures before and 91 after**, with the
same PID set. The read-only count used parent PID 1 and fixture commands
containing `/tmp/fc-` (including `/private/tmp/fc-`),
`tmux-leak-check.py`, or `guard_fixture`. It excluded zombies.

| Round | tmux self-test | Process-guard self-test |
| ---: | --- | --- |
| 01 | PASS (21.4 s) | PASS (23.6 s) |
| 02 | PASS (27.4 s) | PASS (22.1 s) |
| 03 | PASS (21.1 s) | PASS (19.5 s) |
| 04 | PASS (25.3 s) | PASS (23.7 s) |
| 05 | PASS (24.6 s) | PASS (23.1 s) |
| 06 | PASS (24.5 s) | PASS (20.0 s) |
| 07 | PASS (18.1 s) | PASS (19.7 s) |
| 08 | PASS (20.3 s) | PASS (23.5 s) |
| 09 | PASS (16.9 s) | PASS (19.5 s) |
| 10 | PASS (18.2 s) | PASS (21.6 s) |

## Exit paths

`FixtureProcessGroups` in `scripts/lane-gate-lifetime-test.py` passed for
both entry points. Each case observed a live fixture before allowing normal
completion, injecting a failure, or signaling the caller. It checked the
exit status and waited for every observed process group to have zero live
members.

| Exit path | tmux self-test | Process-guard self-test |
| --- | --- | --- |
| Normal completion | PASS | PASS |
| Injected failure with live fixtures | PASS | PASS |
| SIGTERM | PASS | PASS |
| SIGKILL | PASS | PASS |

The existing lane-gate lifetime cases, `test-leak-check.py --self-test`,
`swift-test-guard-test.sh`, and `macos-test-wrapper-test.sh` also passed.
