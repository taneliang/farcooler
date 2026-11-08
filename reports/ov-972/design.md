# ov-972: watch an engine agent's terminal, read-only

## What exists (verified)

- **Where the agent runs.** `Exec::run_native` (`crates/daemon/src/workflow_runtime/coding_native.rs:191`) gives every native build its own tmux server (`TmuxServer::in_dir("workflow-<id>", …)`, `:38`), opens one pane in it, and kills the server when the turn ends (`Host::drop`, `:32`). The daemon's own `Runtime` never lists that server, so `terminal.list`, `terminal.attach` and `terminal.write` cannot name the pane. That is the "private tmux".
- **How a pane streams today.** `terminal.attach` (`rpc.rs:1491`) resolves a live pane in the daemon's inventory, then `Runtime::attach` (`runtime.rs:473`) sends an opening replay (modes, scrollback, screen, cursor), then live bytes from a per-pane fanout, as `Event.terminal_output` frames through `resync::TerminalSink`. The Mac (`TerminalStream.swift`) runs `farcooler terminal stream <id>` and feeds stdout to the emulator. iOS uses `Session::open_stream` (`crates/client/src/session/link.rs:169`).
- **`Runtime` is just `{tmux, inventory, marks}`** (`runtime.rs:27`). A `Runtime` built over the engine's server can run the same `attach`.

## Design

1. **Registry.** `run_native` publishes `(workspace, item) -> {server, pane}` into a small in-process registry once the pane's tags are durable (after `:213`) and removes it before the server is reaped. Nothing else about the run changes.
2. **New wire method `workflow.watch`** (own capability `workflow_watch`). Takes workspace + item, owner-only like the other `workflow.*` methods. It looks the pair up in the registry, builds a `Runtime` over the engine's server, and calls the **same** `Runtime::attach` with the **same** `TerminalSink`. Frames are the existing `terminal_output`, so every client decoder is reused.
3. **Read-only by construction, not by a flag.** The method addresses `(workspace, item)`, never a terminal id, and the engine pane is in no inventory the daemon's `terminal.write`, `terminal.resize`, `terminal.tell`, `terminal.interrupt` or `terminal.paste_file` can resolve. `attach` only reads (`capture-pane`, `pipe-pane` output side). No code path takes bytes from the viewer to the pane.
4. **Mac.** New CLI `farcooler workflow watch <workspace> <item>`: opens a `Link`, calls `workflow.watch`, writes frame bytes to stdout. The app reuses `TerminalStream` with those arguments and the existing emulator view with input switched off (no key, paste, mouse report or resize is forwarded; the view is sized by the stream's own columns/rows). Shown from the run row in the workflow view.
5. **Phones.** iOS/Android get the same method through the client crate. If Mac takes the whole lane, file them as follow-ups.

## Proof of read-only (tests)

- Unit: a registered pane running `cat > file`; drive `workflow.watch`, then send `terminal.write`, `terminal.resize`, `terminal.tell` for the pane's terminal id, and assert each is `NotFound` and `file` stays empty. Break it by registering the pane in the daemon's inventory and watch it go red.
- Unit: the watch handler's source mentions no `send_keys`, `send_literal`, `send_bytes_hex`, `resize_*`, `set_pane_size` (grep test on the module).
- Mac: input-off flag test on the emulator view.

## Where the brief was off

- "Reuse the existing terminal view": the 1,500-line `TerminalRenderView.swift` is input-first; read-only needs a switch added, not just a new caller.
- The engine pane sits behind `read -r workflow_start` and one `send_keys("\n")` from the engine itself (`:215`). That is the engine's only input and it stays.
