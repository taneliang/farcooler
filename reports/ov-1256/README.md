# ov-1256: start Rust shards earlier

Linux's four Rust shards now need only Changes and their archive, with
`max-parallel: 4`. Android produces no input they consume. Its old dependency
was a capacity reservation added during ov-1239 integration, not a build/test
dependency. Relay, Site and Tunnel helper now yield their short Linux checks
until Rust shards and Android finish, retaining the 20-job repository budget.
Their `!cancelled()` conditions run them even when those capacity predecessors
fail or skip; existing path selection remains intact.

The native Mac archive now starts immediately after Changes. The existing Rust
platform-check matrix yields its initial Mac slot to that archive. Keeping that
matrix together preserves both required check names and the local gate's full
platform-check plan; the Linux member also waits, but its approximately
11-minute run should still finish around minute 23. The two native shards run
together after the archive and both Swift jobs complete. Swift is a capacity
dependency only: its failure or skip cannot suppress native tests.

## Mac capacity and expected timeline

The repository reserves one Mac for shared background workflows (Canary,
Nightly, release, etc.), leaving four for landing CI. iOS and iOS UI smoke are
one sequential chain and can never consume two slots together. This table is
an upper bound for every completion order, including when iOS smoke is slow;
completed jobs only reduce its counts.

| Stage | CI Mac jobs eligible together | CI Macs | Background | Total |
| --- | --- | ---: | ---: | ---: |
| Changes/plan | none | 0 | 1 | 1 |
| Initial builds | native Rust archive, Swift app build, Swift tests, iOS chain | 4 | 1 | 5 |
| Native archive complete | Rust platform check, Swift app build, Swift tests, iOS chain | 4 | 1 | 5 |
| One Swift job complete | Rust platform check, remaining Swift job, iOS chain | 3 | 1 | 4 |
| Archive and both Swift jobs complete | Rust platform check, two native Rust shards, iOS chain | 4 | 1 | 5 |
| Native shards complete | Rust platform check, iOS chain | 2 | 1 | 3 |

If Swift finishes before the archive, the native shards still wait for the
archive, and the same four-slot bound applies. The capacity checker enumerates
weighted dependency antichains across the actual workflow graph, independent
of the timing assumptions. Its result is 14 landing jobs + 4 background jobs
+ 1 baseline recorder + 1 Nightly yield gate = 20 overall, and 4 + 1 = 5 Macs.
Removing any new capacity barrier is covered by a failing mutation test.

Using ticket run 43674843448 as the timing baseline, Linux's archive is ready
around minute 7 and all four shards should finish around minute 18. The native
archive should finish around minute 12, both Swift jobs around minute 20, and
parallel native shards around minute 26–27. Expect roughly **26–28 minutes**
instead of 46:38, assuming similar runner speed, warm caches and queue load.
The extra minute or two over the ticket's ideal 25-minute estimate retains the
existing background reservation. No hosted timing claim has been verified yet.

On the first main CI run after landing, record its SHA/run ID and fetch
`gh run view RUN_ID --json jobs,createdAt,updatedAt`. Confirm all four Linux
shards start together immediately after archive upload without an Android wait;
the Mac archive starts beside Swift; the two native shards overlap after Swift;
and interval counts stay within five Macs and 20 total jobs. Compare the
start-to-last-completion wall clock with 46:38 and the 26–28-minute estimate.
Separate runner queue delays from execution time if it misses the estimate.
Check that the deferred service/helper jobs also complete before the last
native shard. All existing job IDs, expanded check names, runner labels,
matrix members, artifacts and test commands are unchanged, including names
used by engine gates and branch protection. No test moves to Nightly.

## Foreground validation

All commands were run in the foreground and awaited. Python scripts using an
env shebang used `PATH=/opt/homebrew/opt/python@3.14/libexec/bin:$PATH` because
the default Python is 3.9 and the gate requires 3.11+.

- `./scripts/lane-gate.py --dry-run --base origin/main`: pass, all 11 local job families planned.
- `HEAVY_LOCK_HELD=1 ./scripts/lane-gate.py --self-test`: pass, including process supervisor, landing policy and engine gate tests. The first sandboxed attempt failed because `ps` was blocked; the rerun with process access passed.
- `python3 ./scripts/ci-concurrency-test.py`: pass, including capacity/yield self-tests and the new scheduling contract. The new contract fails against the original workflow on its Android dependency.
- `./scripts/ci-job-time-check.py --self-test`: pass.
- `python3 scripts/nightly-dispatch-test.py`: pass for scheduled, all and every targeted shard.
- Queue-aware actionlint at `/private/tmp/ov-1240-review-actionlint/011a6d15e749bb3f2d771eed9c7aa0e7e3e10ee7-cd4f58e8f71c/actionlint`: pass across the unmodified workflow inputs. This is the existing locally patched upstream validator documented in ov-1239, accepting only scalar `queue: max`/`queue: single`. Stock Homebrew actionlint still rejects the pre-existing `concurrency.queue` keys throughout the repository; no diagnostics were filtered and no workflow was transformed to obtain the passing result.
- `./scripts/file-size-budget.py` and `git diff --check`: pass.
- Parsed comparison against the original CI workflow: all job IDs, check-name templates, runners, steps, artifacts and test commands unchanged. Only dependencies, the two Rust shard concurrency limits, and Tunnel helper's explicit failure/skip-safe condition changed.

No application code changed, so application builds and full platform suites
were not rerun. The affected scheduling, local planning and capacity checks
above exercise this change.
