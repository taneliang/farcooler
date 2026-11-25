# ov-1454 · One writer per terminal

The runner grants one writer lease for a terminal. A second client can keep
reading the stream. The runner returns a viewer state and a reason when the
lease is occupied. **Take Over** changes the holder and sends a private event
to the previous writer. The browser refreshes the runner's status while the
pane is visible, so the viewer state and takeover reason also survive a missed
notification.

The runner holds a per-terminal lock from input admission through delivery.
A lease generation fences queued input after a takeover. A browser releases
its lease when leaving a pane, and the runner frees leases when their owning
connection closes or is revoked. An input RPC without a generation claims a
vacant lease before delivery. This includes prompts, file pastes and keys
sent on behalf of a client. The Mac's terminal input process, native client
connection and CLI helpers share an app-instance identity. If no daemon is
listening, the stdio relay starts a detached runner. The runner holds the lock
and serves the shared socket while clients are connected. It exits after
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
