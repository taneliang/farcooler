# ov-1454 · One writer per terminal

The runner grants one writer lease for a terminal. A second client can keep
reading the stream. The runner returns a viewer state and a reason when the
lease is occupied. **Take Over** changes the holder and sends a private event
to the previous writer. The browser refreshes the runner's status while the
pane is visible, so the viewer state and takeover reason also survive a missed
notification.

The runner holds a per-terminal lock from input admission through delivery.
A lease generation fences queued input after a takeover. A browser releases
its lease when leaving a pane, and the runner frees leases when the last
participating connection closes or is revoked. An input RPC without a
generation claims a vacant lease before delivery. This includes prompts, file
pastes and keys sent on behalf of a client. The Mac's terminal input process,
native client connection and CLI helpers share an app-instance identity. If no
daemon is listening, the stdio relay starts a detached runner. The runner
holds the lock and serves the shared socket while clients are connected. It exits after
five seconds with no connections.

The browser probes the lease RPC on its existing control connection. On an
older runner it retains legacy terminal behavior and leaves the Writer badge
hidden. An older Mac CLI with direct access to tmux needs to be updated to
participate in the lease.

## Failing first

| New regression | Observed before the change | Verified after the change |
| --- | --- | --- |
| `only_one_connection_can_type_into_a_terminal` | The second connection's `terminal.write` returned success on a live pane, where the test expected a conflict. | The runner refused competing input and resize, allowed the Mac's same-instance geometry helper, handed over explicitly, notified the former holder, and rejected stale input before it reached tmux. Closing the winner freed the lease. |
| `stdio_sessions_share_the_writer_when_the_runner_starts_without_a_daemon` | Two stdio processes opened separate leases for the same runtime directory. The second write returned success. | Both SSH sessions reached one runner, and the second write was refused with `writer_busy`. |
| Chromium, 320 px | Both tabs displayed Writer. The wait for the viewer reason timed out after 2.5 s. | Viewer input stayed off the wire. The second tab acquired input via Take Over and the first displayed the takeover reason. |
| Chromium, 1280 px | Both tabs displayed Writer. The wait for the viewer reason timed out after 2.5 s. | The same handoff passed with both streams attached and the page within the viewport width. |

The Chromium cases run through the production writer controls and xterm against
a stateful test fixture. The Unix-socket regression exercises the production
runner and a live tmux pane. A separate smoke check exercised two real Mac
`terminal input` subprocesses, including same-app resize, viewer refusal,
the takeover notice on stdout and the bytes visible in the pane.

| State after handoff | 320 px | 1280 px |
| --- | --- | --- |
| Previous writer | [Viewer](viewer-320.png) | [Viewer](viewer-1280.png) |
| New writer | [Writer](writer-320.png) | [Writer](writer-1280.png) |

## Foreground verification

- Chromium work suite passed all 14 cases, including both viewports. The board browser check passed.
- The final runner test pass was green across 110 cases. This includes 88 live
  socket tests, 12 stdio tests, five forced-SSH tests, three relay-scope tests,
  and the daemon ownership and dispatch table checks.
- Both TypeScript checks passed. Web Vitest passed 255 cases, the Rust-backed
  web adapters passed 50 cases, and the workflow controller suite passed.
- The Rust web tests passed. The WASM target check and local web bundle build
  passed.
- The Rust CLI unit suite passed 639 tests, and the shared client unit suite
  passed 408. Core, protocol, and transport unit suites passed.
- The Mac app and test targets built. The Swift test run passed 2,019 tests
  across five test targets. The isolated Mac input smoke check passed.
- Protocol compatibility, capability names, file-size budget, shared-core
  placement, and copy lint passed.

The extra browser sleep check on the locally built bundle timed out during
passkey setup, before connecting to the fixture runner. Both two-client
browser regressions passed.

The broad native run recorded 3,394 daemon unit tests passing, 14 failures,
and 18 ignored. Failures included recovery locks, agent delivery and workflow
fixtures. The focused terminal and transport suites above passed independently.

