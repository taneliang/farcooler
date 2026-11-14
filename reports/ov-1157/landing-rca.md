# ov-1157: why landing is slow, and what to change

Read-only investigation, 2026-11-14 ~10:50 +08. Window: Nov 13 00:00 to Nov 14 10:50 +08 (about 35 h). All times are local (+08) unless marked Z.

Sources: engine receipts in `~/Library/Application Support/com.farcooler.FarCoolerCanary/workflow/receipts/` (1,053 effects parsed; stage start = previous stage's receipt time, stage end = the stage's `.json` receipt mtime, because a gate writes its files only when it ends), `workflow get --json` for the landing definition, `~/Library/Logs/com.farcooler.FarCoolerCanary/farcoolerd.log*`, `gh run list/view` for 22 main CI runs, the gate `.output` summary tables, and origin/main code (e21474271).

## TL;DR

The landing lane was busy for the whole window and landed **17 tickets in about 32 h (0.53 tickets/h)**. One group costs about **1 h 50 min end to end** (gate ~45 min + CI ~50 min) and groups run strictly one at a time, so the ceiling is about **13 groups a day** even when nothing goes wrong. Things went wrong a lot:

| Where the hours went (lane-hours, Nov 13 00:00 to Nov 14 10:50) | h |
| --- | --- |
| Local gate, useful (passed, then pushed) | 10.1 |
| Local gate, wasted (rejected, killed or sabotaged, see below) | 3.6 |
| GitHub CI wait after push | 11.3 |
| `base` waiting for a green main | 5.9 |
| fold + push | 0.5 |
| **Total serial lane time** | **~31.4** |

Ranked root causes (hours of landing lane per day, normalized to 24 h):

1. **Strict serialization: CI of group N blocks the gate of group N+1.** ~8 h/day of CI wait during which the gate machine is idle. A speculative stack (gate N+1 on N's pushed commit while N's CI runs) roughly halves per-group latency.
2. **An *unknown* main stalls everything, and wait deadlines are stale.** `base` treats red main as passable but a pending, superseded or cancelled run as Unknown, and `cancel-in-progress` keeps producing cancelled runs. G643 waited the full 3 h. Then 7 groups (G793, G795, G797, G802, G815, G885, G1528) failed at `base` within 6 s of starting. The cause is that wait deadlines come from a stale `wait_since`: split halves inherit it, and so do relaunches after an interrupt. Those failures handed ~15 reviewed tickets back to build. That rework (rebuild + re-review) is the single largest hidden cost.
3. **Fold conflicts handled by halving instead of ejecting the named member.** 75 conflict receipts, 24 groups ended Failed at fold, 16 distinct tickets conflicted, and innocent co-members failed with them. The conflict receipt already names the member and files.
4. **The local gate duplicates CI almost exactly** (same ci.yml jobs: clippy, full nextest, Swift tests, app build, Android, iOS frameworks) at ~45 min, with `CARGO_BUILD_JOBS=4` and a load ceiling the lanes keep blowing through (load 30-54).
5. **Bisect is a peel, and some rejections are not the code's fault.** G1171 spent 2 x 38 min peeling one member at a time; G1326's gate was killed by an app update (36 min); G1567's gate tree was deleted mid-run by the orchestrator's ov-943 prune chore (16 min plus a spurious bisect).
6. **macOS CI slot contention and `cancel-in-progress`** add 15-118 min queue waits to overlapping runs and hide reds (b8ab1dd24 showed "cancelled").
7. **App updates/daemon restarts** interrupted 4 CI waits and killed 1 gate; two interrupted CI waits then reported `timeout` within minutes.

## 1. Timeline

Stage minutes are wall time on the lane. "x" = Failed group. Fold-conflict-only groups are collapsed at the end.

