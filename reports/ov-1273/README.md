# ov-1273: coding rules on the pipeline core

Step 1b of the pipeline redesign (docs/engine-pipeline.md, section 10 step 1).
The coding adapter's pure half, `coding/v1`, now runs on the pipeline core
from ov-1266. It is still unwired: nothing in `crates/daemon`, `crates/cli`
or `crates/store` changed, and the old reducer is untouched.

## What landed

| File | Lines | Holds |
|---|---|---|
| `crates/workflow/src/adapters/coding.rs` | ~420 | `CodingV1`, which implements `AdapterRules`. It holds the catalog, the order rules, `start`, `observe`, `suspects` and `project`, plus the adapter's data (`Data`), the facts an answer carries (`Facts`) and the effect data (`Work`) |
| `crates/workflow/src/adapters/coding_templates.rs` | ~140 | `layout(&Plan, &config)`: the four templates as schema-3 layouts, built from the same `Plan` as `templates::build` |
| `crates/workflow/src/adapters/mod.rs` | 10 | `linked()`: every adapter linked into this build, for `pipeline::step` |
| `tests/coding_layouts.rs` | ~130 | Each template validates. Seven edits that the old validator refused are refused here too |
| `tests/coding_walkthroughs.rs` | ~380 | 8.1 and 8.2 end to end, each finishing with a replay |
| `tests/coding_differential.rs` | ~490 | 28 scenarios run through the old reducer and the new core |

### Three small, neutral changes to the core

1. **`View::memberships`.** `start` for a fold has to put each member's
   candidate into the effect's data (3.3), but `View` only carried member
   keys. It now carries the batch's `Membership`s; a token's view gets an
   empty slice.
2. **`observe` takes the adapter's config.** The CI rerun rule reads its job
   list from config, just as `catalog`, `validate` and `start` already read
   theirs.
3. **Batch forming.**
   - A batch station with no window now forms as soon as it has room, from
     whatever is queued, up to `size`. ov-1266 formed full batches only, so a
     `size: 8` landing would have waited for eight tickets. The old engine
     sends a queued group as soon as the landing is free, and 4.4 says "the
     next batch forms behind".
   - The new `form.free_on_crossing` lets a batch give up its place in the
     batch station's pool once it crosses a `repeat: once` boundary. This is
     ov-1209's gate-during-CI: after the push, the next batch folds and gates
     while this one waits for CI. A crossed batch can never go back before
     the push, so this is safe. It is opt-in; 8.4's issues don't set it.
   - The vocabulary test still passes.

## Where each rule moved from

| Old | New |
|---|---|
| `StageKind` landing and PR verbs, `emits_unrouted` (definition.rs) | `Op` and its `labels`. A failed check or gate answers `red`, as in 8.1. A refused push answers `refused` (old `base_moved`) |
| `lands()` | `.push`, `.land` and `.merged` are outward, `repeat: once`, on boundary `coding:target:<ref>`, with `passed` as the crossing label |
| `GroupDef::exclusive` | `.fold`, `.gate`, `.group_review`, `.push` and `.land` hold `coding:target:<ref>` |
| `validate_routes` | `order_rules` in coding.rs:<br>• landing verbs only inside a batch, ticket verbs only outside one<br>• a pull request is opened before it is readied or merged<br>• `changes_requested` goes to a build<br>• a failing gate can't fall forward toward a push<br>• every station that needs a candidate has one on every path<br>• every path into a push passes a gate after the last change |
| `evidence_matches` | `observe`: a judging step's evidence must name the candidate it was given |
| `take_route`, `makes_candidate` | `observe`: build, base, fold and merge make the next candidate (`Data::candidate`) |
| The push rule (only a gated candidate) | `start("push")`: `Data::gated` must equal the candidate, or a visit since the candidate was made ended `skipped` |
| `Outcome::FoldConflict` | `fold` → `conflict`, with `Facts::conflict`. `suspects()` gives the named members, or every member when none are named |
| `GroupFailure` | `Facts::failure` → `Data::failure` on a red check, gate or CI, or a rejected group review |
| ci.rs `rerun` and `CiRetry` | `observe` on `ci`:<br>• red with every failed job listed in `config.ci_rerun.jobs` returns `again`<br>• the station's `again` policy counts it<br>• the look after it must name the same run at a newer attempt |
| Landing infrastructure gets 3 attempts (reducer.rs) | `attempts: 3` on the template's base, fold, gates and CI |
| Split on a failed gate | `on_reject: bisect` (4.5) |
| Join `rejected` limit 1 (2 for the rehearsal) | The batch's `returns` policy |
| Review back-edge limit 2 | The station's `rework` policy, default 2 |

