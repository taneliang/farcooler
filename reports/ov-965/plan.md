# ov-965 plan: the engine owns only a ticket's stage

Base: `train-367` at `86732976c`. Not pushed. Nothing lands before the owner answers ov-964.

## What is true today (checked, not taken from the brief)

The brief and ov-964 item 7 say a tracked card refuses lines, rank, holds, waits, blockers and subagent records. Since ov-962's fix rounds, that is only half true:

- **Waits, holds, blocks, subagent records already work on a ticket at rest**, at `legacy` and `released`. They do it by dropping the fence triggers for one write and putting them back (`with_guards_down`, `workflow_projection.rs:117`), from `waits.rs:506,613`, `tasks.rs:941,973`, `workers.rs:447,529`, `agents.rs:390`. `rpc_plan.rs:249` no longer refuses; it only skips the column move. So M-g and B4 are closed, by a workaround.
- **Lines, plan rank and line holds are still refused.** `put_on_line` and `update_line` call `refuse_if_owned` (`ticket_lines.rs:406,443`). `set_line_order` has no check and aborts on `workflow_excludes_ticket_lines_update`. Any `ensure_on_line` (dispatch, `rpc_plan.rs:87,144,163`) fails the same way. This is what still stops plan order and PR chips.
- **A ticket's timed hold in Todo never lets go.** `set_wait` lets a ticket hold in Todo without moving it (`waits.rs:502`), but `next_hold_due` and `release_due_holds` read only `status='backlog'` (`waits.rs:577,595`). New finding.

So the database still says "owned means refused", and each writer that a ticket should pass lifts the fence by hand, with a DDL statement per write. A writer that forgets is refused. Lines were forgotten.

## The change

Make the guards say the rule: **a ticket at rest (tracker-only definition, phase `WaitingApproval`) is fenced only on its column.** A card with a run, and a closed ticket, keep every fence.

1. **Registry 68, `engine_stage::migration`, `Older::Refused`.**
   - New table `engine_stage_tickets(task_id PRIMARY KEY REFERENCES tasks(id))`: the cards the engine holds as a ticket at rest. Absence means fenced, so a missed sync fails closed.
   - Backfill it from `workflow_states` in the same migration, so a released install at 67 with tracked tickets carries forward.
   - Recreate, same names, with `AND NOT EXISTS(SELECT 1 FROM engine_stage_tickets …)` added: `workflow_excludes_ticket_lines_{insert,update}`, `workflow_excludes_ticket_work_{insert,update}`, `engine_fence_{ticket_lines,ticket_work,task_blocks}_d`, `engine_fence_task_blocks_{i,u}`.
   - Split `engine_task_execution_fence`: it keeps `status, status_since` for every owned card; a new `engine_task_wait_fence` covers the `wait_*` columns except for a ticket at rest.
   - Unchanged: terminals, `landing_tickets`, landings and wakes fences (runs and landing stay the engine's), identity and delete fences, freezes.
2. **Sync** `engine_stage_tickets` in `workflow_apply`, right after `workflow_states` is written: insert for each owned item that is a ticket at rest, delete for the rest. The only writer of `workflow_states` is that one statement.
3. **Callers.** Remove every ticket use of `with_guards_down` (`TICKET_GUARDS`, `WORK_GUARDS`); the projection keeps its own. `put_on_line` and `update_line` refuse only a card with a run. Release a ticket's due hold in Todo too.
4. **Cutover pins.** `preflight` accepts registry 68 with a pinned `CUTOVER68_SCHEMA_HASH` for the `engine_*` objects, and treats `engine_stage_` tables as engine metadata, as it does `engine_manual_`. The four `workflow_excludes_ticket_*` triggers are outside the engine namespace, so:
   - the composed catalog at 68 gets its own pin, `SOURCE68_SCHEMA_HASH`, for `schema:unknown` and for the raw-60 catalog check after transfer;
   - `verify` allows exactly one named delta for a source below 68: those four objects, and only to the registry-68 text.
5. **Startup.** ov-963's `inspect_against` already admits a released 67 below the current schema; `Store::open` then migrates it. No change expected; the test proves it.

## Tests (each red without its change, at `legacy` and `released`)

- On a ticket at rest: put on the line, update the line (stage, branch, hold), plan order, set and clear a wait, timed hold released (Backlog and Todo), block and unblock, record and end a subagent, `start_work`/`end_work`, end an agent. Each via the store with no fence lifted.
- Still refused: a hand column move on the ticket; every one of the above on a card with a run; a closed ticket.
- The marker: added at Enqueue of a ticket, removed at Start and at close.
- Migration: a released 67 database with a tracked ticket and a run-owned card opens, migrates to 68, keeps `released`, backfills the marker for the ticket only, and the lines work on it.
- Cutover: a legacy 68 source cuts over and verifies; a 67 source cuts over to 68 through the named delta; a changed 68 trigger is refused.
- Reruns: `engine_cutover` store suite, `engine_cutover_rehearsal` example, `a_cutover_survives_every_crash`, `old-reader.py`.

## Left alone, said out loud

- **Migrated cards with open lines, waits or blocks still can't be tracked** (`workflow_ownership.rs:26-31`, `workflow_projection::untracked`). With this change they could be. That's ov-964's migration question (option B), not item 7, so it waits for the owner.
- **Start on a ticket with an open wait, block or subagent** keeps them, now under a run's fences. Not new; the run doesn't read them.
