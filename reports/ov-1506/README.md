# ov-1506 macOS native test failures

## Historical CI evidence

| Failure | What the log establishes |
| --- | --- |
| Cancelled Gradle cleanup | The test spent 14.20 seconds and ended with `one: Failed at build attempt 2`. Provision and both Build effects settled. The fixture never reached Check, where it creates the output and cancels the item. [Run 45891805388 attempt 1](https://github.com/taneliang/farcooler/actions/runs/45891805388/job/140162695172) |
| Published group folders | The test spent 8.89 seconds and reported the same three settled effects and terminal state. The member never reached Join to create the group. The elapsed time allows at most one five-second command timeout. [Run 45853106520 attempt 2](https://github.com/taneliang/farcooler/actions/runs/45853106520/job/140042012042) |
| Orphaned stray | The panic reports `Gone` where the second `end` expected `Said("ended")`. The first request had returned `unsettled` while the stray worked. After the test killed the stray, the hold's own exit loop could remove the socket before the client got a second reply. The test accepts either terminal response and checks socket removal and the agent's exit status. It publishes the stray's PID after the subshell that started it has exited. [Run 45891805388 attempt 2](https://github.com/taneliang/farcooler/actions/runs/45891805388/job/140168813834) |

The historical cleanup logs and uploaded inventories contain no per-build exit codes or adapter output. They establish a failure in the shared build prerequisite. The exact historical command failure for either build test remains unverified.

## Reproducing the build failure

The original cleanup fixture used the landing test's Python adapter. It created a source file, staged it with `git add`, and then committed it, all under the same five-second command deadline. A safe failure increments the attempt and issues a new effect, while the checkout and its Git index stay attached to the same item. [Fixture adapter](../../crates/daemon/src/workflow_runtime/coding_tests.rs) · [Process supervision](../../crates/daemon/src/effect_runner.rs) · [Retry transition](../../crates/workflow/src/reducer.rs)

To test interruption while Git held the index lock, a scratch driver set `result-*.txt filter=ov1506` in each fixture's private `.git/info/attributes` after the fixture wrote its ignore list. It set `clean = /bin/sleep 60` in the repository's `[filter "ov1506"]` section, so the wait happened inside the real `git add` process group. The original fixture failed both tests with two different attempt results:

| Test | First build | Second build | Test time |
| --- | --- | --- | ---: |
| Cancelled Gradle cleanup | Deadline killed the process group while the index was locked. Receipt exit was null. | `git add` found the stale `index.lock` and exited 128. Python recorded exit 1. | 11.11 s |
| Published group folders | Deadline killed the process group while the index was locked. Receipt exit was null. | `git add` found the stale `index.lock` and exited 128. Python recorded exit 1. | 9.19 s |

Both reached `Failed at build attempt 2`. The group replay took 9.19 seconds, close to the historical 8.89 seconds and below two full build deadlines. This establishes that a slow first build can make the second fail immediately even after the process group is quiescent. The original fixture also passed 106 runs per case without the filter, including runs beside SwiftPM builds and runs with up to 64 concurrent test processes. There is no CI evidence that a stale lock was present on either runner.

The cleanup fixtures now make a fresh candidate with one invocation of the resolved Git executable using `commit --allow-empty`. This removes the Python and staging prerequisites and keeps the original five-second deadline. The group still provisions and builds its own candidate, and the Gradle tests still pass or fail a real supervised check or cancel before it starts. If the runtime fails to reach its expected state, the test prints each item's retained process exits and output tails.

## Related tickets

The ov-1482 split moved the background-task and native-receipt code. The two cleanup tests use configured command builds and the hold test talks to the hold binary directly. The group failure predates the split.

The ov-1475 holds had observed their leader's exit and kept their sockets while detached descendants were alive. The orphaned-stray test likewise sees the hold account for the stray while it works. Once the stray dies, the hold removes its socket and returns the agent's status. Attempt 2 was a race between that successful exit and the client's assertion. See `git show ac8644bab:reports/ov-1475/README.md`.

## Verification on macOS

The same clean-filter replay passed with the new fixture. The Gradle cancellation test took 1.68 seconds and the group folder test took 2.54 seconds.

The stress loop ran on macOS 27.2.1 arm64 with 10 logical CPUs. Each invocation selected one test in a fresh process, as Nextest does. Cargo and test build output went under `$FARCOOLER_SCRATCH`. Set `binary` to the unit-test executable from `cargo test -p farcooler-daemon --lib --no-run` after building the daemon with `cargo build -p farcooler-daemon --bins`.

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
with ThreadPoolExecutor(max_workers=12) as pool:
    for round_number, test, result in pool.map(run, product(range(1, 101), tests)):
        ok = result.returncode == 0 and '1 passed' in result.stdout
        passed[test] += ok
        print(f'{"PASS" if ok else "FAIL"} {round_number}/100 {test}')
        if not ok:
            print(result.stdout)
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

The six-worker Nextest selection ran 129 tests covering native hold, the process supervisor, cleanup, fold trees, coding verbs, and the command-build timeout regression. It passed 128. The existing pane-echo test measured 135.06 ms against a 100 ms limit before reaching the simulated lsof probe. In isolation it passed at 18.93 ms and observed overlap with both the probe and the folder walk.

The daemon layer and file-size checks passed.
