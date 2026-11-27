# ov-1506 macOS native test failures

Three tests failed in the job "Rust native tests (macOS 2/2)". Both cleanup failures came from one fixture defect. The native hold failure was a race in the test, and the hold behaved correctly. Neither cause came from the ov-1482 module split.

## Causes

### Cleanup tests: a first Build that overran its deadline left a file the retry never committed

The CI panics for these two tests match:

| Test | Run | Duration | Panic |
| --- | --- | ---: | --- |
| `build_cleanup_cancelled_releases_gradle_reports_and_target` | [45891805388 attempt 1](https://github.com/taneliang/farcooler/actions/runs/45891805388/job/140162695172) | 14.222 s | `one: Failed at build attempt 2`, with one Provision and two Build effects settled |
| `build_cleanup_group_inherits_published_folder_list` | [45853106520 attempt 2](https://github.com/taneliang/farcooler/actions/runs/45853106520/job/140042012042) (82d082f1f) | 8.909 s | the same |

Each test failed in its setup, before it reached the cleanup it checks. The ticket never got to Check (Gradle case) or Join (group case).

The old setup ran the landing fixture's Python adapter as the Build. That adapter writes `result-<run id>.txt`, then runs `git add` and `git commit`, all within a 5-second deadline (`coding_tests.rs`, `timeout_ms: 5000`). The cause comes from three facts in the code:

1. **The first Build was killed at its deadline.** On a busy runner, Python plus two Git commands can take longer than 5 seconds. Both historical runs overlapped the slow SwiftPM cleanup test, which took 20.7 s and 17.4 s in those runs. Killing the Build is a safe failure, so the engine retries.
2. **The retry runs in the same checkout without resetting it.** `settle_run_tree` runs only for Run stages (`coding_execute.rs:719`). A retried Build carries on from what its earlier attempt left, which is intended behavior for agents.
3. **The retry cannot clean up after attempt 1.** Attempt 2 has a new run ID, so it writes and commits `result-<run2>.txt` only. Attempt 1's `result-<run1>.txt` stays untracked. `unfinished_build` (`coding_execute.rs:914`) then fails the build with "changes it hadn't committed". Two failed attempts is `Failed at build attempt 2`. If attempt 1 was killed while `git add` held `index.lock`, attempt 2 fails sooner: Git exits with 128.

The timing fits. In the group case the whole test took 8.909 s, which is less than two 5-second deadlines. So only attempt 1 timed out, and attempt 2 failed quickly, as point 3 predicts.

**Replay.** A temporary test used the original fixture (`lander("pass")`, `at_join`) and made only the first Build sleep for 6 s after writing its file. It reproduced the CI signature in 7.45 s:

```
one: Failed at build attempt 2: Some("The build ended its turn with changes it hadn't committed: result-wf1:f0b9.txt. …")
Execute(Provision) settled false Some(Passed(…))
Execute(Build) settled false Some(SafeFailure)          ← attempt 1, exit null (killed at the deadline)
Execute(Build) settled false Some(SafeFailureBecause("…hadn't committed: result-wf1:f0b9.txt…"))
```

An earlier replay with a slow Git clean filter covered the `index.lock` variant. The first Build had no exit code. The second ended with Git exit 128 and Python exit 1, and the run reached `Failed at build attempt 2`.

The CI panic could not show this. The old `until` formatter printed only verb receipts and fetch output (`FETCH []`). It now also prints each item's failure reason, each effect's journal outcome, and the process logs, which is how the replay output above was read.

**Fix.** These tests check cleanup, not the Build process. Setup now builds the candidate with Git plumbing: `commit-tree HEAD^{tree} -p HEAD`, then `update-ref HEAD`. Neither touches the index. Setup then reports a passed Build through `runtime.observe`, and settling that Build triggers the cleanup under test. The configured build command is `/usr/bin/false`, so a sweep that launches it by mistake fails the test. Checks are `/usr/bin/true` or `/usr/bin/false`. Provisioning, group construction, Check, cancellation and cleanup still run through the runtime. Other `building_group` callers use the same setup through `review_built_group`. ([build_cleanup_tests.rs](../../crates/daemon/src/workflow_runtime/build_cleanup_tests.rs), [coding_verbs_tests.rs](../../crates/daemon/src/workflow_runtime/coding_verbs_tests.rs), [fold_trees_tests.rs](../../crates/daemon/src/workflow_runtime/fold_trees_tests.rs))

### Native hold: the hold finished on its own before the test's second `end` got its reply

The CI panic was in [45891805388 attempt 2](https://github.com/taneliang/farcooler/actions/runs/45891805388/job/140168813834) at `native_hold_tests.rs:216`, with `left: Gone`, `right: Said("ended")`.

The first `end` correctly answered `unsettled` while the stray was alive. That `end` had already sent SIGTERM to the group, so the leader had exited. From then on, the hold's main loop in `serve` calls `end(…, Duration::ZERO)` every 250 ms. Once the test kills the stray, two things race:

- If the test's `end` request is served first, it gets `ended`.
- If the main loop's check wins, `finish` removes the socket and exits. The request then finds no socket, or a connection closed with no reply, and `ask_blocking` returns `Gone`.

Pausing 600 ms between killing the stray and asking makes the main loop win every time. With the old assertion, that pause reproduces the CI panic exactly (`left: Gone`, `right: Said("ended")`).

This is a defect in the test, not the hold. `Answer::Gone` is documented as "the hold ended its group and exited". Production treats it as ended: `coding_native_quiesce.rs:53` closes the native session on `Gone`, as on `ended`. The test now accepts either answer and still requires all of the following:

- the socket is gone
- the agent's exit status is `128 + SIGTERM`
- the first `end` answered `unsettled`, kept the socket and left the hold running while the stray worked

The fixture now writes the stray's PID file only after the subshell that started the stray has exited (`mv forked stray`). This replaces the separate `orphaned` marker and its 5-second polling loop.

**ov-1475 comparison.** This is not the ov-1475 defect. In ov-1475, a hold failed to notice that its leader had exited. Here the hold noticed the exit and finished on its own as soon as the stray was gone, which is the opposite. The hold's refusal to settle while a live stray existed was correct, and that is what the first `end` checks.

### ov-1482

ov-1482 (a901cacee) changed none of `native_hold*.rs`, `build_cleanup*.rs`, `coding_execute.rs`, `coding_tests.rs`, `coding_verbs_tests.rs` or `effect_runner.rs`. The group-folder failure also happened on 82d082f1f, which is before the split. The split did not cause either failure.

## Verification on macOS

The machine was macOS 27.2 arm64 with 10 logical CPUs, already loaded (load average 15–20) by other work. The stress loop added 4 `yes` burners and ran 12 test processes at once. Each run was one exact test in a fresh process of the daemon unit-test binary, built under `$FARCOOLER_SCRATCH/target`.

```python
import os, subprocess, sys, time
from collections import Counter
from concurrent.futures import ThreadPoolExecutor
from itertools import product
binary = sys.argv[1]
tests = (
    'workflow_runtime::build_cleanup_tests::build_cleanup_cancelled_releases_gradle_reports_and_target',
    'workflow_runtime::build_cleanup_tests::build_cleanup_group_inherits_published_folder_list',
    'native_hold::tests::a_stray_whose_parent_exited_first_is_still_accounted_for',
)
env = dict(os.environ, FARCOOLER_TEST_SHELL='/bin/sh')
def run(case):
    r, t = case
    res = subprocess.run([binary, t, '--exact', '--nocapture'], stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, env=env)
    return r, t, res
passed, failed = Counter(), Counter()
load = [subprocess.Popen(['/usr/bin/yes'], stdout=subprocess.DEVNULL) for _ in range(4)]
start = time.time()
try:
    with ThreadPoolExecutor(max_workers=12) as pool:
        for r, t, res in pool.map(run, product(range(1, 101), tests)):
            ok = res.returncode == 0 and '1 passed' in res.stdout
            (passed if ok else failed)[t] += 1
            print(f'{"PASS" if ok else "FAIL"} {r}/100 {t.split("::")[-1]}', flush=True)
            if not ok: print(res.stdout[-3000:], flush=True)
finally:
    for c in load: c.terminate()
    for c in load: c.wait()
print(f'elapsed {time.time()-start:.0f}s; load {os.getloadavg()}')
for t in tests: print(f'{t}: {passed[t]}/100 passed, {failed[t]} failed')
sys.exit(0 if not failed else 1)
```

Result, after rebasing onto 29c41d4de:

```
PASS 100/100 a_stray_whose_parent_exited_first_is_still_accounted_for
elapsed 103s; load (30.60, 24.56, 21.26)
workflow_runtime::build_cleanup_tests::build_cleanup_cancelled_releases_gradle_reports_and_target: 100/100 passed, 0 failed
workflow_runtime::build_cleanup_tests::build_cleanup_group_inherits_published_folder_list: 100/100 passed, 0 failed
native_hold::tests::a_stray_whose_parent_exited_first_is_still_accounted_for: 100/100 passed, 0 failed
```

There were 300 PASS lines and no FAIL lines.

### Each test still fails without its behavior

For each test, a temporary change to production code removed the behavior it checks. All three changes went into one build, and each test failed at its behavior assertion. The changes were then reverted.

| Test | Change to production code | Failure |
| --- | --- | --- |
| Gradle cancellation | `build_cleanup.rs`: skip folders under `apps/android` | `build_cleanup_tests.rs:598`, the assertion that `apps/android/build` is removed in phase `Cancelled` |
| Published group folders | `definition.rs`: a group definition gets `build_folders: None` instead of the published list | `build_cleanup_tests.rs:464`: `assertion failed: tree.join("target/output").exists()`. The defaults deleted output the published list leaves alone. |
| Orphaned stray | `native_hold.rs`: `end` settles once the group is empty, whether or not a stray is alive | `native_hold_tests.rs:210`: `left: Said("ended")`, `right: Said("unsettled")` |
