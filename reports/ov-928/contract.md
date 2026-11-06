# Configured coding adapter contract

Baseline: 7038e1704. This source implements configured-command execution. Native
terminal/hook ownership remains open; this is not acceptance of the entire task.
Final source/evidence are pinned in the handoff. No installed Service selector or
wire-command module is changed.

`workflow_runtime::CodingConfig` selects a canonical local repository, an initial
full base SHA, a local landing ref, a coding command or existing native harness
preset/settings descriptor, bounded check argv/environment, optional reviewer, and CI provider
identity/receipt command. `Runtime::create_configured(root, config)` creates only
its own private database, receipts and worktrees. Config bytes are immutable and
their digest is included in every supported operation. `configured_definition`
builds finite definitions; progression/approval, multiple checks and bounded
review rework remain explicit policy. Fixtures keep their original constructor.

Provision uses the retained guarded Git worktree primitive. Each item's base is
durably pinned before admission from the exact current local landing ref. The
constructor verifies that ref equals the configured initial SHA. Every effect of
an item reuses its pin, including rework, recovery and cancellation. Build runs the configured command under
owned process supervision; a clean actual descendant commit is required, not an
exit message. Native descriptor rendering uses the existing preset/settings builder without
installing hooks or changing login/security state; execution remains refused below. Checks run exact argv with
explicit environment and verify the same clean candidate before and after.
Reviewer success requires a structured receipt bound to the full effect and exact
candidate in addition to a quiescent successful process. Landing uses exact old-SHA
local Git CAS. CI requires a structured provider receipt with the selected identity,
run ID, workflow/event and candidate SHA parsed by retained ci_watch code; there is
no generated always-success result in configured dispatch.

All effects remain behind the existing durable claim/admit boundary and bindings
include workspace, effect, run/fence, resource generation, definition/policy pins,
candidate, iteration and per-item base. Raw process/Git/provider facts survive.
An admitted effect with no authentic settlement is never relaunched. Git operations
publish bound settlement only after retained guarded Git returns; a visible ref or
worktree alone does not prove old-process quiescence. Reconciliation
checks durable facts and exact current Git state; unknown process/provider acceptance
stays visibly uncertain and retains ownership. Cancellation reconciles original facts even after Cancel fences Observe. It must
settle the original operation and prove owned process quiescence before Stop can settle. Process waits,
explicit observations and real claim deadlines wake execution; no ACK/history timer.

Trust: configuration and adapter receipts are private trusted local runner input,
not an untrusted-code sandbox or a public result ingress. The configured commands
may execute code and are only admitted from trusted local construction. Offline
tests use private Git/config/database and deterministic provider-hook fixtures.
No signed-in provider launch, remote push, app activation or main integration is
claimed. This source pins local landing ref and CI repository/workflow/event
identity; it performs only local CAS and supplied provider reads. Remote landing
job identity and push allocation remain pending; no remote landing is claimed.

Relevant verified baseline seams: workflow_runtime.rs constructor and pump;
workflow_runtime/adapters.rs Binding, pin_base, verify_candidate, stop;
effect_runner.rs supervised process groups; git.rs create_worktree_with;
service.rs preset_command_with_hooks (366) and LaunchExtras (165);
ci_watch.rs parse_run (156), summarize (193); landing_job.rs push (451) gates
remote fetch/push and cannot be called safely in this offline allocation.


## Verified native integration gap

`service::preset_command_with_hooks` emits an interactive native launch. The owned
configured-command supervisor uses null stdin and does not own a Service terminal
or native lifecycle hook-completion session. Reusing its string does **not** create
a working native coding adapter. `create_configured` and `open` therefore refuse
`CodingHarness::Native` before execution. Native settings/preset descriptors and
rendering are source-tested only. Existing settings bytes are digest-pinned;
Codex/Cursor project hooks must already exist at the exact selected path, and
plugin directories are refused. There is no installer or new TUI implementation.

The orchestrator has been asked for bounded ownership coordination with ov-927,
which owns Session/server context, rather than a broad UI planning gate. Native
execution and complete ov-928 acceptance remain pending. The working path is
`CodingHarness::Command`, which runs real configured local coding argv and requires
an actual clean descendant commit. Deterministic provider-hook scripts exist only
inside scratch tests; configured production dispatch has no fixed fixture script
or synthetic CI result.

Configured mode is trusted-local only. The wire publication/enqueue seam retains
its existing fixture validation; ov-927 owns that module. No wire support for
configured definitions is claimed. Independent security review is required before
integration, followed by separately allocated native/provider/push/main gates.
Task remains in progress while native ownership coordination is pending.
