# ov-1358 — gate-safe builders and pane evidence

Started at origin/main `22ef4a6d0e84d9846be8f332b145a9702686e17a` and cherry-picked ov-1357 commits `94a9c0342`, `06f68d27a`, and `f3f8386ec`, in order. The historical cause of the six Nov 19 losses remains unknown; their sessions did not retain the needed evidence.

## Verification

- Built the daemon with `TMPDIR=/tmp CARGO_PROFILE_DEV_DEBUG=0 CARGO_INCREMENTAL=0 /usr/bin/python3 scripts/heavy --load-limit 30 -- cargo build -p farcooler-daemon`.
- The following focused ov-1357 daemon and tmux selection passed: **38 tests passed**. It includes the failed-client builder regression, the sustained real-tmux gate pause, native quiescence, yielding, and pane probe tests. The additional `lost_pane_evidence_records_dead_tag_and_client_failure` test checks the actual log for a dead pane's exit facts, a missing tag, and a client failure.

```sh
TMPDIR=/tmp CARGO_PROFILE_DEV_DEBUG=0 CARGO_INCREMENTAL=0 /usr/bin/python3 scripts/heavy --light --load-limit 30 -- \
  cargo nextest run -p farcooler-daemon -p farcooler-tmux --lib -E \
  'test(pane_life::) | test(pane_read::tests::) | test(yielding::tests::) | test(workflow_runtime::coding_native::unread_tests::) | test(workflow_runtime::native_control::tests::) | test(workflow_runtime::coding_native::quiesce_order_tests::) | test(workflow_coding_native_real_pty_normalized_hooks_and_quiescence) | test(a_running_builder_is_paused_while_a_landing_gate_runs_and_resumes_after)'
```

The first sandboxed test run failed where process spawning and signals were denied; the successful run used access to the private test processes and tmux servers.

- `TMPDIR=/tmp PATH=/usr/bin:/bin:/opt/homebrew/bin:$HOME/.cargo/bin sh scripts/lint-gate.sh`: passed.
- `/usr/bin/python3 scripts/file-size-budget.py`: passed.
- `git diff --check`: passed.

`scripts/landing-gate` was not run.
