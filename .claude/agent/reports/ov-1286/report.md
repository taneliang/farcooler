# ov-1286: a lead change checks who asked

## What changed

- **`workspace.set_settings` with a lead** now refuses any caller its process
  doesn't show is a person: `scope-denied` (`needed: person`), at
  `crates/daemon/src/workspace_ops.rs:210`. It's the same check ov-1267 gave
  `terminal.set_role`, moved into one helper, `refuse_unless_a_person`. A lead
  change restarts the orchestrator in the person's seat, so an agent's lead
  change replaced the sitting orchestrator. Test:
  `people::tests::an_agent_never_hands_the_lead_on`. It covers a builder
  pane and the orchestrator pane (both refused), then a person (allowed).
- **`client.revoke`** is now refused `person_only` from a pane, as
  `member.remove` already was (`rpc.rs:1061`). Test:
  `people::tests::an_agent_never_revokes_a_device`.
- **`worktree.remove`'s race.** The orchestrator check now runs inside the
  removal (`Service::remove_worktree_as`, `service.rs:3254`). It holds the
  seat lock of every workspace in the repository, plus any workspace a pane
  in the worktree names. Every promotion to orchestrator (`seat_lock`) and
  every orchestrator start takes that same lock. The panes are checked
  (`service.rs:3318`) and stopped under those locks. The locks are released
  before the repository lock is taken, because `reconcile` holds the
  repository lock. Test:
  `people::tests::worktree_remove_finds_an_orchestrator_promoted_after_the_request`.
  The test holds the seat while the request waits on it, promotes the pane,
  and then lets the seat go. The builder is refused, and the pane and the
  worktree are left in place.

I broke each new check by hand and watched its test fail. With no lead
check, the agent's lead change was answered. With no seat locks, the removal
never waited and deleted the promoted pane. With no revoke check, the
agent's revoke got past the refusal.

## Every `host_admin`-gated RPC an agent pane can reach

On the local socket, a caller that sends no preamble is `host_admin`
(`sessions.rs:39`). So an agent in any pane passes every gate below unless
its handler checks who is calling. The method-level list is `scope_of`
(`rpc.rs:379`). After it come the handlers that test for `host_admin`
themselves.

| RPC | Agent pane | Where / why |
| --- | --- | --- |
| `workspace.set_settings` (lead) | **refused** `scope-denied`, new | `workspace_ops.rs:210` |
| `client.revoke` | **refused** `person_only`, new | `rpc.rs:1061` |
| `member.set` | refused `person_only` | `rpc_people.rs:44` |
| `member.remove` | refused `person_only` | `rpc_people.rs:112` |
| `member.set_own_login` | refused `in_pane` / `person_only` | `rpc_people.rs:196`, `:200` |
| `member.set_role` | refused `in_pane` / `person_only` | `rpc_people.rs:217`, `:221` |
| `member.set_access`, `client.set_cap` | refused `person_only` | `person_access_change.rs:90`, and `person_access.rs:531` (socket path) |
| `client.enroll` (incl. teammate binding, the only other `host_admin` test, `teammate_bind.rs:68`) | refused `person_only` | `person_access.rs:195` |
| `workflow.cutover` | refused, except the owner's own orchestrator with the owner's one-time grant (ov-974) | `rpc.rs:838` checks for the owner. `cutover_delegate::admit` (`cutover_delegate.rs:187`) admits a person, or the owner-led workspace's orchestrator on the owner's "Allow cutover once" grant. A builder is refused there. `Inspect` only reads. |
| `worktree.remove` | **allowed**, unless a pane in the worktree is an orchestrator the caller may not displace (now checked atomically) | Lanes remove their own worktrees when they finish, as the manager skill tells them to. Only an orchestrator pane needs protecting: `service.rs:3318` |
| `ceremony.authority` | allowed: unreachable without a device | `rpc.rs:1040` refuses `AuthRequired` with no preamble device. With a device named, it signs only for that device's own key fingerprint. |
| `daemon.shutdown` | allowed | It stops no work and moves no seat or access. Terminals are tmux's, the app restarts the daemon, and a pane already runs as the runner's Unix user, so it can kill the daemon directly. |
| `repository_root.list` | allowed | A read. |
| `repository_root.add` | allowed | It only widens where the daemon looks for repositories. A pane already reads and writes there as the same Unix user, and no person, seat or access changes. |
| `repository_root.remove` | allowed | It's refused while any pane under the root is running or starting (`service.rs`, `remove_root`), so it never stops a live orchestrator. It needs the root's name typed back (`rpc.rs:1336`), and it touches no disk. |
| `settings.set_branch_prefix`, `theme.upsert`, `theme.delete` | allowed | Cosmetic lines in the runner's config file, which a pane can write directly as the same user. No seat or access depends on them. |
| `settings.set_projector` | allowed | A no-op kept for old apps (`rpc_rows.rs:156`). |

Handlers that read `host_admin` only to decide what to show (paths in
`wire.rs`, `rpc_plan.rs:42`, `shared_logins.rs:277`, `rpc_people.rs:410`)
grant nothing and aren't listed. `runner_key::sign` (`runner_key.rs:193`)
has no RPC. Only the CLI calls it, on the runner's own user (ov-497).

## Noticed, not changed

- Any local process can send a preamble naming a device this runner knows
  (ov-980 only refuses unknown ones). Such a caller is then that device,
  which changes what `client_id`-based checks decide. The pane checks above
  still hold, because they read the caller's process, not the preamble.
  Worth a security ticket of its own if that's not already accepted.
