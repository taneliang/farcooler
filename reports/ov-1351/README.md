# ov-1351 returned landing gate

The retained landing receipts for candidate `9d191999da37acf79e0449d40c8a39fc82fda38e`
show disk admission refusals, before any gate checks ran. All three attempts
exited with code 75:

| Process receipt | Free space | Required space |
| --- | --- | --- |
| `wf1:f8635cf32aaa979345453e84ac1b6ab2708b93bf4f9115932bb437a8450fd59f` | 30.96 GB | 61.00 GB |
| `wf1:86502cc46e5596ba878f0b61a010e3221ba3df384a5e9db07ce0ab6919c79ebd` | 30.96 GB | 61.00 GB |
| `wf1:f9436e3dfce92cc2d2271e0ad65c797fc75036ab4f15040ca503604da694ddca` | 30.69 GB | 61.00 GB |

These process receipts and their `.output` logs live in the installed Canary
engine's `workflow/receipts/` directory. The required space is the gate's 36 GB
budget plus the daemon's 25 GB reserve. This is an environment blocker, not
evidence of a failed Rust assertion. Retrying this branch requires sufficient
free space on the engine volume; changing admission thresholds would conceal
the blocker. The new cleanup code must be running in the engine to reclaim
finished groups automatically.

The returned task did not run `scripts/landing-gate`, modify the installed
engine, or remove other runs' trees. Existing implementation and regression
coverage remain in the preceding ov-1351 commits.

Fresh targeted verification also exposed two load-sensitive tests:
`build_cleanup_scheduled_retry_when_runtime_is_idle` and
`build_cleanup_scheduled_retry_while_another_effect_runs`. Both timed out waiting
for the first retryable cleanup result under parallel load, then passed alone.
Their five-second setup and three-second release deadlines included real Git
and lsof calls. Both observation deadlines now allow 30 seconds, and the
companion build sleeps and times out at 120 seconds. The assertions still
require cleanup before the companion effect ends, accurate byte accounting,
and bounded retry wakeups. Production cleanup behavior is unchanged.

Validation after the fixture repair:

- 98 targeted daemon tests passed in parallel (71.669 seconds), covering
  `workflow_runtime::{build_cleanup_tests,fold_trees_tests,coding_verbs_tests,builder_resources}`.
- `scripts/lint-gate.sh` passed using system Python on PATH.
- `python3 scripts/file-size-budget.py` and `git diff --check` passed.

Native test execution was necessary: sandboxed Git fixture setup failed before
cleanup ran. `TMPDIR=/tmp` also avoids the sandbox's unwritable default compiler
temporary directory. The installed engine's landing disk-admission blocker
remains separate from these passing targeted tests.
