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
- Automatic waiting-join scans share each target head in a pass and run at most once every 30 seconds. Explicit queueing still verifies the head. The CI watcher follows pushed landing SHAs on its background schedule, with a minute between attempts and backoff for misses. The jobs memo keeps up to 512 completed runs and evicts the oldest entry when full. The persisted subject result suppresses polling for finished landings while they remain watched. A test observed zero eligible GitHub calls for a finished landing over a simulated minute. Watcher inventory keeps its one-second background cadence and one-minute backstop.

- A full Get for an unpublished workflow reads a per-repository starter settings cache. Cold or invalidated entries return `suggested_adapter_pending`. A background task performs the Git probes, with one batch in flight across repositories. A local origin edit invalidates the entry immediately and the age limit refreshes the branch tip. CLI commands that prepare a draft poll for the result. The ordinary full Get returns the state immediately.

## Idle spawn census

`idle_board_spawn_budget` runs the workflow dispatcher, terminal watcher, CI watcher, PR watcher, review watcher and landing reader for 60 wall-clock seconds against a private service. The fixture has an idle pane, one landing candidate waiting on a remote head, blocked cleanup folders, and a completed CI run with a last observation older than ten minutes. All workflow items stay at rest. The `lsof`, Git, `gh` and tmux shims append a line per invocation. Git and tmux forward local commands to the real executables, and the remote and CI paths are simulated locally. The interval includes the jobs' first wakes and excludes fixture setup. On the earlier measured base the fixture used a Pages CI subject and sent a one-second wake signal. Starting with landing base `8dc6d9795`, a paused landing group names the completed run through its pushed SHA and the test counts both SHA and per-run requests.

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

All four read latency bounds passed, and the read sequence spawned zero tmux clients. In the idle minute, the daemon launched 20 `lsof` commands (9 recursive), 71 Git commands (2 remote-head probes), 1 `gh` command (0 polls of the finished run), and 64 tmux clients.

The full CLI, store, and workflow suites and the affected daemon unit and integration suites covered 3,385 tests. The initial run passed 3,319. The other 66 passed on rerun with short private fixture paths and a corrected test home. The daemon layer, file-size, and Nightly dispatch checks passed.

## Return-note verification

I replayed the paused-clock tests against the previous scheduler with an adapter for its probe signature. With 20-second probes, occupied time dropped from 100% to 80% over a 100-second sweep across distinct paths and from 57.5% to 20% over ten minutes with one blocked path. The new tests also verify that a blocked path leaves other paths eligible, that its backoff caps at one hour, and that a successful probe clears the backoff.

The loaded fixture's RPCs took 527 ms for Plan, 1,331 ms for Plan theme list, 330 ms for Workflow Get, 49 ms for Task List's first page, and 0.33 ms for Worktree List. The benchmark recorded zero tmux launches during those reads.

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

## Verification on landing base `8dc6d9795`

The rebased CI watcher follows pushed landing SHAs. The idle fixture warms the SHA through the watch, ages the stored result beyond the former refresh interval, and checks that the watcher retains it without listing the SHA's runs or fetching the completed run's jobs again.

| Loaded RPC | Rebased latency | Budget |
| --- | ---: | ---: |
| Plan | 319 ms | < 2 s |
| Plan theme list | 598 ms | < 2 s |
| Workflow Get | 101 ms | < 1 s |
| Task List, first page | 44 ms | — |
| Worktree List | 0.75 ms | < 2 s |

The loaded read sequence used the cached tmux inventory and made zero tmux calls. In the idle minute the daemon launched 23 `lsof` commands (12 recursive), 72 Git commands (one remote-head probe), one `gh` command (zero polls of the finished run), and 63 tmux clients.

The first broad suite inherited the ticket runner's hook socket setting, which misrouted CLI hook fixtures. With a clean test environment, 3,289 of 3,292 selected CLI, daemon, store, and workflow tests passed in parallel. The three failures passed on isolated reruns: the native notice cleanup receipt, an exited owner's pane quiescence, and review recovery after a vanished tmux server. The daemon-layer, file-size, and Nightly dispatch checks passed.

