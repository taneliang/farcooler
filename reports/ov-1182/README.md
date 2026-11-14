# ov-1182: Rust CI under 15 minutes

All ten per-push Rust jobs passed in two branch runs, with maxima of **11m19s**
and **8m53s**. These are job `startedAt` → `completedAt` wall times,
including setup, compilation, tests, artifact transfer and cache teardown.
Runner queue waits are excluded; this is not a claim that the whole workflow
or its queues finish in 15 minutes. Raw timestamps remain in the job records.

- [Run 1: 43581462181](https://github.com/taneliang/farcooler/actions/runs/43581462181) at `5a532b87364d8e50eea79d7e837b7c453f3a1b01`.
- [Run 2: 43582348026](https://github.com/taneliang/farcooler/actions/runs/43582348026) at `9095d68b8997828856c8a806f482d7cee1dfac4f`.

| Rust job | Run 1 | Run 2 |
| --- | ---: | ---: |
| Rust (ubuntu-24.04) | 11m19s | 6m08s |
| Rust (macos-latest) | 5m35s | 7m16s |
| Rust test archive | 8m11s | 3m28s |
| Rust tests (1/4) | 4m21s | 4m31s |
| Rust tests (2/4) | 3m38s | 3m46s |
| Rust tests (3/4) | 4m31s | 4m08s |
| Rust tests (4/4) | 4m01s | 3m52s |
| Rust native test archive (macOS) | 4m19s | 8m53s |
| Rust native tests (macOS 1/2) | 6m10s | 6m15s |
| Rust native tests (macOS 2/2) | 5m32s | 6m03s |

Baseline main run [43557233452](https://github.com/taneliang/farcooler/actions/runs/43557233452)
took **24m10s** for Linux Rust and **39m01s** for Mac Rust. Its full-test/shadow
steps took 15m35s and 26m04s respectively. This is an observed before/after
comparison with differing cache states and runners, not a controlled causal
estimate or a guarantee for every cold cache or future runner.

## What changed

Linux compiles the default workspace once into a nextest archive and runs four
hash partitions. Mac compiles **every** workspace test target once into its own
archive and runs the existing ov-1181 native selection in two hash partitions.
The maintained Mac selector and its source/dependency guard are reused, not
replaced. Full default Mac execution remains in Nightly.

Both platforms package the runtime CLI, daemon and LFS filter, recreate the
channel aliases, remap the checkout and extract under their own worktree's
`target/`. Archive consumers do not compile the suite again. All feature tests,
target checks and doctests remain per push. All eight Clippy invocations run
once on Linux. No assertion, ignored-test policy, test timeout or retry policy
was relaxed. The job timeout is still a failure boundary, not the timing proof.

CI sets `CARGO_PROFILE_DEV_DEBUG=0` and `CARGO_PROFILE_TEST_DEBUG=0`. This removes
debug symbols while retaining optimization settings and debug assertions.
The initial symbol-bearing Linux artifact was 6,313,939,230 bytes; an early
symbol-free trial was 714,851,462 bytes. Final artifact sizes are recorded
separately. Panic locations and test names remain visible in the retained logs.

Uploads use `overwrite: true` to support complete-job reruns without a same-name
artifact conflict; GitHub's per-attempt logs/verdicts are retained. See the
[upload action contract](https://github.com/actions/upload-artifact).

## Coverage proof

Each shard lists its archive without partitioning and with every partition,
then rejects missing, duplicated, unexpected or changed runnable cases. The Mac
comparison applies the same native expression to its base and both partitions.

Both final runs cover **6,977 Linux runnable tests**, split
**1,713 / 1,740 / 1,741 / 1,783**, and **2,601 selected Mac tests**, split
**1,270 / 1,331**. These platform counts overlap and must not be added together
as unique tests. The existing ignored tests retain their policy (35 in Linux's
full inventory). No new test exclusion was introduced.

`coverage-verification.txt` and the four final inventory ZIPs preserve the
comparisons. Unit self-tests plant missing and duplicate cases; deleting one
assignment from a real hosted inventory also makes the guard reject it.
A real local archive additionally exercised four unfiltered partitions and two
filtered partitions, including an ignored case.

The local lane gate derives one **unfiltered** workspace run from CI's shard
command, under both leak wrappers. Its regression test rejects a narrowed
prefix and recognizes all new Rust jobs. The iOS checker now scopes its matrix
rule to iOS jobs and still rejects an iOS smoke matrix; Nightly's report-needs
mutation remains effective with the new Rust jobs. `integration-validation.txt`
records the previously missing local suite and the corrected plan.

## Measurement and validation

`hosted-timing.yml` is the exact temporary branch-push workflow used by both
final runs. It copies production Rust jobs, removing only the path-selection
job/dependencies and their conditions. The temporary workflow is removed from
`.github/workflows/` after measurement.

Run `python3 reports/ov-1182/verify-hosted.py reports/ov-1182/run1-jobs.json reports/ov-1182/run2-jobs.json`
from this checkout with the measured commits available. It checks current
production-job parity, the exact workflow at both measured SHAs, two distinct
run IDs, all ten successful verdicts and every job strictly below 900 seconds.

`local-validation.txt` records passing actionlint (all workflows), ci-changes,
partition/mutation tests, Mac coverage guard, leak checks, shadow self-tests,
Nightly report, iOS shard checks, local lane gate, 16 evidence tests and 9 engine
contract tests. The inherited ov-1181 actionlint annotations and xcode-27 label
configuration allow plain `actionlint` to pass repository-wide.

## Shadow measurement and unsuccessful trials

Shadow measurement moves to Nightly, including green-base discovery, inventory
comparison, load waits and optional sequential selected-suite replay. Baseline
Linux's full command took 588.47 s (584.716 s in nextest) inside its 935 s step;
Mac took 1040.50 s (1034.879 s in nextest) inside 1564 s. Neither replayed a
narrowed proposal on that workflow-changing commit. The remaining 346.53 /
523.50 s include compilation during listing, so they cannot all be attributed
to shadow bookkeeping. The move removes optional replay work from per-push CI;
the baseline does not prove minutes saved solely by deleting bookkeeping.

Earlier trials are retained and **excluded** from final acceptance:

- 43573567048: initial Linux checks/archive passed in 15m16s/15m21s; the old
  Mac layout exceeded 20 minutes. Superseded work was cancelled. Initial
  compression alone took 100.47 s for the 6.31 GB artifact.
- 43574660997 / 43575477705: symbol-free trials exposed slow duplicate Mac
  Clippy work (379 s for workspace Clippy on one runner versus 126 s on another).
  One unchanged timing-sensitive test failed once in Linux shard 2:
  `a_whole_read_between_tears_restarts_the_cap`, at compose_held_tests.rs:405.
  It passed in the earlier trial in 3.117 s and failed after 4.167 s. Its
  300 ms healthy-registry window can be missed by the asynchronous retry loop;
  that is a fixture-timing hypothesis, not a claimed product fix. The complete
  failed-attempt log is retained. No test retry or assertion change was added.
- 43576689824 / 43576844809: superseded before measurement completion to make
  new artifact uploads replaceable on rerun.
- 43577077629 / 43577253123: the combined Mac native build/run hit its 20-minute
  job limit. One log shows 8m19s test compilation after a 2m34s binary build,
  followed by a native execution still running at cancellation. This led to
  the final separate archive and two native shards. Linux coverage in those
  trials was green; partial native execution is not credited as coverage.

The unsuccessful records, native phase logs and the intermittent failure are
preserved under `initial-*`, `symbols-*`, `pre-overwrite-*`, `pre-macos-shards-*`
and `run2-shard2-first-attempt.log.zip`. `run1-inventories.zip` is the earlier
symbol-only trial; final proofs are `run1-{linux,macos}-inventories.zip` and
`run2-{linux,macos}-inventories.zip`. Final complete logs and API records are
`run1-logs.zip`, `run2-logs.zip` and `run{1,2}-jobs.json`.
