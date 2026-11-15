# ov-1275 security follow-up

The same caller-seat rule now protects orchestrator replacement and the terminal stop, restart, and remove RPCs. Once ov-1267 (commit 958cd3f23) lands, its `terminal.set_role` guard should use this helper too. `terminal.write` into the orchestrator pane remains possible; this is a documented guardrail limit and is outside this ticket.

## Pane-ending paths audited

- Checked: `terminal.stop`, `agent.stop`, `terminal.restart`, `terminal.remove`, and `workspace.start_orchestrator` replacement call the shared role-and-seat check before acting.
- Checked: `worktree.remove` preflights every terminal before stopping any. `set_terminal_role` can adopt a pane in a non-main worktree (service.rs `set_terminal_role`); starting a new orchestrator in the main checkout does not constrain adoption.
- Checked: `remove_root` calls `remove_terminal` only after refusing running or starting terminals. `remove_terminal` itself refuses those states; it removes retained dead panes.
- Checked: `stop_reps_of` stops registered reps when a member is removed; this is an internal member-removal operation, not a terminal stop RPC. `fresh_on_new_lead` and `follow_leads` perform the internal lead-change transition; replacement passes the trusted person seat. `restart_published` is reached from the checked restart RPC.
- Checked: watcher cleanup removes spent changes panes; `watch/reap` kills orphan/check-in cleanup panes under its reconciliation fence. Chat migration respawns a chat pane as its TUI; rep resumption relaunches a registered rep. These are internal reconciliation operations, not a caller requesting an orchestrator stop. `relaunch` also implements the checked terminal restart.
- `kill_pane` is called by `stop_terminal` and `remove_terminal`; `respawn_pane` is called by `relaunch_held`. Tests also call these service operations directly as trusted in-process operations.

Verification: the RPC people suite passed (24 tests). The new `agent.stop` regression failed with the guard removed (the builder succeeded), and passed with it present. The broader orchestrator run passed 80 tests; its one failure was the new worktree fixture before its workspace was assigned, and that test passed after the fixture was corrected.
