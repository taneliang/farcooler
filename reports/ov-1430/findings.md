# ov-1430 runner freeze and Developer Tools permission

During the Nov 23 incident, eight daemon threads were in `__sysctl`, and process-argument reads hung on about twenty processes that had yet to finish exec. The process table populated foreground arguments even for caller attribution and workflow recovery, which only need ancestry, groups, and terminal IDs. The incident sample implicates Gatekeeper and trust evaluation, although it did not capture a stack from `syspolicyd`.

## Developer Tools measurement

**Expected effect for ordinary runner launches: little or no improvement.** Every fresh probe, including quarantined probes, executed under the current settings. There was no consistent first-launch penalty or probe-attributed security log entry for the permission to explain. Apple describes Developer Tools as allowing the selected app to run software that would otherwise fail system security policy. That could matter for a tool whose exec is actually denied, depending on whether macOS attributes the launch to the app, daemon, or tmux. Whether granting the permission removes any checks or avoids the slowdown seen under Gatekeeper saturation still requires a paired test. The baseline does not support adding the permission to onboarding. [Apple Developer Tools settings](https://support.apple.com/en-gb/guide/mac-help/mchlc5fb7f9c/mac)

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
3. Restore the snapshot. In System Settings → Privacy & Security → Developer Tools, enable the scratch app, relaunch it and its daemon, and repeat with fresh probes. Repeat from the snapshot for the bundled daemon entry if macOS allows it to be added separately, and for both entries if applicable. Compare results by launch route to see which identity the permission affects and whether it changes assessment events, launch denials, or latency. Restore the original settings. [Apple Developer Tools settings](https://support.apple.com/en-gb/guide/mac-help/mchlc5fb7f9c/mac)

## Verification after rebase

- Rebased onto `fec026430afdd9c4e1cf99a6290e0dd5b683e8e0`. The runtime still checks the active publication before claiming an effect and applies restart pacing before claiming work eligible to start.
- Clippy passed with `--all-targets -- -D warnings` for `farcooler-daemon`, `farcooler-cli`, and `farcooler-client`.
- The Mac target and test bundle built. All 123 focused Mac tests passed across 14 suites, including the read that stayed stalled long enough for the app to report “Runner not responding.” The Mac test floor rose by one, with zero drift from the landing base.
- The first full nextest run passed 5,128 of 5,132 tests and skipped 23. Four daemon cases failed during Git or tmux setup or caller attribution. The leak guard found and reaped one scratch tmux server. All four cases passed when rerun serially. Six freeze regressions also passed in that run, covering the one-second workflow read, pacing six interrupted runs, bounded process reads during delivery, worker capacity and timeouts, and watchdog reporting.
- The standalone full nextest run passed 5,131 of 5,132 tests with six passes on retry. The only remaining failure was `a_dead_panes_reassigned_tty_cannot_end_another_native_pane`, where the probe after shutdown reported `TmuxUnavailable`. All four quiescence tests passed in the final serial run, including that case. Every enabled Rust test passed across the runs. Both full invocations exited nonzero.

The workflow-read regression stalls the unrelated pane's argument read until the assertion deadline. On the landing base, caller attribution called `proc_table::snapshot` synchronously, which read that pane's foreground arguments. With the injected stall on that path, the request would wait for the test to release the read after the deadline. Verification here ran the test on the rebased fix.
