# ov-1162: oversized first-sync replies closed the control connection

The failing step is a first-sync read, rather than the SSH or protocol hello.
`serve_connection` passed each completed response to `Connection::send`.
`framing::encode` rejects an envelope larger than 1,048,576 bytes before
queuing any bytes. The serving loop treated that local encoding failure as
`Ending::Unanswerable`, closed the connection, and discarded subsequent
answers. The stdio relay propagated EOF. A phone's detached board read could
therefore kill the same control connection its next fleet refresh needed,
causing another connect and another oversized board read.

The workflow-size hypothesis alone does not explain the failure: a scratch
state of 768,200 bytes, carrying 516 approval-waiting runs with repeated
definitions, produced an 871,422-byte full workflow reply successfully on
the old code. Its following `task.list` closed that connection. A larger
workflow reply independently triggers the same defect.

## Evidence before the fix

- The existing canary log recorded repeated device sessions ending with
  `reason=protocol`, including `farcooler-862cf7a45f7a` at
  `2026-11-14T04:57:03.215586Z`. That word covers all codec errors; the
  existing INFO log does not establish the exact codec failure for every
  historical retry.
- A read-only `farcooler --json task list` failed with
  `error: the runner stopped answering`. Filtered reads succeeded; the Done
  subset alone contained 987 tasks and produced 1,433,751 bytes of JSON.
  JSON size is not protobuf envelope size. This is corroboration of the
  board-growth trigger, not a measurement of the rejected wire envelope.
- Both real-daemon stdio regressions failed on the original source with
  `oversized sync must refuse only its read, keeping the link: Err(Closed)`.
  One first synced the 768,200-byte state and then requested a board with
  1,200 one-KiB descriptions. The other read a 1,229,790-byte workflow state.
- The separate EOF-drain regression also failed on the original source:
  `the drain lost an answer`. Its large handler is released only when the
  server reads EOF, so this covers the drain path without a timing sleep.

## Change

Both normal response delivery and the EOF drain now use `send_response`.
Only an outgoing response that fails the size check is replaced, with the
same request ID and a bounded `OperationFailed` error:
`what=response_too_large`, `retryable=false`. There are no partial bytes to
undo: encoding checks the size before queuing a frame. Other errors still
close the link, incoming oversized frames are still refused, and the frame
cap, backpressure, scope and revocation rules are unchanged.

The daemon now logs a warning with a static response-kind word, the rejected
encoded byte count and the cap. No request or response contents are logged.
This makes the exact size failure visible at the normal retained log level.

The fix deliberately retains compatibility with installed clients. It adds
no wire fields and does not silently truncate board rows or workflow state.
An oversized read remains unavailable, but no longer disconnects the runner
or loses unrelated answers. Pagination or smaller read schemas are separate
data-access work; this change does not claim to display an oversized board.

## Isolation and ov-1159

All builds used this worktree's `target/`. Reproductions used short private
`/tmp/fc1162*` homes, stub agents, a real daemon and its actual
`--stdio --scope control` relay. Only the fixture's daemon/relay children and
the tmux server identified by its own install ID were reaped. No installed
daemon was stopped, restarted, upgraded or sent configuration mutations. No SSH or
Claude configuration was changed. The two CLI observations above and the
ticket read were read-only.

Inspected ov-1159 through branch `ov-1159-light-workflow-get` at
`3703f2847`: it makes workflow Get smaller and moves slow reads off the
effect inbox. It had not landed on this branch. This fix changes neither
Get nor workflow/runtime/RPC code, and remains useful for oversized boards
and legacy/full workflow readers after ov-1159. An attempted coordination
note to the orchestrator was blocked by automatic approval review; no
message was delivered.

## Acceptance and validation

- [x] Cause identified and reproduced: outgoing oversize encoding ended the
  session; the first board sync reproduces it with a today-sized workflow.
- [x] Real stdio/control regression failed before the fix, as did the
  oversized-workflow and EOF-drain cases.
- [x] Same scratch control connection remains answering in under five seconds
  after the first sync, with state larger than the ticket's reported size.

Initial fixed runs: 173.94 ms for the today-sized state plus rejected board
read and five subsequent fleet/version RPCs; 172.18 ms for an oversized
workflow and board followed by those same RPCs. Final runs completed in 298.53 ms and 451.68 ms respectively, each on the
same connection throughout. The scratch daemon's retained warnings identify
the rejected protobuf envelopes precisely:

```text
response exceeds frame cap; keeping connection response="task_list" bytes=1355061 cap=1048576
response exceeds frame cap; keeping connection response="workflow_reply" bytes=1394759 cap=1048576
```

The two scratch tests passed in 4.43 seconds including fixture setup and
both daemon startups. The existing stdio suite passed all 11 tests, and the
relayed-scope suite passed all three. File-size budget, swallow lint,
doc-comment and whitespace checks passed.

The first broader client run passed its library suite and most integrations,
but four real-daemon integration tests required a `target/debug/farcooler`
that had not been built. The local CLI was then built to satisfy that test
prerequisite; no installed binary was used or replaced. Final complete
suite and lint results:

- `env -u FARCOOLER_NATIVE_HOOK_SOCKET -u FARCOOLER_NATIVE_SESSION TMPDIR=/tmp CARGO_BUILD_JOBS=4 cargo test -p farcooler-client -p farcooler-transport --no-fail-fast --quiet`: **492 passed, 0 failed, 1 ignored**. This includes all 54 transport tests, the new EOF-drain regression, 351 client library tests and all client integration targets.
- `TMPDIR=/tmp CARGO_BUILD_JOBS=4 cargo test -p farcooler-daemon --test a_large_sync_keeps_the_remote_link -- --nocapture`: **2 passed**; the exact sizes and timings above are from this final run.
- Existing daemon stdio and relayed-scope targets: **14 passed**.
- `TMPDIR=/tmp CARGO_BUILD_JOBS=4 cargo clippy -p farcooler-transport --all-targets -p farcooler-daemon --test a_large_sync_keeps_the_remote_link -- -D warnings`: **passed**.
- File-size budget, swallow lint, doc-comment check and `git diff --check`: **passed**.

The first complete run after building the CLI still failed four permission-hook
fixtures: no Permission reached their scratch daemon. Investigation found
`crates/cli/src/hook.rs::run` prioritizes the inherited
`FARCOOLER_NATIVE_HOOK_SOCKET` over the fixture's explicit `--socket`. Removing
that inherited native hook binding (and its session ID) for the final run
made all four pass. No application or fixture source was changed for this
environment issue.

Disk pressure during validation was handled by removing only completed
incremental caches created by this task under this worktree's `target/`;
compiled libraries and binaries were retained. The commit hook initially
found an incompatible Python on the host PATH; putting `/usr/bin` first
let the existing hook run. Implementation and regressions are in signed
commit `5b6ed9506`; this report records final verification separately.

Status: DONE for the reconnect acceptance items. Oversized read availability
remains the explicit limitation described above; the live application was
not deployed or restarted.