## The differential test

### How it works

The old tests don't keep logs; each one builds its inputs inline. So each
scenario restates one existing test as a **world**: which builds fail the
gate or conflict at the fold, how often the remote refuses a push, what CI
and the reviewers answer, and when the orchestrator cancels, sends back or
retries.

1. The old reducer runs the world on `templates::build(plan)` and records
   its log. The test checks that the log replays to the same digest.
2. The new core runs the same world on `coding_templates::layout(plan)`,
   with `linked()`, and must replay too.
3. Both drivers take turns the same way: tickets in order first, then the
   oldest landing group or batch.
4. The test compares three things:
   - each ticket's end: done, cancelled, held at a station, or still at one;
   - the landings: which tickets each push carried, in order;
   - how many times each ticket was built.

Two mutations turned it red. Turning off the CI rerun rule fails the "listed
CI failure" scenario. Making `skipped_since` always false fails the 8.1 skip
walkthrough.

### Result

28 scenarios: 24 match exactly. The other four differ only in the ways
listed below, and each difference is intended by the design.

1. **A push refused three times.**
   - Old: the push's back-edge limit (2) is spent, the group fails, the
     member goes back to build, and it lands on a second build.
   - New: `push.rework` is reached and the batch holds `PolicyLimit` at
     `join.push`.
   - Why: "a policy never closes a token. The worst it does is hold it"
     (4.4).
2. **A base that times out.**
   - Old: `timeout` routes to `Fail`, the group fails, and the member is
     rebuilt.
   - New: the batch holds at `join.base`.
   - Why: 8.1's trace says a base `timeout` holds, because "the target
     hasn't moved in 3 h: something is wrong".
3. **A landing step that fails safely a third time.**
   - Old: the group fails, the member goes back to build, and it lands on a
     second build.
   - New: the batch holds `StepFailed` at `join.fold`. Two safe failures
     still retry and land, the same in both.
   - Why: when `attempts` is reached the work holds `StepFailed` (4.4), the
     same rule as 1.
4. **A diagnosed fold conflict in a real template.**
   - Old: the healthy member is rebuilt for nothing. The template's fold
     step has back-edge limit 0, so after returning the conflicting member,
     retrying the healthy half from `base` exceeds the limit, the group fails,
     and every member goes back to build.
   - The existing test, `a_diagnosed_fold_conflict_returns_only_bad_members…`,
     passes only because its fixture sets the fold's limit to 3 by hand.
   - New: the named member leaves along `returned` and the rest carry on
     (4.1).
   - **This is a bug in today's shipped templates**, worth knowing before the
     cutover. It doesn't block it, because the cutover replaces them.

Some differences don't show in outcomes: the old reducer splits and re-runs
from `base`, while the new core bisects on the failing gate. Both land the
same tickets in the same order. The cost difference is 4.5's point and isn't
counted here.

## The walkthroughs

### 8.1 (fully agentic)

The layout is as written, except as noted below. Four tests cover it:

- **Build, review, route and decide.**
  - A batch of one forms at once.
  - The next batch forms when the push crosses.
  - A rerun on a flaky CI job, where a stale attempt is refused `Evidence`.
- **Fold, gate and push.**
  - A fold conflict naming one member returns it; the rest carry on.
  - A bisect on a red gate finds the failing member.
  - A refused push refolds.
- **Skipping gates.** The orchestrator skips both gates and the push still
  starts.
