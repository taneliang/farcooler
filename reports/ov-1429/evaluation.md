# ov-1429 daemon read evaluation

## Method

I copied only the canary SQLite database into `FARCOOLER_SCRATCH` and ran `PRAGMA quick_check` on the copy. It contained 1,579 tickets across all boards, 30 themes, 135 worktrees, 11,687 agent turns, and 3,912 outbox rows. The workflow board's state was 915,076 bytes. The profile harness opens the copied store through a scratch service and calls the production RPC handler. Its effect owner stays stopped because the original adapter paths and remote bindings are still in the copy. It uses its own install ID and tmux socket. The recorded November 23 owner timings describe the installed runner under load. The profile and regression measurements use scratch.

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

| RPC behind the command | Base `81b30964c` | Fixed replay | Budget |
| --- | ---: | ---: | ---: |
| Plan | 16,128 ms | 388 ms | < 2 s |
| Plan theme list | 15,720 ms | 356 ms | < 2 s |
| Workflow get | 654 ms | 182 ms | < 1 s |
| Task list, first page | 39 ms | 35 ms | — |
| Worktree list | 13.79 ms | 0.24 ms | < 2 s |

The test module was copied unchanged to an archive of base `81b30964c`. It reached all five reads and failed its two-second Plan assertion. The fixed replay met all four budgets and recorded zero tmux launches during the read sequence. Two earlier fixed runs also met the budgets, with Plan at 358–1,365 ms, theme list at 410–1,431 ms and Workflow Get at 168–391 ms. The Nightly workflow runs this benchmark and the idle census as individual checks in Daemon production timing. Each check expects exactly one passing test. For a local run, set `CARGO_TARGET_DIR="$FARCOOLER_SCRATCH/target"` and run `cargo test -p farcooler-daemon --lib board_read_latency_with_engine_work -- --ignored --nocapture`. The enforced bounds cover RPC time. The copied-board profile and the benchmark use the daemon handler directly, so the results exclude socket and CLI rendering time.

In six passes over the copied board with the final SQL, median times were 228 ms for Plan, 345 ms for theme list, 84 ms for Workflow get, 55 ms for Task list, and 6 ms for Worktree list. One theme-list read took 4.14 s while the sampling and store-test processes were active. The capture did not resolve that spike. The read-latency follow-up tracks it together with the Plan outlier. A registry 70 replay concurrent with compilation also had an 8.31 s Plan outlier. Its few samples reached the workflow state overlay but did not account for the whole outlier. Three actual `lsof +D` probes on a disposable 4,000-file tree took 537, 294 and 283 ms each. Those probes remain background work.

## Copied-board RPC and CLI replay

The second read-only snapshot passed SQLite's quick check and contained 1,581 tickets, 30 themes, 64 worktrees, 11,725 turns, 3,983 outbox rows and 921,025 bytes of workflow state. I ran 35 passes against the production handler over 30 seconds, then sent all five CLI commands to that handler over a private Unix socket. The CLI sample includes startup, transport, pagination and rendering. It used the installed Canary CLI with the scratch daemon. The table uses nearest-rank p95 for the RPC timings.

| Command | RPC median | RPC p95 | One full CLI run |
| --- | ---: | ---: | ---: |
| Plan | 227 ms | 577 ms | 376 ms |
| Plan theme list | 350 ms | 893 ms | 652 ms |
| Workflow Get | 89 ms | 129 ms | 94 ms |
| Task List | 55 ms | 97 ms | 287 ms |
| Worktree List | 3.3 ms | 6.7 ms | 42 ms |

To repeat the full CLI measurement, set `FARCOOLER_PROFILE_ROOT` to the disposable copy under `FARCOOLER_SCRATCH`, `FARCOOLER_PROFILE_SECONDS=30` and `FARCOOLER_PROFILE_CLI` to an absolute CLI path. Run `cargo test -p farcooler-daemon --lib profile_copied_board -- --ignored --nocapture`.

## Changes and limits

