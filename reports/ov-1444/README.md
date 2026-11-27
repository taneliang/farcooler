# ov-1444 · Start and recover work on a paired runner

**Start Work** opens the repository, worktree and terminal-type controls. Select a registered repository to switch to its Main workspace, or choose a workspace directly from the runner sidebar. Use an existing checkout or create a worktree with a name and Git branch. The start options are Shell, Codex and Claude Code.

## Actions and runner methods

| Browser action | Runner method | Rust behavior |
| --- | --- | --- |
| Pick a workspace or repository | `workspace.list`, `repository.list` | Resolve the saved or linked workspace. A repository selection resolves to its Main workspace. |
| Resume an existing worktree | `worktree.list`, `terminal.list`, `terminal.attach` when opening an existing shell | Project available checkouts for the selected workspace and repository. Use the existing checkout ID when starting another terminal. |
| Create a worktree | `worktree.create` | Create from the repository checkout’s `HEAD` and claim the worktree for the selected workspace. Return its canonical ID. |
| Start a shell or agent | `terminal.create` | Map the selected terminal type to a runner preset and return the terminal ID. Shell output uses `terminal.attach`. Agent conversations use the existing `agent.rows` and `agent.rows.follow` paths. |
| Rename a terminal or agent | `terminal.rename` | Send the name and refresh the runner inventory after the reply. |
| Close a terminal or agent | `terminal.list`, `terminal.stop`, `terminal.remove` | Ask for confirmation. Read the current state, stop a live process and remove its record. A retry can remove the record if stopping already succeeded. |
| Reconnect a dropped terminal stream | `terminal.list`, `terminal.attach` | Read the terminal’s current state and attach to a live process. A dropped runner connection still uses the existing passkey reconnect flow. |
| Recover a lost terminal or agent | `terminal.list`, `terminal.restart` | Check that the runner still reports the terminal as lost. Use the daemon’s existing recovery logic and follow the ID it returns. |
| Show writer or read-only status | `ServerHello.granted_scope`, `terminal.list` with enforcement through existing daemon scopes | A control or host-admin grant can write. A read grant can see terminal metadata and gets a plain-language explanation when a command requires control. The current daemon does not enforce exclusive writer leases. |
| Open a workspace or terminal URL | Same inventory reads and terminal attach as above | Resolve runner-qualified IDs. Missing workspace and terminal links get an explanation without opening another terminal in their place. |
| Quote a Plan item in the orchestrator’s conversation | No mutation. Uses the selected terminal and the existing conversation reads. | The browser waits for the selected agent pane, then prefills an unsent draft or shows an error in Plan. |

## Shared Rust and the browser

`crates/client/src/work_view.rs` owns repository and worktree selection, terminal-list scoping, the first terminal to open, state labels, action availability, prompts and refusal copy. This replaced the terminal-specific selection and empty-state logic in `services/web/src/client/scope.ts` and the labels and first-pane selection in `app.ts`. The terminal grouping and state projection previously in `crates/web/src/wire.rs` now lives in the shared client as well.

`crates/client/src/session/work.rs` dispatches lifecycle intents through the scoped session. The browser renders the projection and keeps browser history, DOM focus, prompts and terminal rendering. The URL contains the granted runner ID and the workspace and terminal UUIDs. Opening it in a new tab goes through the normal passkey check.

## Browser evidence

The fake runner uses the shared Rust projection and production work controls and xterm renderer. The worktree label includes this ticket’s title and actual branch name. Both captures use long runner, workspace, repository and terminal labels.

| View | Phone, 320 px | Desktop, 1280 px |
| --- | --- | --- |
| Terminal | [Phone](terminal-320.png) | [Desktop](terminal-1280.png) |
| Start Work | [Phone](start-work-320.png) | [Desktop](start-work-1280.png) |

The three Rust projection tests first failed against an empty projection. The initial nine browser cases first failed against the baseline work screen. The stream test independently failed when the fake runner withheld the disconnect event. The browser check then passed all ten cases.

The shared Rust client suite passed 399 tests with one ignored. The browser Rust suite, WASM target check, TypeScript check, web unit and workflow suites, and shared-core lint passed.

The **Web bundles and browser checks** CI job builds `work_view_bin` and runs the work browser check using the same Playwright Chromium installation as the Plan render check. Running that step locally with a deliberately incorrect terminal ID in the rename assertion exited with status 1. The superseded-selection and Plan quote-error regressions also failed before the fix. After restoration and the fix, all eleven work browser cases and the Plan render check passed.

The macOS landing replay built the example, then exited at the lookup for `google-chrome` or `chromium` on `PATH`. Using Playwright’s installed Chromium lets the same CI step run on both hosts. The adjacent tmux self-test passed independently.

The existing SSH browser fixtures now serve the work inventory and preserve runner identity during the host-pin test. All seven Chromium sleep cases passed once the test waited for terminal reattachment before sending input.

To rerun the browser tests after installing the web dependencies, build the web WebAssembly module and install Playwright Chromium. Set `CHROME_PATH` to use an existing Chrome executable.

```sh
export CARGO_TARGET_DIR="$FARCOOLER_SCRATCH/target"
mkdir -p "$FARCOOLER_SCRATCH/tmp"
export TMPDIR="$FARCOOLER_SCRATCH/tmp"
crates/web/build.sh --channel canary --out "$FARCOOLER_SCRATCH/wasm"
cd services/web
npx playwright-core install --only-shell chromium
WEB_WASM_DIR="$FARCOOLER_SCRATCH/wasm" \
  node test/browser/work-check.mjs ../../reports/ov-1444
```