## Return-review verification

The pane regression shares a scratch Service's database, repository fences, and current-thread Tokio executor with its workflow runtime. It sends input through the pane stream while a 350 ms `lsof` shim is running and during the real accounting walk of 48,000 output files. It observes the echo through a kernel pipe, and each latency includes any delay before the input task can wake. The fixed run echoed 15 keys, including 13 during the probe and one during the walk, with a maximum of 8.2 ms. A second fixed run reached 7.3 ms. Temporarily moving accounting and deletion inline produced a 1.76 s key delay and failed the 100 ms assertion. The test waits for cleanup to finish and verifies the recorded byte count. The filesystem cost is tracked by [ov-1438](farcooler://task/-/ov-1438).

Process-log pruning runs on a blocking worker in the first cleanup round and on the hourly maintenance wake. The pruner selects existing sidecars before reading outboxes and rechecks receipts before deletion. The idle regression failed when pruning was restricted to the startup round. It also checks that either a process receipt or a journal preserves output, and a separate test protects both log names for an admitted effect that has yet to write a receipt.

The loaded fixture passed all four budgets. Plan took 743 ms, Plan theme list 958 ms, Workflow Get 171 ms, and Worktree List 0.30 ms. The first Task List page took 52 ms. The idle minute recorded 23 `lsof` launches (12 recursive), 72 Git launches (one remote-head probe), one `gh` launch (zero polls of the finished run), and 64 tmux launches.

The workflow runtime and service suites passed 537 non-ignored tests across the broad run and reruns with the scratch daemon and CLI binaries. The pane regression and both performance checks ran separately. The daemon-layer, file-size, and Nightly dispatch checks passed.

## Verification on landing base `5bfcd67e8`

Git replayed all nine ticket commits without conflicts. The patch series and the resulting `ci_watch.rs` matched the prior candidate.

| Loaded RPC | Rebased latency | Budget |
| --- | ---: | ---: |
| Plan | 300 ms | < 2 s |
| Plan theme list | 572 ms | < 2 s |
| Workflow Get | 134 ms | < 1 s |
| Task List, first page | 23 ms | — |
| Worktree List | 0.22 ms | < 2 s |

The loaded reads launched zero tmux clients. The idle minute recorded 13 `lsof` launches (12 recursive), 33 Git launches (one remote-head probe), zero `gh` launches, and two tmux launches. The census checked that the finished CI result was still available and passed all spawn bounds. The pane test echoed 15 keys during the cleanup sweep, including 14 during `lsof`, and also overlapped the filesystem accounting walk. Its slowest echo was 8.1 ms.

Nextest ran 5,119 tests across the daemon unit suite, three daemon socket integration suites, and the CLI, store, and workflow suites. The parallel run passed 5,115. All four failures passed on a serial rerun with the same binaries and private test setup. They covered two native owner exit races, recovery after a vanished native tmux server, and keeping four native builders alive through a landing. The migration lint and its self-test, daemon-layer, file-size, doc-comment, and Nightly dispatch checks passed.

## Verification on landing base `d7b824fc9`

The workflow RPC keeps the landing base's bundle export branch and ov-1429's cached starter settings for ordinary Get. All 11 draft-approval integration tests passed, including bundle export with oversized state, detailed draft refusals, and starter settings on an unpublished workflow.

| Loaded RPC | Rebased latency | Budget |
| --- | ---: | ---: |
| Plan | 1,698 ms | < 2 s |
| Plan theme list | 1,751 ms | < 2 s |
| Workflow Get | 438 ms | < 1 s |
| Task List, first page | 24 ms | — |
| Worktree List | 0.94 ms | < 2 s |

The loaded fixture passed all four bounds and made zero tmux calls during the reads. The idle minute recorded 20 `lsof` launches (nine recursive), 66 Git launches (one remote-head probe), one `gh` launch (zero polls of the finished run), and 64 tmux launches.

