# ov-1429 daemon read evaluation

## Method

I copied only the canary SQLite database into `FARCOOLER_SCRATCH` and ran `PRAGMA quick_check` on the copy. It contained 1,579 tickets across all boards, 30 themes, 135 worktrees, 11,687 agent turns, and 3,912 outbox rows. The workflow board's state was 915,076 bytes. The profile harness opens the copied store through a scratch service and calls the production RPC handler. Its effect owner stays stopped because the original adapter paths and remote bindings are still in the copy. It uses its own install ID and tmux socket. The recorded November 23 owner timings describe the installed runner under load. All new measurements here use scratch.

## Findings

| Read | Scratch baseline | Where the time went |
| --- | ---: | --- |
| `plan` | 4.92 s, 21.52 s while sampling | The per-ticket spend query walked turn history and joined models. The profile has `Store::plan` → `spend_of_ticket` → `sqlite3VdbeExec`. |
| `plan theme list` | 4.61 s, 16.87 s while sampling | The CLI uses the same `plan.get` projection with closed tickets included. The original scratch probe timed a second copy of the default projection. Theme trend queries walked turn history per themed ticket even after the initial index change. |
| `workflow get` | 189–371 ms | Execution status loaded every board's outbox and held the runtime activity mutex while deserializing. Recent process evidence read the same history again. |
| `task list` | Original unpaged request panicked | The UTF-8 ellipsis could lengthen the field, which underflowed the encoded-size adjustment. The paged CLI uses the task projection in bounded replies. |
| `worktree list` | Original copied-board sequence ended at the task-list panic | A request refreshed tmux inventory inline, with two attempts separated by 150 ms after the first failure. Fleet projection also cloned the inventory once per worktree. |

The profile excerpt records the baseline, the intermediate index-only capture, and the final registry capture. On the copied database, a standalone SQL reproduction of the spend query across 476 worked tickets took 13.74 s before indexing and 135 ms with the index. After indexing, the trend query still took 12.70 s across the same tickets because SQLite started from the turn index used for grouping. Keeping the ticket's agent rows on the outer side of the join reduced that query run to 189 ms. These are aggregate SQL timings from the system Python SQLite build. The RPC runner uses Cargo's SQLite build and also includes projection work. [Symbolized profile](profile.txt)

## Verification under load

The generated fixture has 1,500 tickets, 25 themes, at least 900 KB of engine state, and 4,000 settled effects in addition to the live dispatch rows. It runs four real fixture build subprocesses and the candidate's pre-landing check concurrently. All five subprocesses register their activity before the RPCs start. The fixture keeps a pane open on a private tmux server so a new inventory call is observable.

| RPC behind the command | Measured | Budget |
| --- | ---: | ---: |
| Plan | 358–1,365 ms | < 2 s |
| Plan theme list | 410–1,431 ms | < 2 s |
| Workflow get | 168–391 ms | < 1 s |
| Task list, first page | 24–26 ms | — |
| Worktree list | 0.28 ms | < 2 s |

Both benchmark runs met the budgets. The benchmark also checked that the read sequence spawned zero tmux clients. To run the loaded fixture, set `CARGO_TARGET_DIR="$FARCOOLER_SCRATCH/target"` and run `cargo test -p farcooler-daemon --lib board_read_latency_with_engine_work -- --ignored --nocapture`. The enforced bounds cover RPC time. The copied-board profile and the benchmark use the daemon handler directly, so the results exclude socket and CLI rendering time.

In six passes over the copied board with the final SQL, median times were 228 ms for Plan, 345 ms for theme list, 84 ms for Workflow get, 55 ms for Task list, and 6 ms for Worktree list. One theme-list read took 4.14 s while the sampling and store-test processes were active. The capture did not resolve that spike. A registry 70 replay concurrent with compilation also had an 8.31 s Plan outlier. Its few samples reached the workflow state overlay but did not account for the whole outlier. Three actual `lsof +D` probes on a disposable 4,000-file tree took 537, 294 and 283 ms each. Those probes remain background work.

## Changes and limits

- Indexed terminal turns, active effects and per-item effect history. Registry 70 pins those additive indexes in the existing cutover catalog and preserves the raw row manifest. Plan spend and trend joins start from the ticket's agents. Workflow execution reads settled effects only if an activity still owns them. Logs keep their existing per-item history.
- Worktree listing and terminal-change announcements reuse the watcher inventory. Workflow admission uses the same process proof at capture and recheck, skipping the tmux fallback the final check could not accept. Listing takes one snapshot for the fleet. Added a pane-echo test that sends keys through the daemon's persistent input implementation while a cleanup probe is held at its preflight boundary.
- Cleanup starts when work changes, retries on its own timer, and keeps an hourly maintenance wake. It retains the live-writer, repository and path fences. Process-log pruning runs on the blocking pool during recovery. The service spaces recursive open-file probes at least five seconds apart and uses a 15-second cleanup retry interval. A deterministic minute with a backlog of probe attempts reached the spawn boundary 12 times.
- Automatic waiting-join scans share each target head in a pass and run at most once every 30 seconds. Explicit queueing still verifies the head. The CI watcher coalesces repeated kicks, keeps completed SHA and run results while the subject remains named, and checks Main's head before re-reading its runs. A test observed zero eligible GitHub calls for finished fixed subjects over a simulated minute. Watcher inventory keeps its one-second background cadence and one-minute backstop.

The owner observed active child commands in a 10-second sample, which measures concurrency and time spent alive. It cannot be converted to launch counts. The probe and CI tests count or gate the targeted spawn paths. The full Get reply for an unpublished workflow still generates starter settings with local Git probes. A full per-executable idle launch census of a daemon running all production jobs on the copied board remains unverified.

## Follow-up tickets

These local ticket drafts cover the measured remaining costs over 100 ms and providers that can exceed that threshold. They are stored with this report because the live board was outside the measurement scope.

| Ticket | Cost | Acceptance |
| --- | --- | --- |
| [ov-1429 A](follow-ups.md#ov-1429-a--avoid-recursive-open-file-walks) | Recursive open-file checks and filesystem accounting | Bound the cost of one large-tree cleanup check and preserve every open-file and disk fence. |
| [ov-1429 B](follow-ups.md#ov-1429-b--profile-the-full-cli-and-idle-runner) | Plan projection, Workflow get startup and loaded reads, Task list startup, whole-CLI time, and idle launch census | Measure and attribute endpoint and CLI time and count launches per executable on a self-contained replay. |
| [ov-1429 C](follow-ups.md#ov-1429-c--keep-terminal-ci-observations-across-subjects) | GitHub and Git transport calls | Persist terminal run observations across changes to the watched subject set and measure provider time separately. |

## Test results

The full store suite passed with 674 tests and three ignored. The targeted daemon suites for cleanup, scheduling, effect observation, admission, process logs, CI watch, and task paging passed, including the scratch pane's five 100 ms echo deadlines. The daemon layer and file-size checks passed.
