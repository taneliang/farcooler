# ov-1486: the Engine view on the web

Engine is a new page in the web app, under the sidebar tools. It shows each
ticket's step and state, landing groups, and drafts waiting for approval. A
person can cancel, resolve a step, resolve a cancellation, settle a push, and
approve or decline a draft.

## One thing differs from the ticket

**A workspace's own orchestrator can still cancel and resolve.** The ticket
asks for the same requests to be refused from the orchestrator's session. The
runner allows these two today on purpose (ruling R-107, ov-999), so the
orchestrator can supervise the engine. This ticket changes no permission, and
taking those two away is the owner's call.
`ov1486_the_workspaces_own_orchestrator_keeps_the_cancel_that_r107_gives_it`
records the cancel as it stands.

| Request | Builder's or reviewer's pane | Another workspace's orchestrator | The workspace's orchestrator |
|---|---|---|---|
| Cancel | Refused | Refused | **Allowed (R-107)** |
| Resolve step | Refused | Refused | **Allowed (R-107)** |
| Resolve cancellation | Refused | Refused | Refused |
| Settle | Refused | Refused | Refused |
| Approve or decline a draft | Decides nothing | Not tested | Decides nothing |

## The read

The page uses the runner's one-reply `workflow.get` from ov-1434. On a real
runner with 340 items:

- The whole state is 1,141,713 bytes, more than one 1 MiB frame.
- The reply is 617,660 bytes, one frame, answered in 57 ms in a debug build.
  The call's deadline is 30 s.
- The browser draws all 340 rows in under 100 ms once the reply is in hand.

`plan.get` only names tickets that a theme, the queue or an agent refers to.
When a row has no title, the core also reads the paged `task.list`.

If a board is too large even for the one reply, the runner leaves out tracker
cards first, and the page says how many.

## Evidence

- `workflow_runtime/engine_view_tests.rs`: a person's cancel, resolve, resolve
  cancellation and settle on real stuck runs, with the refusals above. The
  settle uses a real push to a local bare remote.
- `tests/workflow_draft_approval.rs`: decline, approve and cancel through a
  real `Session` over the runner's socket.
- `npm run test:engine` in `services/web`: a real runner makes the 340-item
  page, then a browser draws it and presses each button at 320 and 1280.

The browser test answers presses with a stand-in session. The runner tests
above cover what the runner does with them. The two aren't joined in one run
through SSH and wasm.

Each of these checks was removed in turn, and its test failed:

- `Settle` taken off the person-only list.
- `ResolveCancellation` taken off the person-only list. The command then got
  past admission. The engine has its own owner check behind that one.
- Any agent role admitted, not only an orchestrator.
- Any workspace's orchestrator admitted.
- A pane's answer left marked as the person's.

## Not run or not passing

- `board-check.mjs` times out waiting for `#board-title`. It does the same on an
  unchanged copy of the starting commit.
- `csp-check.mjs`, `sleep-check.mjs` and `review-check.mjs` weren't run. They
  need a full build or a fake runner.
- `npm run test:engine` isn't wired into CI.

## Captures

Light, with long ticket titles. The landing is also shown in dark.

| | 320 | 1280 |
|---|---|---|
| Top and draft | [320](top-320-light.png) | [1280](top-1280-light.png) |
| Approve and Decline | [320](draft-buttons-320-light.png) | [1280](draft-buttons-1280-light.png) |
| Confirming with a reason | [320](confirm-320-light.png) | [1280](confirm-1280-light.png) |
| Stuck landing | [320](landing-320-light.png) | [1280](landing-1280-light.png) |
| Stuck landing, dark | [320](landing-320-dark.png) | [1280](landing-1280-dark.png) |
