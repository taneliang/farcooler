# ov-1480 workflow and terminal writer verification

Linux CI's pane process stopped after proposing the draft with `authentication required`, matching the caller-seat refusal reproduced in fcdiag and leaving the enclosing test waiting for landing, while the macOS landing report covers the quick gate and targeted socket and stdio runs and this workflow fixture passed when run separately on macOS. [CI attempt 1](https://github.com/taneliang/farcooler/actions/runs/45795667586/job/139827713806) · [CI attempt 2](https://github.com/taneliang/farcooler/actions/runs/45795667586/job/139842114609) · [ov-1454 landing report](../ov-1454/README.md#landing-gate-repair)

Nextest counted one failed test per CI attempt. The standalone
`the_orchestrator_pane_runs_the_skills_commands` entry passed because its
pane environment was absent. It failed when
`a_ticket_goes_from_draft_to_landed_on_the_commands_the_skill_teaches`
launched it in tmux. The pane recorded `proposed` and panicked at the
workflow RPC helper with `authentication required`. The 60-second line
was nextest's slow-test notice. `common/mod.rs:242` asserted the detached
test's exit status after the failure.

On fcdiag with tmux 3.4, temporary diagnostics showed `workflow.get`
returning `authentication required` when the live caller-seat recheck
changed from `Agent(Some(...))` to `Unreadable`. The VM refusal occurred
later in the flow, while polling for review. Linux reads ancestry from
`/proc` and macOS uses a kernel process-table snapshot. The exact check
that failed in CI remains unconfirmed. The fixture sends `workflow.*`
RPCs over a Unix socket, subject to workflow admission. The terminal
input lease governs writes into the pane's PTY.

The fixture now gives a refused `workflow.get` up to two seconds to recover
its live proof. Workflow mutations still require successful admission and
their expected receipts. The owner connection acquires the orchestrator
pane's input lease before approving the draft, so the run exercises the
orchestrator's commands while another client holds that lease.

## Verification

| Host | Result |
| --- | --- |
| fcdiag, Ubuntu 26.04 ARM64, tmux 3.4 | 105 passed across the same three suites with CI's leak checks. Nextest run `3d0a4ac6-d7b0-4f72-a953-60024a0961dc` |
| macOS 27.2.1 ARM64, tmux 3.8 | 105 passed across `engine_flow_per_skill`, `rpc_over_socket`, and `stdio_transport`. Nextest run `afa9dc0a-9dc9-4eba-a2b0-4be63fa08d8d` |

Both runs included OV-1454's five daemon regressions for exclusive typing,
vacant lease claims, native takeover notification, shared stdio leases, and
survival of the writer after the initial SSH session closes. The outer
workflow test verified all the command markers and the landed Git commit.

In a disposable Linux copy, injecting one `authentication required` result
on the first read after proposal stopped the original fixture at
`proposed` (1 passed, 1 failed). The revised fixture landed the ticket with
the same injection (2 passed). Build output stayed in ticket scratch. The
Linux tests used a private 32 GiB tmpfs within the guest scratch directory
to meet the workflow's free-space check.
