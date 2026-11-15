# ov-1266: pipeline core, neutral and unwired

Step 1a of the pipeline redesign (docs/engine-pipeline.md, second pass).
The core is a new pure module, `crates/workflow/src/pipeline/`. Nothing in
the daemon, the CLI or the store uses it, and the old reducer is unchanged.

## What landed

| File | Lines | Holds |
|---|---|---|
| `types.rs` | ~265 | The core types of 2.3: `Pipeline`, `Station`, `Retired`, `Token`, `Batch`, `Membership`, `Crossing`, `Archived`, `Effect`, `Location`, `Work`, `Hold`, `Visit`, `Outcome` |
| `layout.rs` | ~330 | Schema 3: `Layout`, `StationDef`, `Policy`, `Form`, `OnReject`, `Move`. `Graph` flattens a layout and owns routing (4.1), reachability and `validate` |
| `adapter.rs` | ~77 | `AdapterRules` (3.2), `OpSpec`, `Outward` (`Repeat`, `Dedupe`, crossing labels), `StartPlan`, `Applied`, `View`, `Adapters` |
| `step.rs` | ~250 | `Input`, `Grant`, `Command`, `Refusal`, `Transition`, and `step` |
| `flow.rs` | ~450 | Arrivals, starts, answers, routing, the `rework` and `attempts` policies, holds, closing, and the migration rule |
| `batch.rs` | ~200 | Forming by size or window, `finish`, `returned`, `on_reject` (bisect, return suspects, hold), unbatching |
| `control.rs` | ~230 | Every intervention of 5.1, with stopping (5.2) and settling (5.3) |
| `publish.rs` | ~230 | Publish (7): renames, replacements into retired stations, unbatching, stranding, `after` |

Tests live in `crates/workflow/tests/pipeline_*.rs`. The fixture adapters
(`test/mail`, `test/calendar`, `test/files`, `test/cms`) exist only there:

- `pipeline_walkthroughs`: 8.3 (the assistant) and 8.4 (the weekly brief),
  end to end.
- `pipeline_control`: ordering and reorder; station, pool and host capacity;
  a policy limit that holds, with the override recorded in the visit;
  `attempts`; hold, pull, repeated send-back and skip with a reason;
  `skip: person`; the once-only rule for `Send` and `Pull`; cancel and
  reopen.
- `pipeline_publish`: a publish over queued, held and working tokens; a
  rename, and a removal with a move into a retired station; stranding;
  the migration rule with `after` set to a station and to `hold`.
- `pipeline_vocabulary`: the core-vocabulary check.

Every test ends with a replay that must reach the same state, the same
effects and the same digest. Four mutations of the core turned tests red:
no rework limit, no crossed check, a retired station following its old
route, and a `branch` field on `Token`.

## Where the code departs from the design, and why

1. **`step` takes the adapters:** `step(&Pipeline, &Input, &Adapters)`.
   The design's `step(&Pipeline, &Input)` assumes adapters are linked into
   the reducer. Here they're passed in, so the fixtures can live only in
   tests. Step 1b can wrap the call with the linked set.
2. **`AdapterRules` methods take a `View`, not a bare `Subject`.**
   `start`, `observe` and `suspects` need the visits, crossings, evidence and
   data that 3.2 says they "see". `catalog` takes the adapter's config,
   because a boundary is "a function of the op and the config".
3. **`StationId` is a `String`.** An inner station's id is
   `<batch>.<inner>` (`issue.editor`), so `.` is reserved in layout ids as
   well as `/`. There's no `(workflow, station)` pair yet, because there's
   one workflow.
4. **A station's default capacity is "no limit of its own".** The pool
   binds, so the effect matches "default: its pool's". A frozen copy of the
   pool's number wouldn't follow `SetCapacity { pool }`. A publish resets a
   station's capacity to the layout's value, which undoes any earlier
   `SetCapacity { station }`.
5. **An unknown answer stays `Working { uncertain: true }`.** The design
   uses a separate `Held { Uncertain }`. Keeping the step working keeps its
   binding, place and resources, which the design requires. A later
   `Observe` for the original effect, or a `Settle`, ends it. Interventions
   on uncertain outward work are refused `Stopping`, so they aren't
   recorded and replayed later as 5.3 rule 2 describes.
