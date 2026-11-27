# ov-1475 · Native build exit and detached processes

## Historical holds

The Nov 26 process table showed the five leaders as unreaped zombies. The earlier investigation recorded the daemon refusal “native agent left a process running outside its group.” On that version, `end()` returned `Unsettled` only after `waitid` saw the leader exit, a process-table read succeeded, the agent group was empty, and the hold still saw a live detached descendant. In `serve()`, that result released the lock and left the socket open for another check after 250 ms. The old runner turned the refusal into an Uncertain outcome before it wrote the turn.

| Build | Hold PID | Zombie leader | Descendants identified in the investigation |
| --- | ---: | ---: | --- |
| ov-1412 | 58699 | 58763 | Test holds and their shell loops |
| ov-1426 | 91468 | 91533 | Test tmux servers |
| ov-1434 | 5968 | 6056 | Test holds and tmux servers |
| ov-1435 | 45099 | 45207 | Cargo/nextest fixtures, Python helpers and a test hold |
| ov-1465 | 98600 | 98748 | Test holds and their shell loops |

The review found no live process naming any of these holds as responsible. A hold can still find a process by the unique ID it learned before reparenting. The descendant descriptions above are from the earlier investigation, which attributed processes using scratch directories, working directories and start times. Exact membership of each live hold's in-memory lineage set was unavailable.

The earlier report's SIGCHLD explanation is ruled out by the zombies. Ignoring SIGCHLD would have caused macOS to reap the leaders. Resetting the disposition still protects holds launched with SIGCHLD ignored.

## Scratch reproduction and settlement

On macOS 27.2.1 (26B131), the hold code from the ticket's main base at `1d4ef97b1` reproduced the same branch in scratch. Hold 79379 kept its socket with zombie leader 79380 and reparented `sleep` 79387 in its own group. The scratch fixture explicitly disclaimed responsibility, and the API returned 79387 for the sleep. The hold answered `end` with `unsettled`, then reaped the leader, removed the socket and exited 0 after the sleep finished.

The build regression uses an agent that exits 0 with a detached three-second sleep. With the same fixture on `origin/main` at `555682ae6`, the build failed with “the native session was lost without a receipt.” The updated runner observes the leader independently of the pane, saves the turn before teardown, reports the PID and command in workflow get, the ticket and Needs You, and passes the step once the sleep exits.

Resolve accepts a completed saved native build turn while the hold confirms the leader has exited and the group is empty. The active runner records that decision, then closes its pane and drains output before publishing the process fact. An interrupted run publishes that fact from the saved turn immediately. The hold continues accounting for the detached processes until they exit. The regressions exercise both paths with a process that waits for an explicit test release and verify that settlement precedes its exit. This path requires a saved turn and a hold that can report leader exit and group state.

## macOS responsibility

When the responsibility call returns the queried PID, the hold can still retain a descendant it learned through parent identities before reparenting, as the scratch reproduction shows. A descendant can escape observation if an intermediate parent exits between snapshots and the hold never learns the child's identity. The macOS regression forces self-responsibility and covers that gap with a detached sleep whose parent is gone before the first look.

## Landing verification

Rebased onto `29c41d4de`. Two conflicts came from upstream work on the same files. In `engine_stuck.rs`, ov-1469 replaced the stuck-build listing with startup and quarantine tracking. The leftover table now sits beside that code, unchanged. In `coding_native.rs` recovery, a quarantine refusal recorded before the runner stopped is still kept. A saved turn, when one exists, takes precedence over it. Upstream's ov-1482 had already moved background task polling into its own file, so this branch now moves only the native tests. Upstream's one test change, the quarantine receipt passed to `native_launch`, is carried into `coding_native_tests.rs`.

Afterwards `workflow_runtime.rs` has 1,437 lines and `coding_native.rs` has 818. The file-size, doc comment, daemon layer, plan layer and swallow checks pass, and clippy on the daemon tests is clean. Nextest ran 301 selected daemon tests, covering native, recovery, settlement, quarantine, resolve, Needs You and yielding. It also ran 24 CLI workflow tests. All passed except one, including all five ov-1475 regressions. Four delivery tests need the CLI binary, and they passed once it was built. The exception is `the_native_launch_refuses_the_quarantined_binary_behind_an_npm_codex_launcher`. It fails before the engine runs, because this Mac's `node` is an x86 binary that cannot start ("Bad CPU type in executable").
