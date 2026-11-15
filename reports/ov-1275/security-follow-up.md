# ov-1275 security follow-up

The same caller-seat rule now protects orchestrator replacement and the terminal stop, restart, and remove RPCs. Once ov-1267 (commit 958cd3f23) lands, its `terminal.set_role` guard should use this helper too. `terminal.write` into the orchestrator pane remains possible; this is a documented guardrail limit and is outside this ticket.
