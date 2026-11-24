# ov-1446 · Workspace board and tickets in the browser

The workspace Board shows the runner’s ticket columns and unread state. Open a ticket to read its intent, acceptance criteria, constraints and labels. Create and edit tickets from the browser, open the existing activity and note sheet, and start a ticket under the workspace’s published workflow. The Board uses the workstation’s shared navigation and can sit beside the orchestrator. A ticket link carries the runner and workspace and opens the same ticket from a new tab, after reload, or through Back.

The full step 3 in [the parity inventory](../ov-1439/parity.md) would exceed the ticket’s roughly 1,500-line budget. This branch delivers the board and ticket actions. The remaining work is split into two tickets on **Web first**:

| Ticket | Scope |
| --- | --- |
| ov-1458 | Browse worktree files and diffs, comment inline, approve and restore the review position. Rust owns the anchors, reviewed state and stale-candidate rules. |
| ov-1459 | Show Engine stations and ticket drill-down beside the Plan and orchestrator layout from ov-1445. Prove creation through landing in one scripted browser run, including opening the Engine’s transient agent and worktree. |

## Runner behavior

- The Board uses the existing paged `task.list` path in the shared Rust session. Each response is bounded at the wire. The browser receives columns and card summaries from Rust.
- Opening a ticket marks through the runner timestamp that the ticket displayed. Mark Board Read uses the newest visible ticket timestamp. The runner merges marks by person, so a second device sees them on its next read.
- Ticket edits use the revision that populated the form. A concurrent edit keeps the user’s draft in the form and reports the conflict. Edits preserve untouched acceptance criteria and linked worktrees.
- Start uses the authenticated Engine admission, current ticket run and published policy pin. It sends the existing `workflow.start` action, and a changed run or publication invalidates the displayed action.
- The ticket links to its active registered agent terminal and worktree when those records are available. Worktree links open the existing Start Work picker at the exact checkout. The existing read/control grants govern all calls.

## Browser evidence

The scripted fake-runner check creates a ticket, edits it, refuses a stale save, starts it, follows both destination links, and returns to the ticket through Back, reload, and a direct link. A second browser context observes the merged read state. The workstation smoke check opens a Board ticket beside the orchestrator and verifies the draft survives navigation. Rust socket tests exercise ticket writes, shared reads and Engine Start through the daemon dispatcher, and read a workspace with more than 1 MiB of ticket data.

The Rust client suite passed 407 tests with one ignored. The Rust web, web unit and workflow suites, Board, Work, Plan and workstation browser checks, both TypeScript checks, the WASM build, Clippy and shared-core lint passed.

The web build disables inlining for the browser’s Go Tailcat module. The measured Brotli download is 5,003,167 bytes for Stable and 5,000,563 bytes for Canary, within the existing 5,500,000-byte limit.

The captures use the parity project’s ticket titles and the long brief from ov-1446.

| View | Phone | Desktop |
| --- | --- | --- |
| Board | [320 px](board-320.png) | [1280 px](board-1280.png) |
| Ticket | [320 px](ticket-320.png) | [1280 px](ticket-1280.png) |

To rerun the browser check with the web dependencies and Playwright Chromium installed:

```sh
export CARGO_TARGET_DIR="$FARCOOLER_SCRATCH/target"
cargo build -p farcooler-client --example work_view_bin
cd services/web
WORK_VIEW_BIN="$CARGO_TARGET_DIR/debug/examples/work_view_bin" \
  node test/browser/board-check.mjs ../../reports/ov-1446
```