| Group | Members (at gate) | base | fold | gate | push | ci | Outcome |
| --- | --- | --- | --- | --- | --- | --- | --- |
| G475 | (Nov 12 group) | - | - | 46 (pass) | - | 105 Rejected | x CI red (Nov 12 run 0732a8c29/9600cbee1 era) |
| G580 | ov-1094 + 6 | - | 4 conflicts (ov-1094) | - | - | - | x fold |
| G643 | ov-1040, ov-1048, ov-1092 | - | 2 + conflict (ov-1085) | 53 Rejected (swift-tests, 1 issue) | - | - | then base **180 timeout** (main had no green from 17:14Z to 23:51Z Nov 12) x |
| G729 | ov-1120, ov-1113, ov-1116 | 51 | conflict (ov-1114) + pass | 49 | 0.2 | 15 Interrupted, then `timeout` 0.0 | x (pushed, but group Failed) |
| G793, G795, G797, G802, G815, G885 | queued since 05:5x | **timeout in 0.1 each at 08:56** | - | - | - | - | x all six |
| G979 | ov-1074 | 0.1 | conflict (7 min fold) | - | - | - | x |
| G980 | ov-1077, ov-1092 | 0.1 | 7 | 71 | 0.4 | 93 | landed 13:54 |
| G982 | ov-1096 (after 3 conflict halvings dropping ov-1105) | 0.1 | 0.3 | 45 | 0.2 | 87 | landed 16:07 |
| G986, G1006 | ov-1126 + 9; ov-1085, ov-1071 | 0.1 | 4 + 2 conflicts | - | - | - | x |
| G1069 | ov-1110 | 0.1 | conflict (ov-1108) + pass | 64 | 0.2 | 46 `cancelled` | pushed; group cancelled |
| G1073 | ov-1109 (docs) | 37 (waiting on 0b8da5e69 CI) | 2 conflicts + pass | 3 | 0.4 | 4 | landed 18:43 |
| G1095, G1103, G1160, G1162 | ov-1074..; ov-1071; ov-1108; ov-1105 | - | 8 conflicts | - | - | - | x |
| G1169 | ov-1129 | 0.1 | 3 conflicts (ov-1128) + pass | 44 | 0.2 | 28 | landed 19:57 |
| G1171 | ov-879, ov-895, ov-1134 | 0.1 | conflict (ov-1132) + pass | **38 Rej + 38 Rej + 50 pass** (peeled to ov-879) | 0.2 | 51 | landed 22:55 |
| G1173, G1244, G1247, G1263, G1265 | ov-1131.., ov-1085/1071, ov-1105/1108, ov-1128.., ov-1136/1098 | - | 13 conflicts | - | - | - | x |
| G1267 | ov-1130 | 0.1 | 0.1 | 44 | 0.2 | 62 | landed 00:43 |
| G1271, G1275, G1279, G1281, G1285 | ov-1071/1085, ov-1108/1105, ov-1074.., ov-1132, ov-1098 | - | 11 conflicts | - | - | - | x |
| G1326 | ov-1085 | 0.1 | 0.0 | **36 killed (SafeFailure, daemon restart 01:22)** + 42 pass | 0.2 | 60 | landed 03:03 |
| G1330, G1334, G1336 | ov-1105/1107, ov-895+8, ov-1131 | - | 7 conflicts | - | - | - | x |
| G1338 | ov-1134 | 0.1 | 0.0 | 61 (rust 27 min under load 42) | 0.3 | 4 Interrupted (restart 04:10) + 12 `timeout` | x (pushed 884a383a5; its run later failed a flaky Rust ubuntu test) |
| G1402 | ov-1071 | 45 (waiting on 884a383a5) | 0.0 | 39 | 0.3 | 31 | landed 06:18 |
| G1425, G1429 | ov-1107; ov-1074 + 12 | - | 5 conflicts (ov-1098) | - | - | - | x |
| G1431 | ov-1117, ov-1119 | 0.2 | 0.2 | 57 | 0.2 | 47 Interrupted (restart 08:02) + pass | landed 08:03 |
| G1433 | ov-1132 + 4 | - | 4 conflicts (ov-1132) | - | - | - | x |
| G1524 | ov-1144 (after 3 halvings dropping ov-1145) | 0.1 | 0.1 | 40 | 0.3 | 10 Interrupted (restart 08:55) + 24 **Rejected** | x pushed b8ab1dd24, Web job red (stale csp-check copy from ov-895), run 43536261416 |
| G1526 | ov-1143 (after 2 conflicts on ov-1147) | 0.1 | 0.1 | 36 Rejected (`ModuleNotFoundError: process_guard`, a real bug) | - | - | x |
| G1528 | - | **timeout in 0.1 at 09:56** | - | - | - | - | x |
| G1567 | ov-1150, ov-1151 | 34 (waiting on e21474271) | 0.0 | **16 Rejected: its tree was deleted mid-gate** | - | - | bisected to ov-1150 alone (running) |

Landed on main in the window: ov-1120, ov-1113, ov-1116 (G729), ov-1077, ov-1092, ov-1096, ov-1110, ov-1109, ov-1129, ov-879, ov-1130, ov-1085, ov-1134, ov-1071, ov-1117, ov-1119, ov-1144 = **17 tickets**, plus the hand-landed ov-1154 fix (e21474271).

### Totals by cause (lane minutes)