The broad run was stopped after its daemon unit failure verdict. A targeted
retry of those 14 cases with the native macOS tools first on PATH passed 11.
Three still failed: a Codex queued-answer timing case and two workflow
script-hook recovery cases.

## Review follow-up

| New regression | Failure on `ed98d7134` | Result after the fix |
| --- | --- | --- |
| `every_terminal_input_route_claims_the_vacant_lease` | All eight routes left the lease vacant. Both clients successfully pasted a file into the same pane. The other requests reached their agent-specific delivery gates. | The first request through each route acquired the lease, and the competing client received `writer_busy`. |
| `a_stdio_writer_survives_the_starting_ssh_session` | The first SSH command exited and the surviving client's write failed with `Closed`. | The first SSH command exited, the second client wrote with its original generation, and a third client was refused with `writer_busy`. |

The foreground runner integration build passed. The socket, stdio, relay
scope, forced-SSH and agent transport suites passed 115 tests, with one
latency measurement ignored.

Another 94 integration tests passed across cutover recovery, real SSH scope
and revocation, relay exit, passkey support, daemon startup and log handling.
The final detached-runner build also passed all 13 stdio transport tests again.

The Rust WASM check, both TypeScript checks and all 14 Chromium work checks
passed again. The two-client cases exercised explicit takeover and viewer
feedback at 320 px and 1280 px.

The Mac app and test targets built. An isolated smoke check held a live
terminal lease through the CLI input process. The native FFI connection with
the same app identity reached the compose delivery gate. A different app
identity received `writer_busy`.

The focused Mac test run exercised 196 tests in the native and conversation
target. Two tests initially could not find a fixture in the scratch copy.
After linking the fixture directory, both passed in the five-test retry.
The other selected Mac test target passed its three tests.

The Rust CLI and shared client unit suites passed 639 and 408 tests,
respectively. All 54 transport unit and integration tests passed. The broad
live-client suite passed 30 of 34 tests. In the four failures, the shared
Claude permission fixture timed out before any permission event reached the
ring. A retry with the native system tools first on PATH had the same result.

The daemon executable passed all six unit tests. The complete daemon library
run reported 3,399 passed, nine failed and 18 ignored. All nine failures
passed on serial retry with native system tools first on PATH.

## Mac takeover notification follow-up

The runner tracks each live connection participating in the same app instance's
lease. They share one generation and each receives `TerminalWriterLost` on
takeover. Closing one connection leaves the lease with any remaining
participants.

| Regression | Failure on `38b02823d` | Result after the fix |
| --- | --- | --- |
| `takeover_notifies_native_input_after_ffi_claimed_the_lease` | `terminal.tell` acquired the lease through the FFI connection, and the Mac input connection joined that generation and wrote to the pane. After takeover, the FFI received the event and the input connection timed out waiting for it. | Both connections receive the displaced generation and report `writer_taken_over`. The test waits for the notification before any further input. |

The foreground integration run passed 122 tests across the live socket,
stdio, forced SSH, relay scope, revocation and agent transport suites. One
latency measurement was ignored.

All 14 Chromium work checks passed, including the two-client handoff at
320 px and 1280 px. The Chromium board check and both TypeScript checks passed.

The CLI and shared client unit suites passed 639 and 408 tests, respectively.
Each suite had one ignored test. The standalone CLI and daemon built, and all
six daemon executable unit tests passed.

The first daemon library run reported 3,337 passed, 71 failed and 18 ignored.
The scratch path exceeded the Unix socket length limit after native workflow
fixtures resolved the symlink. The failed cases were retried using macOS's
shorter runtime directory for fixture data, with build output kept in the
ticket scratch directory. Sixty-seven passed on that retry. The four remaining
tests passed on serial retry.

## Landing-base rebase

Rebased the three ov-1454 commits onto `ebc4b78674c9267dc69ad11f8137170afecfa9cc`.
Git applied them cleanly. The range comparison matched each rebased patch to
its original, so the failing-first observations above still cover this
candidate.

