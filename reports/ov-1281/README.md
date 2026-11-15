# ov-1281: migrate the old engine's state onto the pipeline

Step 2 of the pipeline redesign (docs/engine-pipeline.md, section 10 step 2,
10.1-10.3, and the field mapping of 11). `migrate` is a pure function from
the old `State` to the new `Pipeline`. It is still unwired: nothing in
`crates/daemon`, `crates/cli` or `crates/store` changed, and the old reducer
is untouched.

## What landed

| File | Lines | Holds |
|---|---|---|
| `crates/workflow/src/migrate/mod.rs` | ~215 | `migrate(state, log, outbox, setup)`, the `Setup` (layout and pool sizes), the outbox's `Dispatch` rows, the archive, queue order, and `effect()`, which gives a kept old effect its new shape |
| `crates/workflow/src/migrate/items.rs` | ~325 | Tickets become tokens and live groups become batches. Phase and dispatch status set the `Location` |
| `crates/workflow/src/migrate/fields.rs` | ~60 | `ITEM_FIELDS`: the decision for every `Item` field (mapped, dropped or refused) and why |
| `tests/migrate_phases.rs` | ~420 | Every Phase and every ticket and group stage of every template, a group mid-gate, a waiting group, dispatch status, Uncertain, Cancelling, Failed, archived items, back edges and refusals |
| `tests/migrate_fields.rs` | ~215 | Every `Item` field has a decision, and each mapped field has a test |
| `tests/migrate_replay.rs` | ~85 | Replay across the cutover at every point of the 28 differential scenarios |
| `tests/migrate_rehearsal.rs` | ~180 | The 10.3 rehearsal: twelve builds, four places |
| `tests/migrate_snapshot.rs` | ~100 | A sanitized copy of a real workspace's state, migrated |
| `tests/differential_support/mod.rs` | ~500 | ov-1273's differential drivers, moved out of `coding_differential.rs` so that the replay can share them |
| `tests/fixtures/migrate-snapshot.json` | 1 | The sanitized snapshot: 66 items and 30 archived |
| `reports/ov-1281/sanitize.py` | | Makes that fixture from a live snapshot (see below) |

Changes to existing test support:
- `Run::resume` carries a driver on from a migrated pipeline.
- `Run::input` stamps the pipeline's own workspace and epoch.

## The signature

```rust
pub fn migrate(state: &State, log: &[Input], outbox: &[Dispatch], setup: &Setup)
    -> Result<Migrated, Vec<Quarantine>>
```

- **`log`** is the legacy log. It is used only for order: each item's last
  mention ranks the queues, and gives `Membership::joined`.
- **`outbox`** is the old runtime's rows, one per effect, with the 10.1
  dispatch states.
- **`setup`** is the layout to publish and the workspace's pool sizes.

`Migrated` holds:
- the pipeline, published once by `migrate` with the old revision, `seen`
  and `source_sequences` restored;
- what was quarantined, each with its reason;
- the effects whose rows the store must mark `suppressed`, in outbox order.

`Err` is the refusal list. Each entry names the item and says why it has no
faithful place. Nothing is guessed.

Dispatch state, per 10.1:

| Row | Becomes |
|---|---|
| `admitted` | `Working`, with the same effect id, run, fence and attempt |
| `uncertain` | `Working { uncertain }` |
| `pending`, `claimed` | `Queued`. The effect is listed for suppression, and the token's fence is bumped on its next start |
| `settled`, `suppressed`, no row | `Held { Quarantined }` |

## Acceptance

1. **Pure, with a test for every Phase and every stage.**
   - `migrate` has no I/O and takes everything as arguments.
   - `every_ticket_and_group_stage_of_every_template_migrates_in_place` cuts
     every differential scenario at every step. It checks each item against
     its step's station and records every (step, phase) pair it sees. It
     asserts that every stage of every template and every stored Phase came
     up.
   - Separate tests cover:
     - a group mid-gate, which keeps its effect and members and holds the
       landing;
     - an Uncertain item and its late answer;
     - Failed items, which are held `StepFailed`;
     - archived items, including a landed group's crossing.
2. **Every Item field decided.**
   - `migrate::ITEM_FIELDS` lists all 43 fields.
   - `every_item_field_has_a_decision` builds an item with every field set
     and asserts that its serialized keys equal the table. Deleting one row
     turns it red; I checked.
   - `items.rs` also destructures `Item` with no `..`, so a new field fails
     the build first.
   - Each mapped field names the test that shows it, and that test must
     exist.
   - Each refused field is set alone on a running ticket and must refuse,
     naming that ticket.
3. **Replay.**
   - 28 scenarios, cut at every point: 426 cuts.
   - 408 end exactly where one engine alone ends them.
   - 17 batch differently (see "Differences" below).
   - 1 ends differently after a person releases quarantined work. It is
     listed in `RELEASED` and explained below.
   - Each migrated run also replays from its snapshot to the same state.
4. **The 10.3 rehearsal** runs as `migrate_rehearsal.rs`. It has one test
   per case, plus "nothing starts at the cutover". Removing the core's host
   check turns the host case red.
   - Not covered: 10.3's crash, publish and rounds cases. They need the store
     and the mail adapter, which arrive in steps 3 and 4.