| Cause | min | Evidence |
| --- | --- | --- |
| Useful gates (12 passes, median 45 min) | ~605 | gate tables: rust 11-31, swift-tests 6-12, android 6-14, swift 3-6, wire 3 min |
| Gate re-run because peel-bisect (G1171 x2) | 77 | shared-core-lint failure; culprit found only on the 3rd gate |
| Gate killed by app update (G1326) | 36 | farcoolerd "shutting down" 00:55Z / gate SafeFailure 01:22 |
| Gate deleted by prune chore (G1567) | 16 | `.output`: "could not find Cargo.toml in .../tree-wf1-9dd3692f..."; scratchpad `classified.json` 10:41 marks `tree-wf1-9dd3692f…` (short daf3793b) `keep: false` |
| Gates that caught real bugs (G643, G1526) | 89 | swift-tests 1 issue; process_guard import |
| CI wait after push, landed groups | ~470 | 4-93 min, median 51 |
| CI wait that ended Failed (G475, G729, G1069, G1338, G1524) | ~210 | Rejected/Interrupted/timeout/cancelled |
| `base` waiting for main green | ~355 | G643 180 timeout, G729 51, G1073 37, G1402 45, G1567 34 |
| Fold (incl. 75 conflict attempts, seconds each) | 25 | cost is ticket failures, not minutes |

Hidden cost not in the lane total: every Failed group sends its tickets back to `build` (join back-edge limit 1). From fold conflicts and base timeouts, about **35 ticket-rejections** in 35 h (16 distinct conflicting tickets plus their innocent co-members, plus ~15 tickets in the 7 base-timeout groups). Each costs a builder rebuild (often a fresh key, ov-1142..1153) and a fresh review, i.e. hours of agent time and another trip through the queue.

## 2. Contributing factors, with evidence

### F1. One group at a time, end to end (serial lane)
- The `landing` group has `exclusive: wf1:56ecfe…` (definition `/groups/landing/exclusive`), so exactly one group holds the lane from `base` through `ci`.
- In the window the lane was never idle: 31.4 of ~32 h are accounted for by stages. Groups G980 → G982 → G1069 → G1073 … start the minute the prior group's CI passes (e.g. G982 ci ends 16:07, G986 base starts 16:07).
- While a group sits in `ci` (11.3 h total), the gate machine does nothing landing-related. While a group gates (13.7 h), GitHub does nothing for landing.
- **Pipelining is safe here**: the gate already proves "main + group" builds and tests; CI on the pushed commit re-proves it. Gating group N+1 on top of N's pushed SHA (a speculative stack, as merge queues do) is only wrong if N's CI fails, in which case N+1's result is discarded and it re-gates on the new main. Expected effect: per-group latency drops from gate+CI (~95 min) to max(gate, CI) (~50 min) → about **2x throughput, ~8 lane-hours/day back**.

### F2. Unknown main stalls all groups; wait deadlines are stale
- `base` = AwaitBase "green", poll 60 s, timeout 10,800,000 ms (3 h), `routes.timeout: Fail`.
- What "green" means in code: a Passed **or Failed** run on main's head ends the wait. Red is only noted (`red:true`, coding_verbs.rs:190-195, ov-1076), so red main does not block. A run that is pending, missing, or Superseded/cancelled is `Unknown` (coding_verbs.rs:545-550) and keeps the wait going until the timeout. **So `cancel-in-progress` is what turns busy periods into base stalls.**
- G643 waited exactly 180.0 min (20:00Z → 23:00Z Nov 12). In that span the head's run kept being cancelled by the next push (9f6cbfb39 after 133 min, be90a63da, 0732a8c29: all "CANCELLED (superseded)"), and the 9600cbee1 run took 99 min in the mac queue.
- G793, G795, G797, G802, G815, G885 each hit `timeout` **0.1 min** after their `base` stage began (08:56-08:57 Nov 13), and G1528 did the same at 09:56 Nov 14. In code the deadline is `effect.created_ms + patience` (coding_verbs.rs:185 base, :241 ci). Here `created_ms = item.wait_since.get_or_insert(now)` (reducer.rs:592-594). Two bugs make that stale:
  - **Split halves clone the parent group, `wait_since` included** (reducer.rs:415-428 does not reset it). A half spawned by a split and queued for 3 h times out on its first poll. It fails and frees the resource, and the next half does the same: a cascade lasting seconds. The six Nov 13 groups were created in the G580/G643 split storm, which fits.
  - **One sweep uses one `now_ms`** (workflow_runtime.rs:781, 405, 541). Steps entered during a long sweep are stamped with the sweep's start time. After an `Interrupted` (app update), the relaunch keeps the old `wait_since` (reducer.rs:228-238). G729 and G1338 recorded `ci` timeouts 0-12 min after an interrupt, though their CI was still running.
