# ov-928 continuation host contract

Preserves signed phase1 b2329453 and its evidence. No wire/auth/client/Store-schema
changes or installed Service activation. The remaining native host is daemon-local
and uses the same `TmuxServer::create_terminal_window` managed PTY, tags, send and
kill lifecycle as Service, and `agent_hooks::MessageAssembler`/`facts` normalization.
It does not substitute configured null-stdin execution for an interactive harness.

Native settings/project hook bytes remain immutable existing files. They must
already name the runtime's private `h.sock` endpoint; the runtime never installs or
merges hooks. A gate holds the new PTY before harness execution while the exact
operation/terminal/server identity is durably recorded. The retained preset builder
receives the correlated session identity. Host accepts only the selected agent,
exact worktree cwd and bound session on the private socket. Wrong-session/cwd,
subagent Stop, prose and process exit cannot assert successful completion. Hook
normalization ends the turn; owned terminal processes are stopped and observed
quiescent before a process settlement is published, then actual clean descendant
Git HEAD supplies the candidate. Cancellation/deadline closes the same owner.
Admitted work with missing settlement is never relaunched. Unprovable ownership
or quiescence leaves uncertainty visible. Tests use a fake interactive provider on
real private tmux PTYs and real hook frames, with no signed-in provider launched.

Both selected repository and bare landing repository hold lifetime exclusive writer locks.
Landing adds optional immutable identity (job key, canonical local bare repository,
remote ref). The existing guarded Git launch/transport mechanics are extracted
without changing installed landing semantics. Local CAS, exact remote old-base,
a proven descendant candidate plus exact expected-old-SHA lease push CAS, exact remote candidate observation, and quiescent bound
settlement are all required. Only explicit canonical private filesystem remotes
are enabled here; network/provider push remains outside this allocation. A crash
after push without settlement remains uncertain and never retries the push.

Source freezes before original923 warm-target checks and independent security
review. Later live-provider/main/cutover allocations are not claimed by fixtures.

Native `Runtime::open` verifies the retained lifecycle registrations before any
admission (including Cursor's existing gating Stop). Native process environment
supplies explicit TERM/COLORTERM. The root must fit a Unix socket path; config/hook
files already exist and are never installed. Codex/Cursor project hooks must be
available with the same pinned bytes in the provisioned worktree (offline tests
commit existing hook files in scratch repositories). Settings may retain other
user hooks; validation reads only the required lifecycle entries.

Native recovery leaves admitted unknown work alone. An explicit admitted Stop
can close the original durable owner after verifying current pane/daemon/worktree/
terminal/tty tags, stop its process tree, prove every observed owned PID reaped,
and settle the original before Stop. Gone/dead/foreign owners without quiescence
proof remain uncertain. No discovered PID is killed during ordinary reconciliation.
The current native host supplies no permission grants: gating hooks receive `{}`.

Local landing uses a cleared environment, inherited Git guard pins, system/global
config disabled for transport, and refuses effective URL/include redirects. It
records argv/environment/output digest/exit, full job identity and expected base.
Remote success requires both quiescent transport settlement and actual exact SHA;
visible ref alone never repairs a lost acceptance receipt. Local landing CAS follows
remote acceptance, so a later local CAS failure remains visibly uncertain.