The pane test's first run overlapped compilation and measured a 291 ms echo before it observed either cleanup phase. After compilation finished, the standalone rerun echoed 10 keys with a maximum of 73.1 ms. Eight echoes overlapped `lsof`, and the test also observed an echo during the filesystem walk and verified the cleanup receipt.

Nextest ran 4,837 tests across the daemon, CLI, store, and workflow suites and selected socket integrations. The first run passed 4,772. The fallback app directory exceeded the macOS Unix socket path limit. All 65 initial failures passed on a serial rerun with a shorter private app directory and the same binaries. Migration lint and its self-test, daemon-layer, file-size, doc-comment, and Nightly dispatch checks passed.

## Verification on landing base `e69f25328`

The CI watch keeps the landing base's retry and recovery checks. A CI retry can reuse a GitHub run ID. The watch reuses jobs for a completed attempt as long as its conclusion and update time match, and refreshes jobs when the run becomes active again. The regression test covers those transitions and checks that replacing a completed attempt takes one cache slot.

| Loaded RPC | Rebased latency | Budget |
| --- | ---: | ---: |
| Plan | 744 ms | < 2 s |
| Plan theme list | 1,024 ms | < 2 s |
| Workflow Get | 199 ms | < 1 s |
| Task List, first page | 42 ms | — |
| Worktree List | 0.25 ms | < 2 s |

The loaded reads launched zero tmux clients. The idle minute recorded 23 `lsof` launches (12 recursive), 72 Git launches (one remote-head probe), one `gh` launch (zero polls of the finished run), and 64 tmux launches.

The pane latency test exceeded its 100 ms deadline on three runs, with maxima of 317 ms, 120 ms, and 125 ms as the probe completed. A standalone run with fixture data under `FARCOOLER_SCRATCH` passed. All ten echoes arrived within 49.6 ms. Eight overlapped the open-file probe, and the test also observed the accounting walk and verified the cleanup receipt. The cause of the earlier timing misses remains unresolved.

All 15 CI watcher tests passed. Nextest ran 4,728 tests across the daemon, CLI, store, and workflow suites and selected socket integrations. The parallel run passed 4,725. All three initial failures passed in a serial rerun with the same binaries. They covered native notice cleanup, a recorded verdict without a turn-end event, and recovery of an admitted CI effect after reconfiguration. Migration lint and its self-test, daemon-layer, file-size, doc-comment, and workflow lint checks passed.

## Verification on landing base `ab1c6074d`

Git replayed all 12 ticket commits cleanly. The patch series matched the previous candidate, including the changes to `process_logs.rs` and `wire_commands.rs`.

| Loaded RPC | Rebased latency | Budget |
| --- | ---: | ---: |
| Plan | 322 ms | < 2 s |
| Plan theme list | 324 ms | < 2 s |
| Workflow Get | 139 ms | < 1 s |
| Task List, first page | 29 ms | — |
| Worktree List | 0.28 ms | < 2 s |

The loaded reads made zero tmux calls. The idle minute recorded 23 `lsof` launches (12 recursive), 72 Git launches (one remote-head probe), one `gh` launch (zero polls of the finished run), and 64 tmux launches.

The first pane latency run exceeded the 100 ms deadline by 6.2 ms. The standalone rerun passed with seven echoes, a maximum of 88.4 ms, five echoes during the open-file probe, and an echo during the accounting walk. It also verified the cleanup receipt's byte count. After the test-only rebuild, one run reached 112.5 ms and the standalone rerun passed with ten echoes and a maximum of 47.0 ms. Eight echoes overlapped the probe and the test observed the accounting walk. The source of the timing misses remains unmeasured. [Tmux overhead follow-up (ov-1450)](farcooler://task/-/ov-1450)

