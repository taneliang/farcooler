# ov-886: evidence

Branch `ov-886-hook-epochs`, on main `4dac77977`. Design: `reports/ov-886/design.md`.

## Commits

| Commit | What |
| --- | --- |
| `e46c87ef0` | Design note |
| `dbe0ea335` | store: `begin_launch` (compare-and-set on the terminal's version), `roll_back_launch` (only while still current), `terminal_for_launch`, rows in `meta` |
| `42142ff10` | cli + wire: `HookLine.launch`, filled from `FARCOOLER_LAUNCH` when it's a uuid |
| `f881ce681` | ingress: tagged frames routed by launch alone; retired launches dropped before any effect; launch gate around every effect; `relaunched` |
| `9db5a0c5b` | service: create, split and restart record and export a fresh launch, cancel the old one's in-flight state, roll back on a failed spawn |
| `a5f70d78d` | ingress: the launch gate is striped per pane; frames that name no launch take none; `observe_cwd` runs outside it |

## Red before green

Each test was run and seen failing before the change it covers, then passing after. Where the test was written after its code, it was turned red by reverting exactly the guard it tests, and then the guard was restored.

| Test | Red against | Red output |
| --- | --- | --- |
| `a_hook_from_the_launch_a_restart_retired_reaches_nothing` (real recorded Claude, Codex and Cursor payloads) | main's ingress | `[("claude", ["from the killed process", …]), ("codex", …), ("cursor", …)]`: all three harnesses delivered the killed process's prompt |
| `a_retired_launchs_ask_is_never_held` | main's ingress | `left: "{\"hold_ms\":60000}" right: "{}"`: the dead process's ask was held for a phone |
| `a_hook_from_the_current_launch_reaches_its_pane_beside_a_twin` | main's ingress | `the current launch's sentinel never arrived: []`: two Codex panes in one worktree, neither heard anything |
| `a_relaunch_waits_for_an_admitted_frame_then_clears_what_it_left` | `relaunched` without the gate | `the relaunch cancelled the pane while an admitted frame was still acting on it` |
| `a_frame_checked_after_a_relaunch_acts_on_nothing` | `held_for` without its re-check | `the old launch may no longer act on the pane` |
| `of_two_restarts_from_one_version_only_one_launches` | `begin_launch` without the version check | the second launch succeeded |
| `a_late_rollback_leaves_a_newer_launch_alone` | `roll_back_launch` without its `value = attempted` guard | the newer launch was overwritten |
| `a_launch_whose_pane_went_by_cascade_routes_nothing` | lookup without the pane-exists check | the orphaned row routed |
| `each_restart_exports_a_fresh_launch_and_retires_the_one_before`, `a_created_agent_pane_exports_its_launch_and_a_shell_none`, `a_restart_whose_spawn_fails_puts_back_the_launch_it_replaced` | a service that records no launch | no `FARCOOLER_LAUNCH` on the pane's start command |
| `a_failed_restart_leaves_a_newer_launch_alone` (real tmux: the pane is killed inside the restart, so `respawn-pane` fails after the launch was recorded) | the real service with the store's rollback guard removed | `left: Some(01a1be54-5ddd-…) right: Some(01a1be54-5e58-…)`: the newer launch was overwritten |
| `a_hook_names_the_launch_its_pane_carries` (the real `farcooler` binary, env var set) | main's CLI | `left: None right: Some("019a6b2e-…")` |

Two ingress tests pass before and after, by design: `a_claude_started_inside_a_pane_is_not_pulled_into_it` and `a_launch_carried_out_of_its_worktree_reaches_nothing` guard the new launch path against pulling in frames that merely inherited a pane's launch id. Main routes neither frame anywhere either.

## Green runs (targeted)

- store: 199 tests (`pane_launches`, `compat`, `engine_cutover`, migrations, `delete_terminal`).
- cli: 47 (hook unit tests, `a_hook_names_its_panes_launch`, `a_native_hook_follows_its_private_socket`, `a_hook_is_never_in_the_way`).
- daemon `a_hook_reaches_the_daemon`: all 48, including the 5 new ones.
- daemon lib: 121 hook ingress, fence, held-ask, registry and launch-gate tests; 151 service tests (restart wiring, orchestrator launch, pane env, split, chat migration, stand-in agents, the new launch tests).
- Adjacent suites before the gate fix: 249 (claims, followups, member removal, engine native builds, wire, registry, stop over the socket, launches).
- After the gate fix: 175 (hook ingress, launch gate, held asks, fence, registry, claims, followups, stop over the socket, the hook integration suite).
- Clippy `-D warnings`, all targets: store, agent-hooks, cli, core, daemon, clean.

## Real-producer evidence

- **Payloads**: the routing tests use the spike's recorded Claude, Codex and Cursor payloads (`tests/fixtures/hooks/*-before-move.json`), with the launch added on our envelope the way the CLI adds it. The producer's payload is never touched.
- **The CLI reading the variable**: the real `farcooler` binary, run with `FARCOOLER_LAUNCH` in its environment, sends it in the frame (`a_hook_names_its_panes_launch`).
- **Claude carrying pane variables**: this builder's own Claude, launched by Far Cooler through `with_pane_env` and the login shell, has `FARCOOLER_ACTOR=agent:01a1be05-…`, `FARCOOLER_TASK=ov-886` and `FARCOOLER_WORKSPACE=…` in its process environment (`ps eww` on the `claude` process). `FARCOOLER_LAUNCH` is added to the same `env` prefix. Claude's hooks are its child processes, as this session's tool shell is, and that shell sees the same variables. This pane predates the change, so a hook frame actually carrying a launch hasn't been observed from a live Claude yet.
- **Codex and Cursor**: not observed (Claude-only until Nov 10, no provider probes). If either doesn't carry the variable, its frames are untagged and routed exactly as on main, so neither regresses. The earlier source traces (`fed5b102f`, `069da6d15`) suggest Codex 0.166 on Far Cooler's current flags keeps an embedded hook host, which would carry it.

## Acceptance

1. **Reviewed design**: `design.md` specifies old/new identity (launch id), lifetime (one durable current launch per pane, no retained tombstones), respawn success and failure, and CAS rollback. It needs the independent review in item 3.
2. **Regressions**: delayed old hook (all three harnesses, same-session included), failed-spawn rollback (store and real tmux), newer-launch-survives, fresh restart binding, concurrent restarts, daemon restart (durable reopen), in-flight effects, no cross-pane attachment. Not covered: agy (not on main).
3. **Independent security review**: not done; a builder can't review its own work. Red-before-green is above; the full train gate is the orchestrator's.

## Open

- Live confirmation that a restarted Claude's hook frames carry `launch` (one restart of a real Claude pane on a build with this change, reading the frame at debug level).
- Codex and Cursor runtime confirmation, after Nov 10.
- Unmanaged agents in a managed worktree still attach by worktree match through the untagged path, as on main. That could be closed once Codex and Cursor are proven, by refusing untagged frames for a pane whose current launch has been heard tagged.
- ov-878 keeps its agy guard until it exports the launch on its path and shows an agy hook carries it.
