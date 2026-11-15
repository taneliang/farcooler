- `TMPDIR=/tmp cargo test -p farcooler-workflow`: 196 passed, including 65 group
  regressions. These cover targeted ejection, unchanged half splitting when no
  member is named, diagnostics on singleton return, invalid attribution,
  empty-group resource release, deterministic replay, and cancellation/requeue.
- The new cancellation test was run with only the cancellation fix removed:
  it failed with `left: Failed`, `right: WaitingJoin`; restoring the fix passes.
- The real Git/runtime test
  `coding_gate_tests::nov14_gate_output_ejects_the_planted_breaker_and_lands_the_healthy_batch`
  passes. Its gate emits the unchanged Nov 14 receipt bytes and exits nonzero.
  One member commits a copy-ratchet breaker; two healthy members stay together
  and land. Recovery returns the same attribution and diagnostic brief.
- Parser regressions pass for changed-file-header exclusion, whole relative
  paths, absolute traceback paths, `./` paths and bounded UTF-8 briefs.
- `cmp` confirms the fixture is byte-identical to the original canary output.
- `python3 scripts/file-size-budget.py`, `python3 scripts/swallow-lint.py`, and
  `python3 scripts/doc-comment-check.py`: pass.
- Source diff whitespace check passes. The unchanged receipt intentionally
  preserves eight whitespace-only lines from lane-gate's historical output.

Initial sandboxed daemon test runs failed before reaching the feature because
Git invocation in their temporary repositories was refused. The affected runtime
suite was rerun with normal local process access.

`TMPDIR=/tmp cargo clippy -p farcooler-workflow -p farcooler-daemon --all-targets -- -D warnings`
passes.

The existing named-tree gate-recovery test timed out in the parallel runtime
suite, then passed in an isolated rerun (6.51 seconds). Two representative
broader-suite failures also reproduced in isolation: scheduled cleanup times out
at `build_cleanup_tests.rs:649`, and the native-agent fixture settles at `Build`
with `Phase::Uncertain`, before any landing group or gate exists. These paths do
not exercise the new gate-attribution or dropped-group handling.

The broad `workflow_runtime::` run was stopped after over nine minutes: all other
cases had reported, but
`concurrency_tests::workflow_get_shows_an_effect_waiting_for_a_place` remained
hung (its watcher awaits an inbox reply after the sweep). It reported 45 failures:
26 native-agent tests, 16 agent-review gate tests, two scheduled-cleanup tests,
and the named-tree gate-recovery timeout that passed in isolation. This is not a
green full-runtime run. The native/agent-review failures end before the new batch
gate handling; they are retained as validation limitations rather than changed
under this ticket.

All 47 landing-verb cases passed across the parallel run (46) and isolated
recovery rerun (1), and the two new `coding_gate_tests` passed. All landing-template
cases reported passing in the broad run. These include the existing anonymous
batch-split and singleton-gate rejection regressions.

The commit hook initially selected an incompatible Intel Python from
`/usr/local/bin/python3` (`Bad CPU type in executable`). Committing with
`/opt/homebrew/opt/python@3.14/libexec/bin` first on PATH runs the same hook with
the installed native Python, without bypassing it or changing system settings.
The native-agent failure also reproduces with `/usr/bin` ahead of `/usr/local/bin`;
its fixture explicitly invokes `/usr/bin/python3`, independently of the hook.
