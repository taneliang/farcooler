# ov-1308 fix round 1

The review finding required executing the bounded command, rather than replacing
its argv with the test agent. The existing follow-up commit `f329f11c7` does that:
the test disables `TEST_ARGV`, substitutes only the Codex executable, and launches
the generated shell through real tmux. The stand-in records its actual arguments
and supplies the received prompt to the interactive hook fixture. The test checks
the entire ticket and the pinned hooks configuration, and retains the assertion
that the window command is below 8,192 bytes.

This round also checks that argv contains the generated shell (allowing the
engine's `nice` wrapper), parses the received hooks and compares their registrations
with the pinned endpoint, and checks command reconstruction after the shell
consumes the brief file. Production behavior from the earlier commits is retained.

Validation runs use real tmux and local sockets with sandbox escalation. Build
jobs and test threads are limited to four; no formatter was run.

- `cargo test -p farcooler-daemon -j 4 --lib workflow_runtime::coding_native::tests:: -- --test-threads=4`: 23 passed.
- `cargo test -p farcooler-tmux -j 4 -- --test-threads=4`: 104 passed (79 unit tests, 25 live tmux tests).
- `PATH="/usr/bin:/bin:/usr/sbin:/sbin:$PATH" CARGO_BUILD_JOBS=4 /usr/bin/python3 scripts/gate-flaky.py run -- cargo nextest run -p farcooler-daemon --profile gate --test-threads 4 --no-fail-fast`: **passed**, 3,862 passed, 19 skipped, 768.822 seconds. No failures or retries.
- `python3 scripts/daemon-layers.py`: passed, 648 recorded edges in 55 layer pairs.
- `python3 scripts/file-size-budget.py`: passed, 2,168 files, 27 grandfathered.
- `PATH="/usr/bin:/bin:/usr/sbin:/sbin:$PATH" HEAVY_LOCK_HELD=1 /usr/bin/python3 scripts/lane-gate.py --self-test`: passed.
- `git diff --check`: passed.

The first gate self-test attempt selected an incompatible Intel Python from the
inherited PATH in fixture shebangs. Selecting system Python fixed the environment.
The first daemon run exposed an overly strict shell assertion added this round
and one native settlement failure. The assertion was corrected for the engine's
`nice` wrapper; all 23 native tests then passed together, including the settlement
test. A subsequent `cargo test -p farcooler-daemon -j 4 --no-fail-fast -- --test-threads=4`
ran every binary: its library had 3,306 passes and one failure in the existing
`a_run_gate_restarting_in_a_named_tree_keeps_a_persons_changes` test, with
`process start identity unavailable` during owner reaping. All integration
binaries passed. That test passed alone on rerun (3.53 seconds), and then the
entire crate passed under the repository's gate wrapper as recorded above,
without requiring its retry path. Both ov-1308 tests passed in that full run.

## Landing-gate return: Swift scratch shells

The reported failure was in `NativeOfferTests.theRealCLINamesTheRunningAgent`:
`terminal send` returned `resource not found`, before the test could observe its
fake Claude process. It was not a Rust build failure. The original scratch home
and daemon log had already been removed by test cleanup. The same create/send
sequence passed 20 times locally, so the exact transient failure was not reproduced.

Inspection found that Swift's scratch daemons did not receive Cargo's
`FARCOOLER_TEST_SHELL=/bin/sh`. They started the owner's login shell (fish on
this Mac), including personal configuration. `ScratchDaemon.isolate` now sets
the test shell and the stub-agent switch required for the CLI to preserve that
override. This applies to cleanup's CLI calls too. The real-CLI test still types
and detects its own fake Claude through the real shell, tmux, CLI and daemon.

Validation, with four build jobs and no formatter:

- `CARGO_BUILD_JOBS=4 apps/macos/build-test-native.sh`: passed.
- `CARGO_BUILD_JOBS=4 FARCOOLER_MAC_TEST_UNIFIED_RUST=1 scripts/heavy -- apps/macos/test.sh -j 4`:
  passed, 1,979 tests in the main Mac suite, including the previously failing
  real-CLI test (4.508 seconds). All other test targets passed too. Temp-file,
  preferences and tmux leak guards passed. Log: `/tmp/ov-1308-return-swift.log`.
- `cargo test -p farcooler-daemon -j 4 --config profile.dev.opt-level=1 --config profile.dev.package.farcooler-cli.opt-level=0 --config profile.dev.package.farcooler-daemon.opt-level=0 --lib ov_1308 -- --test-threads=4`:
  both ticket regressions passed in 3.26 seconds: the 20,000-character ticket
  starts through real tmux with a bounded command, and a window refusal reaches
  workflow get with tmux's reason. Log: `/tmp/ov-1308-return-engine.log`.
- `python3 scripts/daemon-layers.py`: passed, 648 edges in 55 layer pairs.
- `python3 scripts/file-size-budget.py`: passed, 2,168 files, 27 grandfathered.
- `HEAVY_LOCK_HELD=1 python3 scripts/lane-gate.py --self-test`: passed.
- `git diff --check`: passed.

Commands select system Python first in PATH; the inherited Python executable is
an incompatible Intel binary on this host. Engine implementation is unchanged;
the full daemon-crate pass recorded above remains applicable.