5. **Gates.** Both commands passed in the foreground:
   - `cargo nextest run -p farcooler-workflow`
   - `cargo clippy --workspace --all-targets -- -D warnings`

   Every new file is under 1,500 lines, and the vocabulary test passes.

Mutation checks (each reverted afterward):
- A token's fence set to 0: 7 tests fail.
- An uncertain item given a pending effect: 1 test fails.
- A batch's fence set to 0: 4 tests fail.

## The field table (11.1)

**Refused** (the item is named with this reason):

| Field | Reason |
|---|---|
| `execution` | A read-time view that is never stored |
| `follows`, `followers` | The pipeline has no followers yet (ov-1266 report, item 15) |
| `deciding` | A decision script is running for it, and the pipeline has no scripts |
| `hook_refused` | A script refused its decision, and the pipeline has no scripts |

**Dropped:**

| Field | Reason |
|---|---|
| `definition` | The published layout is in force for every token; there is no pinning |
| `pin` | `Published::digest` names the one workflow in force |
| `resource` | An operation's resources come from the adapter's catalog |
| `iteration` | Derived from visits |
| `rework_history` | Its back edges are the visits written from `back_edges`; the reasons stay in the legacy log |
| `retries` | Retries have no cap; the legacy log keeps them |
| `hook_calls` | Script call counts; the pipeline has no scripts |

**Mapped:** every other field, as 11.1 says. `ITEM_FIELDS` names where
each one goes.

## Departures from the design, and why

1. **An item whose step has no station is refused, not held.** A hold needs
   a live station to stand at. The layout must have a station for every
   step a live item is at.
2. **`Work::run` is the pending effect's id**, not the old run digest. The
   core binds `Observe` by `pending == effect`, or `run == effect` when
   uncertain, so the effect id is what keeps the old runtime's late answer
   bound.
3. **Host occupancy is not a `migrate` argument.** The store stamps it on
   every input (`Input::host`), and stamps it on Genesis in step 3. The
   rehearsal shows the core reading it.
4. **`rework_history` reasons are not attached to visits.** Only counts
   survive, because the visits written from `back_edges` don't know which
   reason went with which pass. The legacy log keeps the text.
5. **Visit order across stations is unknown.** The old item counts back
   edges per step. The policies count only per station, so counts are
   enough.
6. **An archived batch keeps the old group's name** (`landing`), and its
   crossing is at `{station}.push` if it pushed.
7. **Quarantined work:**
   - A Blocked item at a wait or decision station is quarantined. Nothing
     runs there for it to wait on.
   - A CI rerun whose request is in flight is quarantined. Its old answer
     means "the rerun went out", which the pipeline would read as CI's
     verdict.
   - An uncertain stop is quarantined (two items in the real snapshot).
8. **A Cancelling group is refused.** The pipeline stops and closes tokens,
   not batches. A Cancelling ticket already closed as landed is refused too,
   because a stopped token closes as cancelled.
9. **A `ticket/v1` item's candidate is dropped.** It is the tracker
   template's pin, not a commit.

## Differences in replay, as the design intends

- **Batching regroups.** Two or more tickets may be waiting at the join for
  the old orchestrator's `Queue`. The pipeline already has them queued at
  the batch station: 11.3 removes `Queue`, and arriving at a batch station
  queues. So the pipeline forms one batch where the old run queued them one
  by one. The same tickets land, in the same order, with the same builds.
  Seventeen cuts end this way, and the test accepts them only when at least
  two tickets were queued at a batch station at the cutover.
- **A released CI rerun gets a fresh allowance.** This happens in one
  scenario, "a second red after the rerun stops the landed group", at its
  one quarantined cut.
  - The cut catches a rerun in flight, which is quarantined. A person
    releases it, and the CI look starts a new visit.
  - The station's `again` policy counts per visit, so it allows one more
    rerun, and the second red reruns and lands.
  - To keep the old limit, a release would have to carry the visit's rerun
    count. I'd rather the person releasing it decides. **Owner: say if you
    want it carried.**

## The snapshot fixture

`sanitize.py` reads `.claude/agent/reports/pipeline/state-snapshot.json`.
It keeps only the old `State`, so no configuration, notices or places come
across.

What it changes:
- Ticket keys become `t-N`.
- Every other string becomes a salted-hash stand-in, so equal strings stay
  equal. The exceptions are step ids, enum variant names, group ids,
  template sources such as `ticket/v1`, and 40-hex commits, which become
  other 40-hex values.
- It asserts that none of `/Users`, the user name, the GitHub handle, the
  product name, `ov-` or `@` survives. I also listed every plain string left
  in the output by hand.

What it keeps:
- every live group and its members;
- every ticket that isn't Failed or at rest;
- three of each other (phase, step) kind;
- the last 30 archive entries.

`migrate_snapshot.rs` asserts where all 66 items land:
- the group mid-gate, working at `join.gate-0` with its 3 members;
- 21 queued at join: 16 waiting, plus the 5 of a waiting group, in that
  group's order;
- 3 working and 2 uncertain at build;
- 3 uncertain and 2 quarantined at provision;
- 9 held `StepFailed`;
- 9 tracked tickets at rest;
- failed groups with no tickets archived as history.

The core then takes its first tick, and the run replays.

There is no legacy log in the snapshot, so its queues fall back to outbox
order and each group's own order.
