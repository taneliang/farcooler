# ov-1506 macOS native test failures

## Archived CI evidence

| Failure | Evidence |
| --- | --- |
| Cancelled Gradle cleanup | At 20:56:14 UTC, Nextest reported 14.222 seconds and `one: Failed at build attempt 2`. The outbox listed one Provision and two Build effects as settled and inactive. `gradle_reports_cleanup` was still waiting for Check, so it had yet to seed the Gradle output or issue Cancel. [Run 45891805388 attempt 1](https://github.com/taneliang/farcooler/actions/runs/45891805388/job/140162695172) |
| Published group folders | At 16:34:13 UTC, Nextest reported 8.909 seconds and the same two Build settlements for `one`. `building_group` was still waiting for the member to reach Join, so it had yet to queue the group. Two five-second command deadlines exceed the entire test duration. [Run 45853106520 attempt 2](https://github.com/taneliang/farcooler/actions/runs/45853106520/job/140042012042) |
| Orphaned stray | The first `end` returned `unsettled` while the stray was alive. At the second `end`, after the test killed the stray, the panic reported `Gone` instead of `Said("ended")`. The hold's autonomous exit loop can reap the leader and unlink the socket before the second client gets its reply. The fixed test accepts either terminal response and checks both socket removal and the agent's exit status. The fixture publishes the stray's PID after its parent subshell exits. [Run 45891805388 attempt 2](https://github.com/taneliang/farcooler/actions/runs/45891805388/job/140168813834) |

Both cleanup failures exhausted the landing fixture's two Build attempts. The adapter ran Python, created a file, then ran `git add` and `git commit` within a five-second deadline. A retry reused the checkout and received a new run ID, which the fixture used in the filename. The Build can settle as a safe failure after a timeout or nonzero exit, or after a zero exit with a dirty tree or no new commit. The test runtime supplies a fixed free-space value, which rules out its low-disk admission check. [Original fixture](../../crates/daemon/src/workflow_runtime/coding_tests.rs) · [Process supervision](../../crates/daemon/src/effect_runner.rs) · [Build settlement](../../crates/daemon/src/workflow_runtime/coding_execute.rs) · [Retry transition](../../crates/workflow/src/reducer.rs)

The archived panic printed the state and dispatch statuses with `FETCH []`. The formatter only read verb receipts and fetch output, so the Build outcomes, process exits and adapter output were absent. The failed test's temporary directory also disappeared before the leak check. Both runs' artifact inventories contain prebuilt test archives and test lists. Establishing the historical process-level trigger would require the Build receipts or a matching reproduction.

## Cleanup fixture replay and repair

The original fixture passed 30/30 per case with 16 simultaneous test processes and 12 CPU workers on arm64 macOS. A second round passed 20/20 per case with 16 test processes at nice +5 and 10 CPU workers. A watcher observed 60 Build receipts during that round, all with exit 0 and an empty output tail. These replays used the original adapter without a filter or a deliberately held index lock.

Both historical cleanup failures overlapped the same SwiftPM cleanup test. That test took 20.741 seconds in the Gradle failure run and 17.446 seconds in the group failure run. It took 8.499 seconds in attempt 2, when both cleanup cases passed. To test the overlap, a fresh macOS replay used the original cleanup fixtures with the expanded failure output, 12 test workers and 10 CPU-load workers. It passed 30/30 per cleanup case and 8/8 SwiftPM invocations.

A prior controlled replay used a slow Git clean filter to stop the original `git add` while it held the index lock. In both cases the first build had a null exit, the second failed with Git exit 128 and Python exit 1, and the run reached `Failed at build attempt 2`. This shows that supervised interruption can contaminate the same checkout used for the retry. The CI logs contain no evidence that this happened on the hosted runners.

The replacement `git commit --allow-empty` still needs the index lock. In a scratch Git repository, an existing `index.lock` blocked that command. `git commit-tree HEAD^{tree} -p HEAD` followed by `git update-ref HEAD <candidate>` created the candidate while leaving the lock alone. The cleanup fixture now uses those two commands in setup and reports the successful Build result through the runtime. This removes the Python adapter, the five-second build deadline and the index write from the prerequisite. Worktree provisioning, group construction, Check execution, cancellation, and cleanup still use the runtime. The landing fixture's panic now prints the retained Build outcomes alongside exits and output tails. [Fixture and assertions](../../crates/daemon/src/workflow_runtime/build_cleanup_tests.rs) · [Diagnostics](../../crates/daemon/src/workflow_runtime/coding_verbs_tests.rs)

## Related tickets

The ov-1482 split moved the background-task code and native receipt types without changing the code that runs the command Build or the native hold. The published group failure already occurred on 82d082f1f, before that split.

The ov-1475 holds observed their leaders' exit and retained their sockets while detached descendants were alive. The failing hold test also saw its hold account for a live stray. Here the test had already killed the stray, and the hold was able to finish before answering the client. See `git show ac8644bab:reports/ov-1475/README.md`.

## Verification on macOS

The fresh stress loop used 12 test workers and four CPU-load workers on macOS 27.2.1 arm64 with 10 logical CPUs. Each invocation selected one exact test in a fresh process. Cargo and test build output went under `$FARCOOLER_SCRATCH`. Set `binary` to the unit-test executable from `cargo test -p farcooler-daemon --lib --no-run` after building the daemon with `cargo build -p farcooler-daemon --bins`. Use `FARCOOLER_TEST_SHELL=/bin/sh` when running the binary directly.

```python
from collections import Counter
from concurrent.futures import ThreadPoolExecutor
from itertools import product
import subprocess

tests = (
    'workflow_runtime::build_cleanup_tests::build_cleanup_cancelled_releases_gradle_reports_and_target',
    'workflow_runtime::build_cleanup_tests::build_cleanup_group_inherits_published_folder_list',
    'native_hold::tests::a_stray_whose_parent_exited_first_is_still_accounted_for',
)

def run(case):
    round_number, test = case
    result = subprocess.run([binary, test, '--exact', '--nocapture'],
                            stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
    return round_number, test, result

passed = Counter()
load = []
try:
    for _ in range(4):
        load.append(subprocess.Popen(['/usr/bin/yes'], stdout=subprocess.DEVNULL))
    with ThreadPoolExecutor(max_workers=12) as pool:
        for round_number, test, result in pool.map(run, product(range(1, 101), tests)):
            ok = result.returncode == 0 and '1 passed' in result.stdout
            passed[test] += ok
            print(f'{"PASS" if ok else "FAIL"} {round_number}/100 {test}')
            if not ok:
                print(result.stdout)
finally:
    for child in load:
        child.terminate()
    for child in load:
        child.wait()
assert all(passed[test] == 100 for test in tests), passed
```

| Test | Consecutive passes |
| --- | ---: |
| `build_cleanup_cancelled_releases_gradle_reports_and_target` | 100/100 |
| `build_cleanup_group_inherits_published_folder_list` | 100/100 |
| `a_stray_whose_parent_exited_first_is_still_accounted_for` | 100/100 |

Each ticket test failed at its behavior assertion with a temporary production-code mutation:

| Test | Mutation and observed failure |
| --- | --- |
| Gradle cancellation | Skipping deletion of `apps/android/` build folders caused the assertion for `apps/android/build` in phase `Cancelled` to fail. |
| Published group folders | Dropping `build_folders` when constructing the group definition used the defaults and deleted `target/output`. The test failed at the assertion that this unselected output remained. |
| Orphaned stray | Allowing `native_hold::end` to settle when the process group was empty while a stray worked returned `Said("ended")` at the first request. The test expected `Said("unsettled")`. |

The six-worker run of the affected daemon modules passed 123 of 125 tests on the first pass. The heavy-lock case resolved an Intel-only Python from PATH and passed in isolation with ARM-native Python first on PATH. The pane-echo test measured 150.26 ms against its 100 ms latency limit during the parallel run. In isolation it passed at 86.63 ms and observed overlap with both the simulated lsof probe and the folder walk.

After rebuilding the restored production code, a four-worker run with the CI Nextest profile passed all 125 selected tests in 194.50 seconds. The selection covered cleanup, fold trees, coding verbs, command-build timeout and restart, the process supervisor, and native hold. The daemon layer and file-size checks passed.
