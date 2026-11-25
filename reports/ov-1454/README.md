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
connection closes or is revoked. Older RPC clients acquire a vacant lease on
their first raw write. The updated Mac input process goes through the same RPC
and shares an app-instance identity with its resize and file-paste helpers.
If SSH starts the runner before a daemon is listening, the first stdio session
holds the runner lock and accepts later connections on the same socket for
the lifetime of that session.

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
