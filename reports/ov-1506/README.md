# ov-1506 macOS native test failures

## CI failures

- **Cancelled Gradle cleanup.** Attempt 1 failed in `until` with `one: Failed at build attempt 2`. The outbox showed a settled Provision and two settled Builds. The fixture never reached Check, where the test seeds the build output and cancels the ticket. [macOS 2/2, run 45891805388 attempt 1](https://github.com/taneliang/farcooler/actions/runs/45891805388/job/140162695172)
- **Published group folders.** The earlier run at `82d082f1f` failed with the same outbox and terminal state while preparing the member ticket. It never reached the join that creates the group. [macOS 2/2, run 45853106520 attempt 2](https://github.com/taneliang/farcooler/actions/runs/45853106520/job/140042012042)
- **Orphaned stray.** Attempt 2 failed at the second `end` in `refuses_until_the_stray_is_gone`. The test got `Gone` and expected `Said("ended")`. The first request had already returned `unsettled` while the stray was working. After the test killed the stray, the hold's own exit loop could win the race with the second client request. Both paths remove the socket once the group and its stray have exited. The test now accepts either response and checks socket removal and the agent's exit status. The fixture publishes the stray's PID after the subshell that started it has exited. [macOS 2/2, run 45891805388 attempt 2](https://github.com/taneliang/farcooler/actions/runs/45891805388/job/140168813834)

## Build fixture reproduction

The cleanup tests shared a five-second wall-clock deadline for the Python adapter and its Git subprocesses. If the deadline wins before the supervisor observes the adapter exit, `effect_runner::supervise` ends the group and `run_coding` records a process fact with a null exit code. The workflow counts that as a failed Build attempt.

Locally, the original deadline passed 12 rounds with eight concurrent test processes and 24 rounds with 24. Inserting a six-second delay immediately after the fixture's Git commit reproduced the exact CI state and outbox in both tests. The local receipts showed null exit codes on both attempts. Both tests passed with the same delay and a 30-second build deadline. The historical logs lack process receipts and captured adapter output, so the exact exit reason for each historical attempt is unverified. In particular, the earlier group test's 8.89-second runtime precludes two full five-second waits.

Only the cleanup fixtures opt into the longer build deadline. Command timeout coverage still uses the dedicated short-deadline fixture.

## Related changes

The ov-1482 module split moved background task and native receipt definitions. The cleanup fixture uses a configured command build, and the native hold test talks directly to the hold binary. The earlier group failure predates the split.

The ov-1475 holds kept their sockets while detached descendants were still alive, although the hold had already observed the zombie leaders. In this test the hold observed the exit, guarded against claiming completion while the stray worked, and removed the socket promptly once the stray died. The earlier investigation is in `git show ac8644bab:reports/ov-1475/README.md`.

## Verification

The foreground stress driver ran each case in its own process, as Nextest does. It queued 100 rounds across 12 workers on macOS 27.2.1 arm64 with 10 logical CPUs. Each invocation selected exactly one test and checked that it ran and passed. The daemon and unit tests were built with `CARGO_TARGET_DIR="$FARCOOLER_SCRATCH/target"`. In the loop, `binary` is the executable printed by `cargo test -p farcooler-daemon --lib --no-run`.

```python
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

with ThreadPoolExecutor(max_workers=12) as pool:
    for round_number, test, result in pool.map(run, product(range(1, 101), tests)):
        assert result.returncode == 0 and '1 passed' in result.stdout, result.stdout
        print(f'PASS {round_number}/100 {test}')
```

| Test | Consecutive passes |
| --- | ---: |
| `build_cleanup_cancelled_releases_gradle_reports_and_target` | 100/100 |
| `build_cleanup_group_inherits_published_folder_list` | 100/100 |
| `a_stray_whose_parent_exited_first_is_still_accounted_for` | 100/100 |

Two temporary production-code changes verified what the tests protect:

- Skipping `remove_dir_all` while still reporting the measured bytes failed the Gradle test at `target, phase=Cancelled` and the group test at the assertion that `custom-output` was removed.
- Ignoring live strays in `native_hold::end` failed the native hold test at its first `end`. It got `Said("ended")` while the stray still worked and expected `Said("unsettled")`.

The broader six-worker Nextest selection covered native hold, build cleanup, fold-tree fixtures, coding verbs and the command-build timeout regression. All 121 selected tests passed across two runs. The first run's legacy-holder test selected an x86_64 Python from `/usr/local/bin` and failed to launch. It passed with the system Python first on `PATH`. The second run's existing pane-echo latency test measured 101.48 ms against its 100 ms limit. It had passed on the first run and uses a separate service fixture. The three ticket tests passed in both runs.

The daemon layer and file-size checks passed.