6. **`Settling` answered `Uncertain` drops the request.** The design keeps
   the intervention's `then` in the hold. This version has no field for it
   yet.
7. **`Hold` gained `then: Option<Target>`.** It records where the work was
   going when a policy held it. `Release` with no destination sends it
   there, so overriding `review.rework` actually sends the token on, which
   is the design's intent.
8. **Stopping work isn't kept in its destination queue.** `Then` replaces
   `Box<Location>`. It says where to go and who asked, and the work joins
   the queue (by priority) when the stop answers. The effect is the same
   except for first-come order between equal priorities.
9. **Decisions:** an `approved` verdict needs `needs` approvals, and any
   other verdict acts at once. `group:` deciders aren't supported yet (they
   need a membership input), and `needs: { total, from }` is just a count.
   A verdict on a batch can name suspects (`Decide { suspects }`), which is
   how the 8.4 editor marks articles.
10. **Windows are numeric:** `{ every_ms, at_ms }`, not `"daily 08:00"`. A
    calendar string needs a time zone, and that belongs to whoever writes
    the layout, not the reducer.
11. **Batches and pools:** a batch holds one place in its batch station's
    pool for its whole life. Inner stations don't inherit that pool.
    `form.order` is left out because the queue is already in priority
    order.
12. **`Send` may target any top-level station,** forward as well as back,
    subject to the crossed check. 5.1 says "before where it is". Nothing
    is gained by refusing a forward send, and the orchestrator can do
    everything.
13. **Crossed holds sit at the destination station,** not "where it is" at
    a retired station. Release in place is refused `Crossed`, so this is as
    safe, and a retired station is deleted as soon as nothing runs in it.
14. **Publish, not yet built:** the preview (7.5) and a stored report; the
    check that `boundary: new` is given when a publish leaves no station
    on a boundary some token crossed (only replaced outward stations are
    checked); the open questions O1–O5 (no special handling); and decision
    stations keeping their verdicts when deciders change (they move like
    queued work instead). A queued token at a removed station moves by
    priority, not by its original arrival revision.
15. **Not built:** followers and `Riding`, dependencies beyond "skip until
    done", `Track` columns beyond `Resting`, the epoch and fences of the
    old store, the `PersonOnly` list of section 9 (only `skip: person`),
    and size limits on inputs.
16. **The vocabulary check is a Rust test, not a `scripts/` check.** The
    ticket keeps changes inside `crates/workflow`. It checks every
    identifier in the module, not only declarations, with two exceptions:
    `BTreeMap`/`BTreeSet`, and calls to `Vec::push` (`.push(`). A field,
    type or function named `push` is still caught.

## What step 1b needs from this

- **A `coding/v1` `AdapterRules`** in the workflow crate, not in tests.
  - `catalog(config)` should give `coding.push`, `.land` and `.merged`
    `Outward { boundary: "coding:target:<ref>", repeat: Once, crossing:
    ["passed"] }`, with `refused` (or `closed`) not crossed.
  - `.base` through `.ci` take the resource `coding:target:<ref>`.
  - `start` needs today's push rule (5.3, 3.3). The batch's data says what
    it was gated on, and a visit at the gate station that ended `skipped`
    means the orchestrator skipped the gate. `View::visits` makes this
    readable.
  - `observe` maps fold conflicts and CI results to labels. A CI rerun is
    `Applied::again`, counted against the station's `again` policy.
  - `suspects` reads paths against failing tests.
- **The four templates as schema-3 layouts,** validated by
  `Graph::validate` with `coding/v1` linked. 8.1 should pass as written:
  the once-rule follows `finish` only by the batch's `passed` route, so
  `land.returned → build` doesn't make `push` reachable from its own
  crossing.
- **Linked adapters for replay:** wire `Adapters::new(&[&CodingV1, ...])`
  where the daemon will call `step`.
- **Decisions to make or confirm** (items 5, 6 and 9 above): whether
  uncertain work should be a visible `Held`, where `Settling` keeps its
  `then`, and how group deciders get membership (an input from the store,
  like host occupancy).
