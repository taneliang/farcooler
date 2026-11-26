# ov-1484 · Web sidebar names

## Runner finding before the change

The Canary runner's `farcooler-canary worktree list --json` matched the screenshot's six `orchestrator` rows and the neighboring `ov-788` and `ov-1021` terminals. All six were distinct terminal records in **Main · overnight**. Five had lost their processes. The workspace's listed orchestrator ID matched the one running terminal.

| Terminal ID | Runner state |
| --- | --- |
| `01a1c47d-b225-7921-a31b-1a742ba2df8c` | LOST |
| `01a1cd4d-def4-7d12-9399-7fc9cb2adf71` | LOST |
| `01a1e3d8-fada-7353-8294-10e0db448827` | LOST |
| `01a20416-9ee1-7611-87a2-fd36bb74947b` | LOST |
| `01a205e2-9594-73a2-abac-0b2fce17afe7` | LOST |
| `01a217d1-04bb-7732-a01e-30db77d32211` | running, current seat |

The web core already scoped these rows to the workspace. It included each lost record in the same list, and the browser displayed its state only in a tooltip. The workspace picker used the workspace name alone, although the runner listed four separate Main workspaces.

## Sidebar behavior

The picker displays the workspace and repository together and keeps its Needs You count. The Rust projection scopes terminal rows, puts the runner's live orchestrator first, and provides its badge. The browser shows ended rows under **Unavailable Terminals**, where each row shows its state and lost processes can still be recovered.

## Browser evidence

The Chromium regression uses the shared Rust work projection and a fixture with two repositories named as on the runner, both with a Main workspace. The long shell title comes from the existing ov-1445 ticket. The two viewport checks failed against the original implementation because the rows appeared as shell, lost orchestrator, exited orchestrator, and live orchestrator. Against the fix, the live orchestrator appears first, the unavailable rows have visible states when expanded, and switching to Verdela shows only its orchestrator.

| Viewport | Before | After |
| --- | --- | --- |
| 320 px | [Capture](before-320.png) | [Capture](after-320.png) |
| 1280 px | [Capture](before-1280.png) | [Capture](after-1280.png) |

The full work-screen Chromium suite passed all 18 cases. The five Rust work-view tests, three Rust web integration tests, five Needs You scope tests, and the web client type check also passed. The full CSP/SSH browser suite remains unverified locally.