- **Holds and settling.**
  - A crossed push that came back `Uncertain` is settled with evidence.
  - A red CI holds, and `Release` to build is refused `Crossed`.
  - Review holds after two rejections; the release records
    `overrode: review.rework`.

### 8.2 (human pull request review)

One test covers it:

- pr-ci red goes back to build.
- `changes_requested` goes to build, with the comments as the brief.
- Skipping a person-only station is refused `PersonOnly`, and two approvals
  are required.
- `merged` crosses `coding:target:main`; afterwards a send back is refused.
- The release manager approves, and production crosses
  `deploy:production`.
- A pull request closed unmerged holds, and the orchestrator releases it to
  build.

### How the layouts differ from the design's YAML

- **Patience is `patience_ms`.**
- **8.1's `form.order` is left out.** The queue is already in priority order
  (ov-1266 report, item 11).
- **8.1's `land` sets `free_on_crossing: true`.**
- **8.2's deciders.** `group:` deciders and `needs: { total, from }` became
  two named people with `needs: 2`. The core has no group membership yet
  (ov-1266 report, item 9).
- **8.2's `source: coding.pr_reviews`** became the reviewers' own `Decide`.
- **8.2's `command/v1` deploy adapter** is a test fixture. The real adapter
  is a later ticket (10.6).

## Not built, or changed from the old behavior on purpose

1. **The conflict brief.**
   - Old: a returned member's brief was `conflict_brief`, with the files.
   - New: the core writes "returned from join.fold: conflict". The files stay
     in the batch's data (`Data::conflict`).
   - Why: an adapter can't write a token's brief. The core could later pass
     the adapter's text through `suspects`.
2. **Adopting the landed commit.**
   - Old: a member's candidate became the pushed commit (`Join { adopt }`).
   - New: the token's own data keeps its build's candidate. The pushed commit
     is the crossing's `artifact`, and `Data::pushed` on the batch.
3. **Suspects for a red gate.**
   - The pure half can't read paths, so the execution half names suspects in
     `Facts::suspects` (paths against failing tests), and `suspects()`
     filters them to the batch's members.
   - Under `bisect` they aren't needed.
4. **One review command per layout.**
   - The command reviewer's argument is in `config.review`, because a station
     has no arguments of its own.
   - Check and gate commands are named by index (`coding.gate:0`), from
     `config.checks`.
5. **Path-route labels.**
   - Path routes answer `passed` and `needs_review`, as in 8.1, instead of the
     host's `push` and `ask`.
   - A config can list other labels in `route.labels`.
6. **Resources on command stations only.**
   - The core checks resources only when a command or agent station starts,
     so `base` and `ci` (waits) hold none.
   - The landing pool, together with `free_on_crossing`, keeps batches in
     order. The resource also keeps two workspaces from landing on the same
     target at once (`busy_resources`).
7. **`coding.pr_reviews` is not an op.**
   - A decision station runs no op.
   - Reviews from GitHub arrive as `Decide` inputs once the runtime has
     adapter sources (step 3).
8. **Retry and send-back are core interventions.**
   - Retry is `Release` of a held token.
   - Send-back is `Send` to build. Both are unlimited (5.4).
   - The differential test checks that both lead to the same outcomes.

## What step 2 (`migrate`) needs from this

- **Map each item's fields into `Data`.** `candidate`, `tree`, `pushed` and
  `ci_retry` become `Data` fields, and a group member's candidate goes into
  `Membership::data` as `Data { candidate }`. All of these are public in
  `adapters::coding`, keyed `coding`.
- **Template station ids keep the old step ids** (`join`, `rehearse`,
  `gate-0`). So `Item::stage` maps to a station by id, and a group's step
  becomes `join.<step>`.
- **Group `back_edges` don't migrate.** Rework and returns are counted from
  visits now, so `migrate` should write visits for the back edges already
  taken, or accept that the count starts again.

## Checks run

- `cargo nextest run -p farcooler-workflow`: every test passes, including
  `pipeline_vocabulary`.
- `cargo clippy --workspace --all-targets -- -D warnings`: clean.
- Each new file is under 500 lines.