Nextest ran 4,736 tests across the daemon, CLI, store, and workflow suites and three workflow socket integrations. The parallel run passed 4,732. Two native delivery and steering cases passed after building the runnable CLI in scratch. Two quiescence cases failed on the immediate pane check after shutdown. The test helper now retries a transient tmux error once after 20 ms and still requires tmux to report the pane gone. All six native quiescence tests passed on the rebuilt candidate. The opt-in CLI test also passed while reading workflow logs from a pane that was writing output.

Migration lint and its self-test, daemon-layer, file-size, doc-comment, and workflow lint checks passed.

## Verification on landing base `dbae6e7fa`

The rebase keeps restart pacing, startup cleanup, and recurring log pruning in the same runtime. Restart pacing reads only indexed active effects. The loaded benchmark stages fresh work in a continuous dispatch sweep, and it follows the same Plan overview and theme detail pages as the rebased CLI. The extracted workflow RPC retains the full, summary, Plan, legacy, bundle, and re-pin replies and uses the starter settings cache.

| Loaded read | Latency | Budget |
| --- | ---: | ---: |
| Plan | 743 ms | < 2 s |
| Plan theme list | 609 ms | < 2 s |
| Workflow Get | 140 ms | < 1 s |
| Task List, first page | 50 ms | — |
| Worktree List | 0.40 ms | < 2 s |

The read sequence launched zero tmux clients. The idle minute recorded 13 `lsof` launches (12 recursive), 52 Git launches (one remote-head probe), one `gh` launch (zero polls of the finished run), and two tmux launches. All 20 pane echoes arrived within 100 ms. The slowest took 25.5 ms, ten overlapped the open-file probe, and an echo overlapped the accounting walk. The cleanup receipt recorded all 48,000 bytes.

Nextest passed 4,826 of 4,828 tests across the daemon, CLI, store, and workflow unit suites and five socket and CLI integration suites. Both failures repeated in isolation with the same assertions on the unmodified landing base. The macOS native-hold tests reported `ended` for a stray process and found the same PID for the relay and the hold. Migration lint and its self-test, daemon-layer, file-size, doc-comment, Nightly dispatch, and workflow lint checks passed.

## Verification on landing base `ebc4b7867`

The rebased cleanup keeps the landing base’s artifact provenance checks, checkout preservation, retry deadlines, and disk totals. Both preflight and final open-file checks use the service’s shared probe budget. A blocking worker checks legacy checkout ownership while holding the tree fence. Recovery and hourly log pruning also run on blocking workers.

| Loaded read | Latency | Budget |
| --- | ---: | ---: |
| Plan | 642 ms | < 2 s |
| Plan theme list | 1,231 ms | < 2 s |
| Workflow Get | 169 ms | < 1 s |
| Task List, first page | 37 ms | — |
| Worktree List | 0.45 ms | < 2 s |

The loaded reads made zero tmux calls. The idle minute recorded 23 `lsof` launches (12 recursive), 72 Git launches (one remote-head probe), one `gh` launch (zero polls of the finished run), and 63 tmux launches. All 16 pane echoes met the 100 ms bound. The slowest took 53.5 ms, seven overlapped the open-file probe, and an echo overlapped the accounting walk. The cleanup receipt recorded all 48,000 bytes.

The rebased Workflow Get can wait up to 200 ms for the first disk-usage sample. The sample runs on a blocking worker and later requests can use the cached result. The existing follow-ups cover filesystem accounting and the remaining read projection costs. [Filesystem accounting (ov-1438)](farcooler://task/-/ov-1438) [Read projection (ov-1436)](farcooler://task/-/ov-1436)

Nextest ran 4,784 tests across the daemon, CLI, store, and workflow suites and ten CLI and socket integration suites. The first pass passed 4,717 tests. The scratch alias resolved to a path too long for native Unix sockets. The test guard reaped one private tmux server left by that run. All 67 affected tests passed with the same binaries and a short private socket path. The loaded read, spawn census, and pane echo regressions also passed in standalone runs. Migration lint and its self-test, daemon-layer, file-size, doc-comment, Nightly dispatch, and workflow lint checks passed.
