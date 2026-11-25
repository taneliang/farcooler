# ov-1455 · Scheduling pause and resume test

On November 24 SGT, the macOS Rust job failed at `all five provisions executed`. All five tickets had started under `Budget::Unlimited` with a recorded sample of 5,636 milli-load. Ticket `c` was still at Provision in `Uncertain`. The fixed-zero budget and unlimited budget both ignore the sample when deciding admission. CI logged the workflow state and omitted the underlying Git diagnostics. [CI run 45263849183, attempt 3, macOS Rust job](https://github.com/taneliang/farcooler/actions/runs/45263849183/job/138057089419)

## Forced sample before the fix

I temporarily made the scheduler's host sampler return `Some(1_000_000)` under `OV_1455_FORCE_HIGH_LOAD` and added an assertion for the intended fake sample, `Some(100)`, after the original assertions. The exact test ran in the foreground at nice +20 and failed before the per-runtime override was in place. The original pause, admission and provisioning assertions passed in that run.

```text
assertion `left == right` failed: the automatic start recorded the test's load sample
  left: Some(1000000)
 right: Some(100)
test result: FAILED. 0 passed; 1 failed
```

With only the original assertions enabled, the exact test passed 20 consecutive foreground runs with the same forced sample.

## Change

The test sets a per-runtime fake before either runtime pumps and checks the recorded sample on each automatic start. It still verifies that a published fixed-zero budget pauses the five tickets and that an unlimited budget resumes them after reopening the runtime. The fixture provisions one worktree per sweep to avoid the concurrent Git metadata writes seen in prior provisioning failures. The next stage waits for approval so each of the five tickets stops after Provision. The override is compiled only in tests. [Prior provisioning race evidence](../ov-1355/README.md)

## Verification

- The exact test passed **100 consecutive foreground runs** in 224.9 seconds on ARM64 macOS. A single `/usr/bin/yes` worker ran throughout, with stdout directed to `/dev/null`. The test driver and worker ran at nice +20. The driver ran one test process at a time with one test thread and stopped on any failure. Sampled worker CPU use ranged from 54.4% to 97.6% of one core. Sampled host one-minute load ranged from 15.53 to 21.59.
- The daemon library's `workflow_runtime::scheduling::tests::` filter passed all 18 tests at nice +20 with one test thread.
- `workflow_runtime::coding_landing_head_tests::a_held_queue_explains_its_missing_target_notifies_once_and_retries` passed at nice +20. This test also calls `schedule_ready`.