- Indexed terminal turns, active effects and per-item effect history. Registry 70 pins those additive indexes in the existing cutover catalog and preserves the raw row manifest. Plan spend and trend joins start from the ticket's agents. Workflow execution reads settled effects only if an activity still owns them. Logs keep their existing per-item history.
- Worktree listing and terminal-change announcements reuse the watcher inventory. Workflow admission uses the same process proof at capture and recheck, skipping the tmux fallback the final check could not accept. Listing takes one snapshot for the fleet. Added a pane-echo test that sends keys through the daemon's persistent input implementation while a cleanup probe is held at its preflight boundary.
- Cleanup starts when work changes, retries on its own timer, and keeps an hourly maintenance wake. It retains the live-writer, repository and path fences. Process-log pruning runs on the blocking pool during recovery. The service waits at least five seconds after a recursive open-file probe completes before starting the next probe. Failed probes back off per path, doubling from 15 seconds to one hour and resetting after the check succeeds. A deterministic minute with a backlog of fast probes reached the spawn boundary 12 times.
- Automatic waiting-join scans share each target head in a pass and run at most once every 30 seconds. Explicit queueing still verifies the head. The CI watcher coalesces repeated kicks, keeps completed SHA and run results while the subject remains named, and checks Main's head before re-reading its runs. The jobs memo keeps up to 512 completed runs and evicts the oldest entry when full. The persisted subject result still suppresses polling for finished fixed subjects. A test observed zero eligible GitHub calls for those subjects over a simulated minute. Watcher inventory keeps its one-second background cadence and one-minute backstop.

- A full Get for an unpublished workflow reads a per-repository starter settings cache. Cold or invalidated entries return `suggested_adapter_pending`. A background task performs the Git probes, with one batch in flight across repositories. A local origin edit invalidates the entry immediately and the age limit refreshes the branch tip. CLI commands that prepare a draft poll for the result. The ordinary full Get returns the state immediately.

## Idle spawn census

`idle_board_spawn_budget` runs the workflow dispatcher, terminal watcher, CI watcher, PR watcher, review watcher and landing reader for 60 wall-clock seconds against a private service. The fixture has an idle pane, one landing candidate waiting on a remote head, blocked cleanup folders, and a completed CI run with a last observation older than ten minutes. All workflow items stay at rest. The `lsof`, Git, `gh` and tmux shims append a line per invocation. Git and tmux forward local commands to the real executables, and the remote and CI paths are simulated locally. The interval includes the jobs' first wakes and excludes fixture setup. A one-second wake signal exercises CI notification handling against the retained completed run.

The owner's 10-second process sample reports how frequently children were alive. The census counts executable invocations.

| Executable or command | Base `81b30964c` launches/min | Fixed launches/min | Bound |
| --- | ---: | ---: | ---: |
| `lsof`, all | 507 | 13–23 | ≤80 |
| `lsof +D` | 496 | 12 | 1–12 |
| Git, all | 1,355 | 57–77 | ≤100 |
| `git ls-remote --refs` | 64 | 2 | 1–2 |
| `gh`, all | 3 | 1 | ≤4 |
| `gh` for completed run 73 | 2 | 0 | 0 |
| tmux | 65 | 2–64 | 1–150 |

The base archive carried the same fixture and the test-only executable substitution for macOS's absolute `/usr/sbin/lsof` path. Its recursive probe count failed the bound. The all-Git count includes the cleanup path's repository guards and tracked-file check. A local run uses `cargo test -p farcooler-daemon --lib idle_board_spawn_budget -- --ignored --nocapture`.

## Follow-up tickets