- All five live writer-lease regressions passed. The full socket and stdio
  suites passed 90 and 13 tests, respectively. The forced-SSH, relay scope,
  SSH revocation, agent transport and daemon ownership suites also passed.
- CLI, shared client and core unit suites passed 639, 408 and 773 tests.
  The live-client suite passed all 34 tests. The protocol and transport
  suites passed 73 and 54 tests.
- The focused daemon run for Engine, Needs You and the workflow modules named
  in the rebase passed all 130 tests.
- All 16 Chromium work checks passed, including the two-client handoff at
  320 px and 1280 px. The board and plan-render checks passed.
- The Chromium walk over real SSH passed with zero CSP violations. All seven
  Chromium idle, sleep and reconnect cases passed with the split terminal.
- Both TypeScript checks passed, along with 255 web unit tests, 50 Rust-backed
  web adapter tests, the workflow controller tests and 83 Rust web tests.
  The WASM target check and canary bundle build passed. The bundle measured
  5,019,260 compressed bytes and passed the size gate.
- The protocol, capability-name, file-size, shared-core and copy checks passed.
- The Mac app and test targets built. The five Swift targets ran 2,019 tests.
  One source-audit test failed because the scratch copy lacked its iOS source
  fixture. The suite passed all three tests on retry with the fixture linked.

The broad daemon library run reported 3,397 passed, 11 failed and 18 ignored.
Eight failures passed on serial retry. The native quiesce cases
`an_agent_that_ignores_sigterm_is_killed_after_its_grace` and
`an_owner_that_exits_while_the_runner_ends_it_is_quiescent` still returned
`TmuxUnavailable` when checking the closed pane. The workflow fixture
`script_hooks_timeout_and_no_receipt_retry_once_then_fall_back` still failed
its expected timeout and missing-receipt outcomes. The broad integration run
had one failure when the pane-resize fixture could not reacquire a PTY number.
All three pane-resize tests passed on serial retry.

The full-bundle smoke checks exposed a stale connection and dropped early
input. The browser now stops polling and releasing through a retired
connection. It buffers early keystrokes locally while the first claim is
pending, then sends them after a grant or drops them on refusal. The SSH
browser fixture returns the same unsupported response as an older runner.

| New regression | Observed before the fix | Verified after the fix |
| --- | --- | --- |
| `restore a writer after its runner connection is freed` | Chrome threw `null pointer passed to rust` in `TerminalWriter.show` when it queried the previous runner. | The writer badge returned on the fresh connection and typing reached the terminal. |
| `hold keystrokes while the initial writer claim is pending` | The writer badge appeared after the delayed grant, but the key pressed during the delay never reached the runner. | The key stayed local before the grant and reached the runner after approval. |

## Landing-gate repair

| Check | Observed before the repair | Result after the repair |
| --- | --- | --- |
| Daemon layers | The scanner reported `terminal_writers.rs` as unclaimed. | The module belongs to the access layer alongside `sessions`. The self-test, tree scan and no-growth comparison passed with 645 recorded edges across 55 layer pairs. |
| Visual tokens | The scanner reported one raw material in the Mac takeover banner. | The banner uses the shared content surface. The self-test and tree scan passed. |
| Clippy | The Mac input CLI used `is_multiple_of`, which Clippy rejected against the workspace's Rust 1.85 minimum. | The hex decoder uses a parity check compatible with Rust 1.85. Clippy passed with warnings denied for all targets in the CLI, client, daemon, protocol and transport crates. |

The foreground landing quick gate passed. All 90 live-socket tests and 13 stdio
tests passed. Both TypeScript checks and all 16 Chromium work checks passed,
including explicit takeover and previous-writer feedback at 320 px and 1280 px.
The CLI unit suite passed 639 tests with one ignored. The Mac app and test
targets built, and all 24 tiling and render-wiring tests passed on this host.
The full landing gate includes Wrangler build destinations under `/tmp`, so
this run used its quick phase and ran the affected builds and suites directly
in ticket scratch.