- The engine-code facts here come from a read of origin/main, not a run.
- The lessons file (Nov 13) records the result: "the groups failed and handed all 15 reviewed tickets back to build".
- b8ab1dd24 (G1524, ov-1144) carried ov-895's stale csp-check expectation; Web failed at 00:53Z, the run was then cancelled by the hand-pushed fix's run, and main read "cancelled" instead of red. Push→next green push 1 h 02 min, until green CI ~1 h 45 min.
- A fix for red main can land through the engine, because base passes on red. But it waits in arrival order behind every queued group, and each of those gates and pushes onto red main first. G1526 gated onto red main at 09:20. The orchestrator hand-pushed e21474271 (ov-1154) to jump the queue.
- Concurrency in code: a group holds the `landing` resource in every running phase, including the whole CI wait (`owns_resource`, reducer.rs:524-527; `ready`, :548-552). There is no pipelining anywhere.

### F3. Fold conflicts: halving instead of ejecting, cherry-pick replay, join limit 1
- `fold` replays each member with `git rev-list main..candidate` and `cherry-pick -x` commit by commit (receipt `wf1:00f82949….git.jsonl`: 8 picks for ov-1096 then a failing pick for ov-1105). Replaying intermediate commits conflicts more often than a 3-way merge of the member's final tree.
- On conflict the receipt names the member and files (`detail: {member: "ov-1105", files: [7 engine files]}`). The route is still `conflict: Split{step: base, otherwise: Fail}` with back-edge limit 3. The split halves by position (reducer.rs:697-705): the first ceil(n/2) stay, the rest become a new group, and each half re-runs base. The kept **first half** which usually still contains the culprit (the culprit is at index 0 or 1 in 60 of 75 conflicts). Sizes go 13→7→4→2 (G1429), 11→6→3→2 (G1095), 10→5→3→2 (G986), and when the split budget runs out the whole remainder fails, including innocent members (e.g. ov-1074 failed with ov-1098 in G1429; ov-1129 survived G1169 only on the 4th try).
- 75 conflict receipts, 24 groups Failed at fold, 16 distinct conflicting tickets: ov-1071, 1074, 1085, 1094, 1098, 1105, 1107, 1108, 1114, 1126, 1128, 1131, 1132, 1136, 1145, 1147.
- Hot files (conflict count): `crates/workflow/src/reducer.rs` 18, `crates/store/src/workflow_overlay/tests.rs` 16, `workflow_runtime/coding_verbs.rs` 15, `workflow_starter.rs` 15, `crates/workflow/tests/groups.rs` 15, `crates/workflow/src/state.rs` 11, `workflow_runtime/wire_commands.rs` 11. These are the engine tickets the orchestrator ran in parallel.
- Ticket `join` has `limit 1, rejected → build`: a second conflict fails the ticket for good (ov-1148 pending), which is why keys ov-1142..1153 exist.
- Groups are huge (limit 32; 9-13 members seen) and members are in arrival order, so one stale engine ticket poisons a big group.

