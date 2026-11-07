# ov-965 result: the engine owns only a ticket's stage

Branch `ov-965-stage-only`, from `train-367` at `86732976c`. Not pushed: nothing lands before the owner answers ov-964. Plan: `reports/ov-965/plan.md`.

## What changed

- **Registry 68** (`crates/store/src/engine_stage.rs`). `engine_stage_tickets` names the tickets the engine holds with no run. `workflow_apply` rewrites it from the state it commits, in the same transaction. The migration backfills it from `workflow_states`, so a released install carries its tickets forward.
- **The guards say the rule.** On a ticket at rest, these pass: lines, plan rank, line holds, waits, timed holds, blocks, subagent records, and an agent's end. The column (`status`, `status_since`) stays fenced on every card the engine holds. A run and a closed ticket keep every fence. A card missing from the marker is fenced, so a sync that never ran fails closed.
- **No more lifting fences per write.** `with_guards_down`, `TICKET_GUARDS` and `WORK_GUARDS` are gone. Each one ran a DDL statement on every ticket write. Only the projection still lifts its own guards, and it now lifts the new wait fence too, because a move clears a wait.
- **New bug fixed:** a ticket's timed hold in Todo never let go. Holding a ticket leaves its column alone, but the sweep only read Backlog.
- **Cutover.**
  - Preflight takes registry 68, pinned as `CUTOVER68_SCHEMA_HASH` (engine objects) and `SOURCE68_SCHEMA_HASH` (the composed catalog).
  - Every comparison after the freeze uses `verify::as_registry68` (commit, backup permit, closed backup, verify). It allows one named delta: the four `workflow_excludes_ticket_*` guards, and only from their exact pre-68 text to the 68 text.
  - Those four guards are outside the engine's namespace. That's why the delta is needed at all.

## Where the brief was wrong

Waits, holds, blocks and subagent records already worked on a ticket at rest at both phases. ov-962 made them work by dropping triggers for one write. What was still refused was lines, plan rank and line holds (`ticket_lines.rs:406,443`, plus the trigger under `set_line_order`), so plan order and PR chips were still stopped.

## Evidence (private synthetic fixtures only; no owner database, app, daemon or provider touched)

| Step | Result |
| --- | --- |
| `cargo test -p farcooler-store` | 641 passed, 3 ignored (lib), with clippy `-D warnings` clean on the store. |
| `stage_only` module, 12 tests | Each runs at `legacy` and `released` where both apply. Covers lines, rank and hold; waits, holds, blocks and subagents with the schema cookie unchanged; a Todo hold; the guards by raw SQL; the marker through Move, Start and close; the pinned 68 catalog; a released 67 install migrating forward; a legacy 67 source through commit, and through the real `transfer::begin` and `closing::commit`; a hand-changed guard refused at commit. |
| Mutants | 19, each reverting one change alone. All went red, and each red test is named in the commit. The first run caught one survivor: the exact-text check was hidden behind the release's `schema:unknown`. The test now asserts the refusal at commit. |
| `engine_cutover_rehearsal` example | Reached `committed`, incarnation 1, manifest equal to baseline (`wf1:969bc0d0…`), independent restore made, and the send, push and Enter counters were all 0. |
| `a_cutover_survives_every_crash` | 7 passed in 70.7 s. |
| `reports/ov-940/old-reader.py` | The registry-60 reader at d625aaeb9 opens v60. It refuses the Prepare file with `NewerData`. The result file is unchanged. |
| Daemon | Lib: 2831 passed. 4 `stream_end` pane-restart tests failed under load and pass alone (7/7); they don't touch the store's guards. Integration: `tracking_follows_publication`, `a_ticket_shows_its_pr_stage`, `a_reviewers_usual_wait_reaches_the_plan`, `workflow_commands_over_socket`, `workflow_service_over_socket`, `plan_waits_follow_the_scope` and `plan_agents_over_the_socket` all pass. |

## Left alone, for the owner or a later card

- **Migrated cards with open lines, waits or blocks still can't be tracked** (`workflow_ownership.rs:26-31`, `untracked`). With this change they could be. That's ov-964's migration question, not item 7.
- **At `legacy`, `set_block` on a card with a run isn't refused.** The block fences are phase-gated, and `set_block` has no `refuse_if_owned`. This was already true before this card; the new raw-SQL test records it.
- **A run started from a ticket keeps its open waits, blocks and subagents,** now under the run's fences.
- Not run: the owner-copy rehearsal, the apps, Linux.
