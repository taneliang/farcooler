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