### F4. The local gate re-runs CI, slowly
- `scripts/landing-gate` → `lane-gate.py` "reads the job list out of ci.yml" and runs the same jobs: `cargo clippy` x8, full `cargo nextest --workspace`, `cargo test --doc`, `apps/macos/test.sh`, `build-app.sh`, Android `gradlew testInstrumentedUnitTest/assembleRelease`, `build-ios-frameworks.sh`, xcodebuild. Only toolchain installs and the iOS UI smoke are skipped.
- Passed-gate wall median **45 min** (36-71). Per job (minutes, median of 12): rust 13, swift-tests 8, android 7, swift 4, wire 3, ios 1-2. Rust ranged 10-31 min, inflated by load (G1338 rust shadow 1,642 s at load 42).
- Engine env: `CARGO_BUILD_JOBS=4`, `HEAVY_LOAD_LIMIT=20` on a 10-core machine; outputs show "waiting: load average 30.1/42.0/46.4/53.7 is 20 or more" in 9 of 18 gates, because builder lanes compile at the same time.
- GitHub CI then runs the same jobs on the pushed SHA (Rust mac 30-56 min critical path). The gate's unique value over CI is "know before main moves"; with a speculative stack (F1) the gate can shrink to a fast pre-check (clippy, targeted nextest via the existing rust-shadow selection, wire lints, doc-comments: ~10-15 min) and let CI be the full proof.
- `scripts/ci-changes.py` has no allowlist entry for `reports/**` or `.claude/agent/reports/**`, so a report file alone widens a narrow change to every family (G1402's gate plan: "tunnel: .claude/agent/reports/ov-1071/report.md…", "ios: …report.md").

### F5. Bisect cost and false rejections
- G1171: rejected twice (38 + 38 min) peeling ov-1134 then ov-895 before ov-879 alone passed. The failing step was `scripts/shared-core-lint.py`, which takes 1 s and names the offending file: attributable without any re-gate.
- G1326: gate killed by the 00:55Z app update (`farcoolerd: shutting down`), recorded SafeFailure after 36 min and re-run from scratch (42 min).
- G1567: the ov-943 prune chore (Haiku, 10:41-10:42) listed worktrees with `farcooler worktree list --json`, which includes the engine's `workflow/tree-wf1-9dd3692f…` (task "tree wf1 …", short daf3793b), classified it `keep: false`, and removed it with `farcooler worktree remove`. The running gate lost its tree: nextest failed, then every later step "working directory … does not exist". The engine read it as a code rejection and bisected ov-1150 + ov-1151. **ov-1150/1151 are not proven bad.**
- G1526 and G643 were real failures; G1526 (`ModuleNotFoundError: process_guard`) would have failed any lane-level `scripts/` self-test before review.

### F6. GitHub CI duration, mac slots, `cancel-in-progress`
- 22 main push runs (Nov 12 06:59Z → Nov 14 01:47Z). Green run wall median 46.9 min, p90 92.3. Critical path is a macOS job in 17 of 22 runs, usually Rust (macos-latest) 30-56 min.
- mac queue wait: median 0.5 min, p90 41 min, max 118 (9f6cbfb39). Waits cluster when runs overlap or a branch Nightly holds the 5 slots (reports/ov-1134). Each CI run asks for 4-5 mac slots at once, so two overlapping runs serialize.
- `concurrency: ci-${{ github.ref }}, cancel-in-progress: true` cancelled 4 runs (9f6cbfb39, be90a63da, 0732a8c29, b8ab1dd24); 3 never produced a verdict on their commit, and b8ab1dd24's red was masked as "cancelled". With a pipelined landing (F1) cancel-in-progress must be off for main, or every speculative push cancels its predecessor's proof.
- Flaky: 884a383a5 (run 43498440945) Rust ubuntu failed `coding_native::recovery_tests::workflow_native_review_vanished_server_…` with tmux/farcoolerd leaks; next push green. be90a63da Rust mac 5 failures after a 36 min queue (load-related).

### F7. App updates and daemon restarts interrupt steps
- farcoolerd restarted at 17:22Z, 20:10Z Nov 13 and 00:02Z, 00:55Z Nov 14 (= 01:22, 04:10, 08:02, 08:55 local), plus an earlier one at ~08:55 Nov 13.
- Effects: G1326 gate killed (36 min lost); CI waits Interrupted for G729, G1338, G1431, G1524. After the interruption, G729 and G1338 recorded `timeout` 0.0-12 min later (CI was still running), failing pushed groups whose commits were in fact on main.
- Every restart re-reads ~5,000 receipts; the daemon log carries ~7,100 "invalid recovery receipt" warnings in 2 days (3,502 "No such file", 2,719 "interrupted native output unavailable", 475 "worktree has changes", 332 "unknown field `findings`").

### F8. Disk and build dirs
- Free space now 13 GB (of 926). `worktrees/` is 90 GB; two stale ticket trees hold 16 GB each (ov-1142-waiting-groups-rebased, ov-1154-engine-stack); lessons note ~150 GB of builder `CARGO_TARGET_DIR` under /private/tmp and a 31 GB engine build dir.
- Disk pressure drove ad-hoc cleanups: the Nov 13 "clear every target/" freed 96 GB but caused four cold builds at once (load 56), and today's prune deleted a live gate tree (F5). Cold gate builds are a large part of the rust/swift minutes.

### F9. Runner CLI "stopped answering"
- `workflow get --json` is 695 KB (166 live items, 83 archived, 54 group items, full definitions repeated per group). Combined with load 30-50 and `git status` calls that the daemon kills on timeout (11 "git timed out and was killed" in 2 days, several on `status --untracked-files=all` in 16 GB worktrees), CLI calls time out.
- From the code: "the runner stopped answering" is the CLI's text for any client error (cli tasks.rs:1985), including an oversized reply. The envelope cap is 1 MiB (protocol lib.rs:1184), and `workflow get` returns state, configuration, notices, deliveries and budgets in one reply (wire_commands.rs:52-69), so its growth heads toward that cap. Every workflow RPC goes through the single runtime actor (workflow_service.rs:33), which also polls every running effect. Synchronous work on that task stalls RPC: `remove_dir_all` in fold (coding_verbs.rs:338), `sync_all`, and per-message `prune_trees` git calls (workflow_runtime.rs:507-515, 10 s cap each). Which of these caused a given stall is unconfirmed.

### F10. Grouping policy and orchestrator behavior
- Group limit 32 and arrival-order membership produce 9-13 member groups that mix engine tickets editing the same files; the orchestrator also ran several engine tickets touching `reducer.rs`/`state.rs`/`coding_verbs.rs` in parallel (lessons 2026-11-14).
- Orchestrator interventions added cost: leave/re-queue collapses that hit real conflicts and failed 4 tickets (lessons, Nov 13 later), cancelling G1069's CI wait (46 min, its commit was green), and a prune chore that deletes engine trees.

## 3. Fixes, ranked

Hours are lane-hours per day at today's arrival rate (~12-15 tickets/day), estimated from this window. "Check" is what keeps the fix honest; per the repo's rule, each check must be broken once and seen red.

### Quick config / policy changes (no engine code)

| # | Change | Files | Saves | Risk | Check |
| --- | --- | --- | --- | --- | --- |
| Q1 | Stop the prune chore from touching engine trees: skip any worktree under `workflow/` or whose task starts "tree wf1"; and stop disk sweeps from deleting `workflow/build-wf1-*` while a group is in `gate-0` | orchestrator skill / `.claude/agent/lessons.md`, the ov-943 brief | 0.5 h + avoids false bisects | none | prune dry-run on a list that includes a `tree-wf1-*` must keep it |
| Q2 | Hold app updates while any group is in `gate-0` or `ci` (or only update between groups) | orchestrator `app update` routine (auto-update-the-app memory) | ~1 h/day (killed gates, broken CI waits) | slower updates | `app update` refuses with a named group when one is gating |
| Q3 | Raise `base` timeout to 12 h and route `timeout → Step base` instead of `Fail` (until E2 lands) | landing definition, `/groups/landing/steps/0` (`arguments[3]`, `routes.timeout`) | prevents 7-group mass failures (~15 tickets rework) | a long red main just waits | workflow get shows the new route; a simulated red main leaves groups Waiting, not Failed |
| Q4 | Small groups, sorted by file overlap: limit 3-4, and never put two tickets that touch `crates/workflow/src/{reducer,state}.rs`, `workflow_runtime/coding_*.rs` or `wire_commands.rs` in one group | landing definition `/groups/landing/limit`; manager skill grouping guidance | fewer conflicts and smaller blast radius | more groups (fine once pipelined) | fold-conflict receipts per day trend down |
| Q5 | Serialize or stack engine tickets that edit the hot files before building them (planning rule) | manager skill / lessons | most of the 16 conflicting tickets | less parallelism on engine work | count of `conflict` receipts naming those files |
| Q6 | Give each main run a distinct concurrency group using `github.run_id`, and set `cancel-in-progress: ${{ github.ref != 'refs/heads/main' }}`. Disabling cancellation alone still allows a newer pending run to replace an older pending run. **Highest-value quick fix**: base treats a cancelled run as Unknown and waits, so supersession caused G643's 3 h stall | `.github/workflows/ci.yml` concurrency | 1-3 h per busy period; no more masked reds from supersession (b8ab1dd24) | more runner minutes, more mac queueing (pair with Q9) | three overlapping main pushes retain distinct groups; branch runs still cancel |
| Q7 | Raise `CARGO_BUILD_JOBS` to 8 for the gate and cap builder lanes instead (or gate at nice 0, lanes at nice 10) | engine config `adapter.config.checks[0].environment`; lane briefs | 10-15 min/gate (rust 11-31 min under load) | lanes slower | gate rust-step seconds in the summary table, median per day |
| Q8 | Add `reports/**` and `.claude/agent/reports/**` to the docs allowlist | `scripts/ci-changes.py` (SITE_ONLY / docs patterns + self-test) | minor (keeps narrow plans narrow) | a test reading reports/ would be skipped (none found) | self-test case "report-only change runs only cheap jobs" |
| Q9 | Keep branch Nightly dispatches off the 5 mac slots while landing is busy | orchestrator; `nightly.yml` concurrency group shared with CI's mac jobs | 15-70 min CI queue on overlap | slower nightlies | mac queue p90 on main CI |

### Engine code changes

| # | Change | Files (origin/main) | Saves | Risk | Check |
| --- | --- | --- | --- | --- | --- |
| E1 | **Eject the named conflicting member** instead of halving: on `conflict`, drop `detail.member` from the group (send it to its own rebase/`join` rework) and re-fold the rest immediately | `crates/workflow/src/` reducer Split handling; `crates/daemon/src/workflow_runtime/coding_fold*.rs`/`coding_execute.rs` | innocent tickets stop failing; ~2-4 h/day of rework | low | test: a 5-member group whose member 2 conflicts lands 4 members in one fold |
| E2 | **Fresh wait clocks and a fix-main lane**: reset `wait_since`/`since_ms` on split-spawned groups (reducer.rs:415-428); stamp Observe with real time, not the sweep's `now_ms` (workflow_runtime.rs:405/541/781); on an interrupted wait, keep the deadline but measure it from the stage's real start. Also let a group marked `fixes-main` jump the queue | `crates/workflow/src/reducer.rs`, `crates/daemon/src/workflow_runtime.rs`, `coding_verbs.rs:185/241` | the 7 instant timeouts (~15 tickets of rework); red-main hours (~1-2 h per incident) | a queue jump lands an unreviewed fix (it is still reviewed and gated) | test: a half spawned by a split, queued 4 h, still gets a full wait; interrupt then CI green → Done; a fix-main group lands ahead of 3 queued groups |
| E3 | **Speculative stack (merge-queue style)**: allow group N+1 to fold+gate on top of N's pushed SHA while N is in `ci`; push N+1 only after N's CI is green; on N's CI red, discard N+1's gate and restart it on the new base | `crates/workflow/src/templates.rs:159-166` (release the resource after push), `reducer.rs` `owns_resource`/`ready` (:524-552), `coding_verbs.rs:182` await_base (accept the predecessor's pushed candidate as base) | ~8 h/day (per-group 95 → ~50 min) | needs Q6; a red N costs N+1 one gate | test: two groups, N's CI pending, N+1 reaches `push` ready; N red → N+1 re-bases; e2e timing in a scratch daemon |
| E4 | **Merge, not cherry-pick-replay**, in fold: `git merge --no-ff` (or squash the member's net diff) onto the fold head; fall back to replay only for linear history needs | `coding_fold` in `crates/daemon/src/workflow_runtime/` | fewer conflicts (intermediate-commit conflicts vanish) | history shape on main changes (no merge commits on main rule → use squash per member) | test: a member whose 2nd commit reverts its 1st folds cleanly onto a main that touched those lines |
| E5 | **Attribute gate failures before bisecting**: map the failing step's files/tests to members (doc-comment/lint steps name files; nextest names crates) and eject the owner; true bisect (halves, both halves gated) only when unattributable | gate route handling in reducer + `coding_gate` evidence parse | 1-1.5 h per multi-member reject | wrong attribution ejects an innocent ticket (it rejoins) | test with G1171's shared-core-lint output |
| E6 | **Treat infrastructure failures as retry, not reject**: tree missing, `SafeFailure`, daemon restart, ENOSPC → re-run the stage, never Split | `coding_gate`, reducer route for `SafeFailure`/Interrupted | 0.5-1 h/day and no false bisects | masking a real failure (cap retries at 2) | test: delete the fold tree mid-gate → stage re-runs, no Split |
| E7 | **Resume, don't fail, interrupted CI waits**: after restart, AwaitCi re-polls the same SHA with the original deadline from stage start | `coding_await_ci` | the G729/G1338 false failures | none | test: interrupt during AwaitCi, CI later green → group Done |
| E8 | **Protect engine trees**: `farcooler worktree list` hides (or flags) `workflow/tree-wf1-*`; `worktree remove` refuses an engine-owned tree while a stage runs | `crates/daemon` worktree list/remove handlers (reconcile already knows `engine_owned`) | prevents G1567-type loss | none | CLI test: remove on an engine tree errors with a plain message |
| E9 | **Thin gate**: when E3 is in, the gate runs only clippy, targeted nextest (rust-shadow selection), wire + doc-comments, and the changed family's build; CI is the full proof | `scripts/landing-gate`, `scripts/lane-gate.py` (`--landing` profile) | 25-30 min/gate (~6 h/day) | a bug CI would catch reaches main and needs a revert/forward-fix; pipelining makes that one group | weekly count of main reds that the full gate would have caught |
| E10 | **Smaller `workflow get`** and CLI timeouts: summary by default (`--full` for definitions), prune archived groups, don't run `git status --untracked-files=all` on the RPC path | daemon workflow wire commands; `crates/cli` | CLI responsiveness | none | `workflow get` p95 latency under load 30 < 2 s |
| E11 | **Fix the join cascade** (ov-1148): auto-rebase a conflicting ticket onto main and re-join without spending the back-edge when the rebase is clean | ticket definition `join` route; `coding_worktree` | the ov-1142..1153 re-key churn | none | test: second conflict with clean rebase re-joins |
| E12 | **Disk budget the engine enforces**: one shared engine target dir with a size cap and LRU eviction instead of per-tree targets; refuse to start a gate under 25 GB free with a plain notice | `workflow_runtime/fold_trees.rs`, landing-gate `--keep-target` | removes ad-hoc sweeps (and their cold builds) | eviction thrash | daily free-space min; gate cold-build count |

## 4. Proposed tickets (in order)

Titles follow "<Area>: <outcome>". **Parallel lanes**: T1, T2, T3 (policy/config) at once today; then T4, T5, T6 in parallel (different files: fold vs await adapters vs worktree CLI); T7 (stack) after T5; T8 after T7; T9-T11 in parallel any time.

1. **Landing: prune never deletes engine trees** (policy, today)
   - Intent: stop chores and disk sweeps from removing `workflow/tree-wf1-*` or `build-wf1-*` while the engine uses them.
   - Accept: prune brief/skill skips engine trees; a dry run over a list that contains `tree-wf1-*` keeps it; G1567's ov-1150/1151 re-gated on an intact tree.
   - Constraints: no engine code; Haiku chore.
2. **Landing: hold app updates during gate or CI** (policy, today)
   - Intent: no update while a group is in `gate-0` or `ci`.
   - Accept: the auto-update routine checks `workflow get` for a running gate/ci and waits; noted in the manager skill as a default.
3. **Landing: base waits, never fails on timeout** (definition, today)
   - Intent: stop mass group failures on a long red main.
   - Accept: `/groups/landing/steps/0` timeout 12 h and `routes.timeout → Step base`; group limit 4; `ci.yml` uses distinct groups for main runs and has `cancel-in-progress` off for main; `workflow get` shows the definition changes.
   - Constraints: definition change via the reviewed owner path; ci.yml change through a normal lane.
4. **Engine: eject the conflicting member at fold** (code)
   - Intent: a conflict removes `detail.member` and re-folds the rest at once.
   - Accept: reducer test: 5 members, member 2 conflicts → 4 fold and gate together; ejected ticket goes to rebase, keeps its key.
   - Constraints: pairs with ov-1148 (join cascade); don't change history shape.
5. **Engine: wait deadlines start fresh** (code)
   - Intent: split-spawned groups reset `wait_since`, Observe uses real time instead of the sweep clock, and interrupted waits resume on the same SHA.
   - Accept: reducer test: a split half queued past the patience still gets a full `base` wait. Runtime test: interrupt during AwaitCi, CI later green → Done. Both tests fail on current main.
6. **Engine: infra failures retry, not split** (code)
   - Intent: missing tree, SafeFailure, ENOSPC, restart → retry stage (max 2), never Split.
   - Accept: deleting the tree mid-gate re-runs the gate; no bisect; `worktree remove` refuses engine trees.
7. **Engine: gate next group while CI runs** (code, after 5)
   - Intent: speculative stack; N+1 folds/gates on N's pushed SHA; pushes after N is green; restarts if N is red.
   - Accept: two-group scratch-daemon e2e shows N+1's gate overlapping N's CI; red N re-bases N+1; lane-hours per landed ticket measured before/after.
   - Constraints: needs T3's main concurrency changes; scratch daemons by PID, short FARCOOLER_HOME.
8. **Landing: thin gate once CI runs behind it** (scripts, after 7)
   - Intent: gate = clippy + targeted nextest + wire/doc-comments + changed family builds, ~15 min; CI stays full.
   - Accept: median gate under 20 min for a week; count of main reds the full gate would have caught (re-run full gate nightly on main as the canary).
9. **Engine: fold merges each member's net diff** (code, any time)
   - Intent: squash-apply each member's final diff (3-way) instead of replaying commits.
   - Accept: revert-inside-branch case folds cleanly; main stays linear.
10. **Landing: attribute gate failures to a member** (code, any time)
    - Intent: map failing step files/tests to members and eject; true halving bisect only when unattributable.
    - Accept: G1171's output ejects the right member on the first reject.
11. **Engine: lighter workflow get and disk cap** (code, any time)
    - Intent: summary `workflow get`, engine-owned shared target with a size cap, low-disk refusal with a plain notice.
    - Accept: `workflow get` < 2 s at load 30; free space never below 25 GB from engine builds.

Policy note for the orchestrator now: plan engine tickets that touch `reducer.rs`, `state.rs`, `coding_*.rs`, `wire_commands.rs` as a stack, not in parallel (Q5).
