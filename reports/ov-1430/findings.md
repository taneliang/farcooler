# ov-1430 runner freeze and Developer Tools permission

During the Nov 23 incident, eight daemon threads were in `__sysctl`, and process-argument reads hung on about twenty processes that had yet to finish exec. The process table populated foreground arguments even for caller attribution and workflow recovery, which only need ancestry, groups, and terminal IDs. The incident sample implicates Gatekeeper and trust evaluation, although it did not capture a stack from `syspolicyd`.

## Developer Tools measurement

**The effect of granting Developer Tools on child-process Gatekeeper checks is still unknown.** Every fresh probe, including the quarantined probes, executed with assessments enabled. First-launch latency varied, and the available security logs attributed no entries to the probes. Apple describes Developer Tools as allowing the selected app to run software that would otherwise fail system security policy. The paired test below would establish whether macOS attributes each launch to the app, daemon, or tmux and whether the grant changes assessments, denials, or launch latency. The existing baseline leaves onboarding unchanged. [Apple Developer Tools settings](https://support.apple.com/en-gb/guide/mac-help/mchlc5fb7f9c/27/mac/27)

On macOS 27.2.1 (26B131), with `spctl --status` reporting assessments enabled, I started a copy of the bundled daemon against a SQLite backup of a freshly initialized scratch runtime. The daemon's tmux launcher invoked the benchmark directly and through a shell in a private tmux server. No system settings changed. For each route I compiled ten tiny C tools that each printed a different embedded random string, signed them with `codesign --force --sign -`, and verified 20 distinct cdhashes across both routes. Half of each route's tools were copied before execution and tagged with `com.apple.quarantine` (`0081;<timestamp>;ov1430;`). Each tool ran twice back-to-back by absolute path. The timer covered subprocess spawn through exit using a monotonic clock.

| Launch route | Quarantined | First exec median (range), ms | Repeat exec median (range), ms |
| --- | --- | ---: | ---: |
| Daemon launcher | No | 35.1 (8.0–102.2) | 32.5 (8.7–100.2) |
| Daemon launcher | Yes | 27.4 (23.5–51.1) | 30.8 (18.1–82.0) |
| Private tmux shell | No | 41.8 (21.1–102.0) | 55.8 (18.1–102.1) |
| Private tmux shell | Yes | 51.9 (9.0–75.6) | 27.0 (6.1–178.3) |

All 40 invocations printed their marker and exited zero. A Rust build was running concurrently, so elapsed times include local scheduling delays. The launch window (17:50:58–17:51:01 local time) contained 2 visible `syspolicyd` and 14 visible `trustd` entries in `log show --info --debug`. Zero entries named a probe or its cdhash. The `syspolicyd` entries concerned process provenance and the `trustd` entries had private fields, so those totals cannot establish how many assessments ran. After timing, explicit `spctl --assess --type execute` rejected one unquarantined probe and one quarantined probe (exit 3 each), even though both had executed successfully. That confirms the difference between explicit assessment and these bare command-line launches. The route used the scratch daemon directly, so app attribution and the behavior of downloaded, Developer ID-signed tools remain unmeasured. [Apple guidance on testing Gatekeeper](https://developer.apple.com/forums/thread/706379)

## Owner verification

1. Use a disposable macOS VM or a separate test Mac with Gatekeeper enabled. Install a scratch copy of the Far Cooler bundle, use a new runtime directory and a private tmux directory, and back up the seed database. Take a VM snapshot before the first launch of the probe tools.
2. With Developer Tools permission off, start the scratch app and daemon as they normally start. Run fresh probes through both a configured command and a tmux-backed workflow step. Include the plain and quarantined ad-hoc probes and a freshly downloaded, quarantined Developer ID-signed tool you trust. Time the first and second exec, record exit status and first output, and collect `log show --info --debug` for `syspolicyd` and `trustd` over the launch window. Record events identifiable by executable path, PID, or cdhash along with the overall counts.
3. Restore the snapshot. In System Settings → Privacy & Security → Developer Tools, enable the scratch app, relaunch it and its daemon, and repeat with fresh probes. Repeat from the snapshot for the bundled daemon entry if macOS allows it to be added separately, and for both entries if applicable. Compare results by launch route to see which identity the permission affects and whether it changes assessment events, launch denials, or latency. Restore the original settings. [Apple Developer Tools settings](https://support.apple.com/en-gb/guide/mac-help/mchlc5fb7f9c/27/mac/27)

## Verification after rebase

- Rebased onto `fec026430afdd9c4e1cf99a6290e0dd5b683e8e0`. The runtime still checks the active publication before claiming an effect and applies restart pacing before claiming work eligible to start.
- Clippy passed with `--all-targets -- -D warnings` for `farcooler-daemon`, `farcooler-cli`, and `farcooler-client`.
- The Mac target and test bundle built. All 123 focused Mac tests passed across 14 suites, including the read that stayed stalled long enough for the app to report “Runner not responding.” The Mac test floor rose by one, with zero drift from the landing base.
- The first full nextest run passed 5,128 of 5,132 tests and skipped 23. Four daemon cases failed during Git or tmux setup or caller attribution. The leak guard found and reaped one scratch tmux server. All four cases passed when rerun serially. Six freeze regressions also passed in that run, covering the one-second workflow read, pacing six interrupted runs, bounded process reads during delivery, worker capacity and timeouts, and watchdog reporting.
- The standalone full nextest run passed 5,131 of 5,132 tests with six passes on retry. The only remaining failure was `a_dead_panes_reassigned_tty_cannot_end_another_native_pane`, where the probe after shutdown reported `TmuxUnavailable`. All four quiescence tests passed in the final serial run, including that case. Every enabled Rust test passed across the runs. Both full invocations exited nonzero.

The workflow-read regression stalls the unrelated pane's argument read until the assertion deadline. On the landing base, caller attribution called `proc_table::snapshot` synchronously, which read that pane's foreground arguments. With the injected stall on that path, the request would wait for the test to release the read after the deadline. Verification here ran the test on the rebased fix.

## Verification on landing base `a07686688917ed48ff8384cc9d4d1a9207d453e7`

- The rebase kept the landing base’s Mac test floor of 1,937 and added one for the watchdog regression. The full CeremonyTests run counted 1,958 cases, above the resulting floor of 1,938.
- The Linux native shutdown check now uses the bounded process-state worker for its zombie probe. Clippy passed for the daemon, CLI and client with `--all-targets -- -D warnings`. The daemon layer check, its comparison with the landing base, and the file-size budget also passed.
- The first full Mac run exposed a timing race in the watchdog test and also reported Keychain, Engine-navigation and scrolling failures. The watchdog test now keeps the read open and waits for the published state. All 94 focused Mac tests passed after that change. A further run using the standard test CLI lookup passed all 11 Engine, scrolling and cancelled-read tests. The Keychain test reported that it could not write its test item. The test used a unique temporary item and no permission changes were made.
- Nextest ran 5,096 tests for the three touched Rust crates and skipped 23. It passed 5,095 cases, seven on retry. The remaining case compared a scratch `HOME` spelled `/private/tmp/...` with a path normalized to `/tmp/...`. It passed with the corrected spelling in the seven-case focused run, which also passed the workflow read, six-run restart pacing, bounded delivery and worker reads, and watchdog reporting checks. The leak guards passed.
- Temporarily restoring the old synchronous argument scan in caller attribution made the workflow-read regression fail at its one-second deadline. The test observed 1.003 seconds before releasing the blocked scan. With the bounded topology lookup restored, all 11 selected Rust tests passed, including the four native-shutdown cases and the one-second workflow read. The Linux-specific zombie test is outside the macOS test inventory.

## Verification on run base `ac9cd861634bb4d10afe0036ae6ee41e6c24511a`

- The branch rebased cleanly. Session membership, heavy-ticket process-group reads, existence probes, and the Linux check-in executable lookup now use the bounded workers. A timed-out probe leaves ownership or gate admission unproved.
- Clippy passed with `--all-targets -- -D warnings` for the daemon, CLI, and client. The daemon layer, base comparison, and file-size checks passed.
- The Mac app and test bundle built, and all 90 focused tests across 11 suites passed. The stalled-read regression observed “Runner not responding” while the original read was still open.
- Nextest ran 5,104 tests across the three touched Rust crates and skipped 23. The full invocation exited nonzero with 5,100 passed, including two on retry, and four fixture failures caused by Intel Python on the inherited PATH. All four passed in two successful reruns with native Python on a scratch-only PATH. Every enabled case passed across the runs, and the tmux leak checks reported no leaks.
- The full run passed the one-second workflow read with a blocked argument query, the six-run restart at two starts per minute, the bounded pane delivery and process-worker checks, all four native quiescence cases, and watchdog reporting. A temporary idle-sleep assertion kept the host awake after two long gaps and expired when nextest exited.

## Verification on landing base `5bfcd67e88252f76ed4c0a264e34866f375fe99a`

- The rebase kept native agent signaling in the hold. Legacy owner checks read topology and session membership through bounded workers. Heavy-queue admission uses bounded process-group and temporary-directory reads. A query timeout leaves admission unproved.
- Clippy passed with `--all-targets -- -D warnings` for the daemon, CLI, and client. The daemon layer check, its base comparison, the file-size budget, and the Swift test-floor check passed.
- The full nextest run passed all 5,129 enabled tests across the three crates, with 23 skipped. Seven cases passed on their first retry. The temp-file and tmux leak guards passed.
- Eight of nine serial checks passed with retries disabled. The native hold timing test failed its resume assertion and nextest reported a leak. No hold from the scratch binary remained after the run. The same test passed by itself with retries disabled and both leak guards passed.
- The workflow read met its one-second deadline while a process-argument query was blocked. The six-run restart honored the configured two starts per minute. The process-worker, pane-delivery, watchdog, native quiescence, and heavy-queue checks passed.
- The Mac app and test bundle built. All 82 focused Swift tests across 12 suites passed, including the stalled read that publishes “Runner not responding” before the read ends.
- The two web suites that consume the conversation-words fixture passed all 41 tests.

## Landing-gate repair

- The Mac now reads the runner timeout message from the shared client presenter. The shared-core lint and its self-test passed without a baseline change.
- The Mac Rust test selection now includes the process-read workers and pane delivery tests that exercise Darwin process queries. Its check and the quick landing gate passed.
- Clippy passed with `--all-targets -- -D warnings` for the daemon, CLI, and client. The shared phrases, chat, and attachment web suites passed all 43 tests.
- The shared words fixture passed through the linked Rust bridge in AgentKit. All 51 focused Mac tests passed, including the app publishing “Runner not responding” while the read was still open.
- The first nextest invocation inherited the workflow’s hook socket, which overrides the fixtures’ socket paths. I stopped that invocation and verified its private tmux namespace had no running servers or scratch daemons. The rerun cleared workflow environment variables before starting the test harness.
- The isolated nextest run passed all 5,129 enabled tests, with 23 skipped and two passing on retry. One retry followed host sleep. The workflow-read, six-run restart, process-worker, pane-delivery, and watchdog regressions passed. The temporary-file and tmux leak checks passed.