| Remaining cost | Measurement | Board ticket |
| --- | --- | --- |
| Recursive open-file scan and filesystem accounting | `lsof +D` took 283–537 ms per probe on a disposable 4,000-file tree. The deletion accounting walk was not separately timed. | [ov-1438](farcooler://task/-/ov-1438) |
| Plan and theme-list projection | On the 35-pass copied-board replay, Plan p95 was 577 ms and theme-list p95 was 893 ms. The earlier Plan outlier was 8.31 s during compilation and the theme-list outlier was 4.14 s during sampling and store tests. | [ov-1436](farcooler://task/-/ov-1436) |
| Workflow Get and cold Task List | Copied-board Get p95 was 129 ms and loaded Get reached 391 ms in the first fixed runs. Cold Task List crossed 100 ms in the first copied-board profile. | [ov-1436](farcooler://task/-/ov-1436) |
| Whole CLI path | The owner measured Plan 207 s, theme list 98 s, Plan JSON 71 s with no output, worktree list 56 s, task list 19 s and Workflow Get 0.6–70 s including timeouts, on the installed runner before these changes. With the scratch service, end-to-end Plan was 376 ms, theme list 652 ms and Task List 287 ms. The replay has one CLI sample per command. | [ov-1436](farcooler://task/-/ov-1436) |
| Git and GitHub transport | In the owner's 40 quarter-second samples, Git was present in 25 samples and GitHub job queries in about 30. Per-call wall times were not isolated. The follow-up also covers CI results after subjects are removed and re-added, or a different subject looks up the same run after restart. | [ov-1437](farcooler://task/-/ov-1437) |
| Background tmux inventory | The owner saw tmux in 23 of 40 quarter-second samples. The rebased scratch census counted 64 launches in one idle minute, versus 65 on the measured base. The loaded read sequence launched zero tmux clients. Per-call wall times under load are unmeasured. | [ov-1450](farcooler://task/-/ov-1450) |

## Test results

The store suite passed 674 tests. The 132 targeted daemon tests covered cleanup, scheduling, effect observation, admission, log reads, CI watch and task paging. All nine draft-approval integration tests and 649 CLI tests passed. The daemon layer, file-size and nightly-dispatch checks passed.

The private-pane test sent bytes through the real input channel while a cleanup sweep was held in preflight. Each key had its own 100 ms deadline. The five measured echoes took 9.1–10.4 ms and the resumed sweep released the output.

## Landing-base verification

On landing base `1c0c2027a`, the dispatcher uses the new automatic queue check on the existing 30-second cadence. The idle fixture now enables automatic queueing and places its candidate at the join stage so the census exercises that check.

| Loaded RPC | Rebased replay |
| --- | ---: |
| Plan | 1,175 ms |
| Plan theme list | 1,237 ms |
| Workflow Get | 347 ms |
| Task List, first page | 33 ms |
| Worktree List | 5.2 ms |

All four read latency bounds passed, and the read sequence spawned zero tmux clients. In the idle minute, the daemon launched 20 `lsof` commands (9 recursive), 71 Git commands (2 remote-head probes), 1 `gh` command (0 polls of the finished run), and 64 tmux clients. During cleanup, all five pane echoes arrived within the 100 ms budget, with a slowest echo of 89.7 ms. The resumed sweep released the output.

The full CLI, store, and workflow suites and the affected daemon unit and integration suites covered 3,385 tests. The initial run passed 3,319. The other 66 passed on rerun with short private fixture paths and a corrected test home. The daemon layer, file-size, and Nightly dispatch checks passed.

## Return-note verification

I replayed the paused-clock tests against the previous scheduler with an adapter for its probe signature. With 20-second probes, occupied time dropped from 100% to 80% over a 100-second sweep across distinct paths and from 57.5% to 20% over ten minutes with one blocked path. The new tests also verify that a blocked path leaves other paths eligible, that its backoff caps at one hour, and that a successful probe clears the backoff.

The loaded fixture's RPCs took 527 ms for Plan, 1,331 ms for Plan theme list, 330 ms for Workflow Get, 49 ms for Task List's first page, and 0.33 ms for Worktree List. The benchmark recorded zero tmux launches during those reads. The slowest of the five pane echoes during the cleanup sweep took 10.8 ms.

The idle census passed with 23 `lsof` processes (12 recursive), 77 Git processes (two remote-head probes), one `gh` process (zero polls of the finished run), and 64 tmux clients in one minute.

All 91 selected daemon unit tests passed across build cleanup, group tree release, owned directories, CI watch, builder resources, and workflow concurrency. The native Codex owned-directory fixture remained ignored because it requires a real login and dedicated scratch credentials.

## Verification on landing base `fec026430`

The rebased dispatcher adopts the current publication and refreshes groups before the rate-limited automatic queue check. Publication previews use the same read-only classification in each inbox path, so a preview does not request cleanup.

| Loaded RPC | Rebased latency | Budget |
| --- | ---: | ---: |
| Plan | 636 ms | < 2 s |
| Plan theme list | 723 ms | < 2 s |
| Workflow Get | 184 ms | < 1 s |
| Task List, first page | 48 ms | — |
| Worktree List | 0.44 ms | < 2 s |

The loaded read sequence launched no tmux clients. The idle minute recorded 22 `lsof` invocations (12 recursive), 77 Git invocations (two remote-head probes), one `gh` invocation (zero polls of the finished run), and 64 tmux invocations. All five echoes during the cleanup sweep arrived within 100 ms, with a slowest echo of 20.1 ms. The resumed sweep released the output.

The CLI, workflow, and store suites passed 708, 266, and 674 tests respectively. The selected daemon socket suite passed 53 tests. Nextest passed 701 of 706 selected daemon unit tests. The remaining five passed on a serial rerun after restoring the dispatcher scan after an inbox message and putting native Python first on the test path. The rerun also passed both ov-1425 publication migration tests and the root-guard test with the macOS scratch home spelled under `/tmp`. The daemon layer, file-size, and Nightly dispatch checks passed.
