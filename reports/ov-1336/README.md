Move **Needs You aggregation** first: the bounded merge/count/group slice occupies **34 Swift lines (shared by Mac and iOS), 20 Kotlin lines and 31 TypeScript lines**. Its copies already disagree on urgency order and duplicate PR reviews. This is only 85 physical lines, but a missed ask or inflated badge matters more than cosmetic drift; a pure, clock-free presenter is a suitable secondary task while landing speed remains primary. Keep daemon classification and native transport outside this move.

# ov-1336 — what is still written separately

## Evidence and counting

Surveyed the checked-out main snapshot `3643128bb44ad59fdbe80ea5b9d8785ca56fa91a` (HEAD equals `origin/main` at survey time, 2026-11-18). Local `main` is older (`3f78754f`); no fetch or checkout was performed. Research only: no builds, tests, lint execution, baseline regeneration or product edits.

Paths below use these unambiguous prefixes:

- **AK/** = `apps/shared/AgentKit/Sources/AgentKit/`
- **M/** = `apps/macos/Sources/FarCooler/`
- **I/** = `apps/ios/FarCooler/`
- **K/** = `apps/android/app/src/main/java/com/farcooler/model/`
- **W/** = `services/web/src/client/`
- **C/** = `crates/client/src/`

Counts are inclusive physical source spans (`end - start + 1`), including comments, declarations and blank lines inside each span. They measure the footprint of separately implemented policy, **not identical executable LOC or a promise that every line can be deleted**. Table totals sum those footprints; platform boilerplate differs. Whole-file counts are marked “whole.” The first slice counts only the specified functions, not all of Needs You. Apple shared code counts once; Mac/iOS-specific copies count separately. Generated wire types are excluded.

Read `scripts/shared-core-baseline.json`: 10 rust-copy entries, 113 wire-word entries, 59 twin files, 590 ratchet files, 6 exemption-file entries. These are inventory counts, not duplicated LOC. Read `scripts/shared-core-lint.py:1–40,143–205,210–239`: rust-copy/wire-word find repeated strings and wire switches; twin uses names/header clues; ratchet counts user-facing literals. It cannot prove semantic equality, misses differently named copies, and excludes UI-importing files from twin logic. Therefore I also read the native views and web renderers, followed runner-data transformations, and compared branches, defaults, ordering, unknown values, actions and time boundaries. Findings below are static source observations, not runtime reproductions or an exhaustive proof over every runner payload.

The brief's stalled-slice premise is stale: **ov-1048 did land task-usage wording** (`C/usage_json.rs:1–8`; `AK/TaskUsage.swift:74–92`; `K/TaskUsage.kt:12–26`). Native formatting helpers remain, but the task Usage section consumes Rust `said`; do not rank its old implementation as three active copies.

## Ranked candidates

Rows are ranked by summed duplicated-policy footprint. Drift rank is independent: 1 is most urgent. Sizes include Rust types/presenter, native and wasm exposure, adapters, fixtures and removal of the old branches: small = one narrow pure rule; medium = several rules/adapters; large = multiple screens/data dependencies. Estimates are scope classes, not elapsed-time commitments.

| LOC rank | Candidate; Swift files/spans and lines | Kotlin files/spans and lines | TypeScript files/spans and lines | Total | Drift rank / move |
|---|---|---|---|---:|---|
| 1 | Plan rulings: `AK/PlanRulings.swift` whole **172** | `K/PlanRulings.kt` whole **162** | `W/plan.ts:132–147,175–204` + `W/plan-view.ts:299–321` **69** | 403 | 4 / medium |
| 2 | Needs You action policy: `M/NeedsYouView.swift:64–101` + `I/NeedsYouScreen.swift:977–1038` **100** | `K/NeedsYou.kt:319–348` **30** | `W/needs.ts:185–206` **22** | 152 | 2 / medium |
| 3 | Task record projection: `AK/TaskBoardModel.swift:799–856,897–909,931–951` **92** | `K/BoardSummary.kt:164–180` **17** | `W/detail.ts:51–78` + `W/detail-view.ts:190–198` **37** | 146 | 3 / large |
| 4 | Task status/note words: `AK/TaskBoardModel.swift:61–71,710–751` **53** | `K/TaskBoard.kt:33–54` + `K/BoardSummary.kt:141–159` **41** | `W/detail.ts:81–132` **52** | 146 | 5 / small |
| 5 | **Needs You aggregate**: `AK/NeedsYou.swift:308–325,343–355,419–421` **34** | `K/NeedsYouItems.kt:217–234` + `K/NeedsYou.kt:114–115` **20** | `W/fleet.ts:125–148` + `W/needs.ts:103–105,155–158` **31** | 85 | **1 / medium** |
| 6 | Answer-refusal words: `AK/ShellNavigation.swift:731–748` + `M/DaemonClient.swift:1452–1463` **30** | `K/NeedsYou.kt:355–363` **9** | `W/copy.ts:160–188` **29** | 68 | 6 / small |
| 7 | Relative-time words: `AK/TaskBoardModel.swift:321–334` **14** | `K/TaskBoard.kt:226–239` **14** | `W/needs.ts:142–151` **10** | 38 | 7 / small |
| 8 | GitHub-link eligibility: `AK/NeedsYou.swift:526–535` **10** | `K/PlanPrStage.kt:92–98` **7** | `W/needs.ts:163–173` **11** | 28 | 8 / small |

The action row counts the kind-to-action decisions embedded in SwiftUI, not an entire view. Its presentation wrappers make the Swift count an upper bound. The record row compares overlapping task-note behavior, not a claim that Android has all the Mac's detail features. Neither row includes the separately counted status or age helpers.

What each move would expose from `crates/client`:

| Candidate | Rust view-model boundary and reason for risk rank |
|---|---|
| Aggregate | New `needs_you_view::aggregate`: ordered runner-qualified rows, group membership, counted total and runner/workspace/repository counts. Highest risk: cross-runner ordering and dedup already diverged. Detailed contract below. |
| Actions | `NeedsYouRowView { actions: [{kind, label, target, option, primary, destructive}], overflow, unsupported_reason }`, taking explicit host capabilities/scope. Native code executes descriptors; daemon still authorizes. Risk: adding a new kind can silently remove an action on one platform. |
| Record | `TaskDetailView { notes, unreadable_count, blocks }`; note fields include `kind_label, quiet, body, byline, timestamp_ms, paired_with, rejected, answered_for`. Build from `tasks_json` output plus viewer/member context; leave polling and note submission native. Risk: unknown notes and actor attribution are lost differently today. Large because phone and desktop feature coverage differs. |
| Rulings | Extend `plan_view::PlanView` with ordered `open_rulings`, `past_rulings`, title, settled text, touches, accessibility text and action availability from viewer/orchestrator context. Existing `plan_view.rs:15–17` explicitly leaves rulings native. Rank 4: significant surface, but less immediate than losing an ask or a record entry. |
| Status/note words | A small shared words response `{status_label, note_kind_label, quiet, known}` keyed by wire values and existing Apple/Material style. Reuse it from task-detail and board presenters. Unknown behavior must be explicit rather than hiding information behind an empty label. |
| Refusals | `AnswerNotice { text, outcome_uncertain, retry_allowed }` from normalized failure kind/code/what and display names; preserve browser-specific grant wording through explicit context. Known refusal strings agree; fallback handling has diverged. |
| Time | `RelativeTimeView { text, next_change_ms }` from optional timestamp, supplied `now_ms` and an explicit surface/style policy. All clients schedule redraw locally. Mostly cosmetic, bounded and easy to pin at thresholds. |
| Links | `ExternalLinkView { url?, label? }` using one raw-authority policy before URL normalization. Reuse core thread-link rules where compatible. Lower priority because the observed difference admits the same GitHub host rather than a foreign host. |

## Observed disagreements (and intentional platform differences)

These are all disagreements found in the compared slices. A difference in feature support is identified separately from an accidental inconsistency; sharing must preserve explicit capability/style inputs rather than silently enabling features.

1. **Aggregate order and PR dedup:** Swift sorts known kinds before unknown, then rank/runner/item ID, and drops repeated PR-review IDs (`AK/NeedsYou.swift:318–325`). Kotlin does the same (`K/NeedsYouItems.kt:217–227`). Web appends each runner's list in member iteration order (`W/fleet.ts:134–148`), with neither global sort nor PR dedup. Its parser omits `rank` entirely (`W/needs.ts:72–96`). Thus a later runner's urgent ask can follow an earlier runner's low-priority item, and one PR can count twice. Counts excluding FYI otherwise agree (`AK/NeedsYou.swift:353–355`; `K/NeedsYouItems.kt:157,233–234`; `W/needs.ts:103–105`).
2. **Old runner/read coverage:** Swift can derive blocked-only items with retiered ranks (`AK/NeedsYou.swift:492–512`); Android explicitly uses fleet-derived items while a reading is absent (`K/NeedsYou.kt:156–166,208–212`). Web clears items and labels the state old when `runner_too_old` is returned (`W/needs.ts:287–290`). This is a fallback/support difference, beyond the recommended first slice.
3. **Actions and support:** Mac and iOS offer a typed decision answer when no choices exist (`M/NeedsYouView.swift:84–94`; `I/NeedsYouScreen.swift:999–1010`); Kotlin returns `Write` (`K/NeedsYou.kt:344–347`). Web only maps supplied actions, so offers no typed decision fallback (`W/needs.ts:185–205`). Swift/Kotlin show sign-in (`M/NeedsYouView.swift:68–69`; `I/NeedsYouScreen.swift:1016–1023`; `K/NeedsYou.kt:338`); web expressly cannot press it (`W/needs.ts:182–191`). Native review fallback is Open/Review (`M/NeedsYouView.swift:96–101`; `I/NeedsYouScreen.swift:1028–1037`; `K/NeedsYou.kt:331–332`); web requires an actionable supplied Open and a listed terminal (`W/needs.ts:186,199–203`). These are capability/fallback differences, not evidence that browser authentication should move into a presenter.
4. **Decision option identity:** Mac builds decision descriptors from `action.title` (`M/NeedsYouView.swift:86,93`), whereas iOS sends `action.id` (`I/NeedsYouScreen.swift:991`), and web sends `action.id` (`W/needs.ts:190`). They agree only when the runner keeps ID and title equal. Preserve the wire ID in a shared descriptor.
5. **Record unknown kinds:** Swift skips unknown note kinds but counts them (`AK/TaskBoardModel.swift:933–950`); Kotlin drops them without a count in `TaskNotes` (`K/BoardSummary.kt:170–178`); web retains any note with a body (`W/detail.ts:70–72`) and defaults its kind label to Note (`W/detail.ts:126–128`). The same new runner note is therefore warned about, absent, or shown depending on client.
6. **Record body and writer:** Swift rewrites a recognized `moved from … to …` body and resolves user/manager/member/answered-for bylines (`AK/TaskBoardModel.swift:799–855`). Android's sampled model retains only kind/time/body (`K/BoardSummary.kt:162,173–177`); web draws raw actor and body (`W/detail-view.ts:194–197`) from `W/detail.ts:72`. Swift also groups question/answer pairs newest-first (`AK/TaskNoteFeed.swift:12–32`), while web iterates received notes in order (`W/detail-view.ts:190–197`). Do not call Android's Unread projection a full desktop record implementation.
7. **Task words:** Swift and Kotlin call `cancelled` “Canceled” (`AK/TaskBoardModel.swift:69`; `K/TaskBoard.kt:40`), web says “Cancelled” (`W/detail.ts:95–96`). Native note labels are Comment, Status Change/status change, Subagent (`AK/TaskBoardModel.swift:734–738`; `K/BoardSummary.kt:147–151`); web says Note, Status, Agent (`W/detail.ts:116–125`). Apple's title case versus Material's sentence case is intentional (`AK/TaskBoardModel.swift:57–60`; `K/TaskBoard.kt:34–40`), not a reason to retain separate policy.
8. **Ruling order:** Swift/Kotlin open rulings preserve input order (`AK/PlanRulings.swift:85`; `K/PlanRulings.kt:105`); web sorts descending ruling number (`W/plan.ts:176`). Past decisions all sort by settled time then number (`AK/PlanRulings.swift:89–92`; `K/PlanRulings.kt:108–110`; `W/plan.ts:179–180`).
9. **Ruling text and scope:** Swift and Kotlin print the entire reversed SHA (`AK/PlanRulings.swift:141–144`; `K/PlanRulings.kt:142–145`); web truncates to eight characters (`W/plan.ts:199–200`). Native section titles depend on owner/member viewer (`AK/PlanRulings.swift:110–111`; `K/PlanRulings.kt:119`), but web uses constant Decided For You (`W/plan.ts:184–186`; `W/plan-view.ts:355`). Native touches include card keys plus theme (`AK/PlanRulings.swift:150–154`; `K/PlanRulings.kt:149–153`); web parses no ruling cards (`W/plan.ts:134–146`) and shows only theme (`W/plan-view.ts:312`). Web ruling actions are Keep only (`W/plan-view.ts:314–317`), versus native Keep/Reverse/Discuss vocabulary (`AK/PlanRulings.swift:115–119`; `K/PlanRulings.kt:122–125`): support gap, not permission to enable new writes.
10. **Refusal fallbacks:** Named `what` cases agree across the four copies (`AK/ShellNavigation.swift:733–739`; `M/DaemonClient.swift:1455–1459`; `K/NeedsYou.kt:356–361`; `W/copy.ts:161–171`). Unknown failures differ: iOS delegates to `RunnerRefusal` or warns the answer may not have arrived (`AK/ShellNavigation.swift:742–747`); Mac/Android use “Couldn’t send that answer” (`M/DaemonClient.swift:1461`; `K/NeedsYou.kt:362`); web distinguishes connection loss, scope, unsupported and removed item (`W/copy.ts:173–187`). A unified notice needs outcome uncertainty as well as text.
11. **Time:** Native helpers say `5m ago`, `2h ago`, switch to months at 30 days and years at 365 (`AK/TaskBoardModel.swift:321–334`; `K/TaskBoard.kt:226–239`). Web says `5 min`, `2 hr`, stays in days indefinitely and capitalizes Just now (`W/needs.ts:142–150`). iOS Needs You uses the native helper (`I/NeedsYouScreen.swift:878–887`); web uses its helper for both Needs You and task notes (`W/needs-view.ts:38`; `W/detail-view.ts:194`). Native helpers otherwise match on clamping future timestamps, floor rounding, and the 11-month cap.
12. **GitHub links:** Kotlin checks the raw authority equals github.com, so rejects an explicit port (`K/PlanPrStage.kt:93–96`); Swift rejects any parsed port (`AK/NeedsYou.swift:527–531`). Web checks `new URL(link).port === ''` (`W/needs.ts:165–172`), after URL normalization: `https://github.com:443/x` has its default port removed and passes. All three reject foreign hosts, credentials and whitespace/backslash by their shown guards. Pin raw-authority rules before consolidating.

## Agents, workflow/plan cards and settings: avoid false triple counts

| Remaining surface | Measured Swift / Kotlin / TS footprint | Assessment and proposed Rust exposure |
|---|---|---|
| Fleet agents list | `AK/RunningAgents.swift` whole **361** / `K/RunningAgents.kt` whole **245** / **0** equivalent fleet-agent implementation found | A substantial **two-copy** move (606 lines, medium), not three. Expose `AgentListView { rows, count, waiting, dots, workspace_groups }` from fleet and optional row ledger. Swift prioritizes attention marks before taking three dots (`AK/RunningAgents.swift:132–135`); Kotlin takes the first three counted agents (`K/RunningAgents.kt:84–90`). This real drift deserves the next native slice after aggregation. Web's plan roster is already Rust (`W/plan-present.ts:1–7`); do not count it as a third native fleet list. |
| Runner settings/devices | `AK/EnrolledDevices.swift` whole **179** / `K/EnrolledDevices.kt` whole **90** / **0** equivalent runner-device list/revoke presenter found | Another two-copy move (269 lines, medium). Expose device rows, added-by text, descendant IDs and revoke-choice copy. Native dedup/foreign filtering and ancestor-loop handling agree on inspection (`AK/EnrolledDevices.swift:46–61,77–93`; `K/EnrolledDevices.kt:31–44,75–87`); casing differs deliberately (`AK/EnrolledDevices.swift:178`; `K/EnrolledDevices.kt:67`). Swift rejects a malformed mixed-type clients array as a whole (`AK/EnrolledDevices.swift:47–49`); Kotlin filters nonobjects (`K/EnrolledDevices.kt:79`). Web `device.ts` manages browser identity/passkeys (`W/device.ts:1–18`), not `client.list` runner settings. |
| Workflow cards | Generated Rust presentation: `AK/WorkflowRuns.swift:4–5,26–30`; `W/workflow.ts:1–18`; Android generated types consumed through its workflow model | Already shared; no new three-way card move. Workflow-change review parsing remains **74 Swift / 71 Kotlin / 0 TS** lines (`AK/WorkflowChangeReview.swift`, `K/WorkflowChangeReview.kt`, whole files), a small two-copy extraction of structured review text and approval availability. Compare their parse guards at Swift 49–72 / Kotlin 43–68; no semantic disagreement found there by tracing paragraph, heading and approval-code checks. |
| Plan stack/cards | `C/plan_view.rs:1–17`; `M/Plan/PlanStore+Sections.swift:16–37`; `K/PlanViewModel.kt:12`; `W/plan-present.ts:30–51` | Already shared, including ticket/landing sections and column pipeline; keep native layout/input state. Rulings remain the triple candidate above. Cost/glance/sidebar native remnants are follow-on two-copy work unless a real TS equivalent is identified; a baseline twin entry alone does not establish one. |

For the zero-TS findings I inspected the web client inventory and searched/read `plan*`, `scope.ts`, `device.ts`, `app.ts` and `workflow*`, following their imports and presenter calls. The web has different feature coverage, so manufacturing a third implementation would distort both ranks. Conversation words/items and generated plan/wire types were excluded as prior shared slices, not ranked again.

## First slice: builder contract

Add `C/needs_you_view.rs` with a pure `aggregate(request) -> NeedsYouAggregateView`, plus the established schema-versioned JSON entry point, native FFI and wasm wrappers. Keep `needs_you_json` as the wire projection: it already supplies rank and nullable fields (`C/needs_you_json.rs:23–42`), but cannot merge runners because it receives only one runner's list. Generate the output types with the existing wire-type workflow.

Suggested input (schema 1):

- `runners: [{ runner_id: String, items: [NeedsYouItemInput] }]`; IDs are stable connection identities, not display labels. Each item preserves raw `id`, `kind`, `rank: u32`, `workspace_id?`, `repository_id?`, `concern?: { yours: bool }`, and its existing payload. Unknown kinds retain their raw word. Accept an envelope around the existing needs-you JSON rather than changing daemon/protocol types.
- Accept already-derived native old-runner items with their existing retiered ranks; deriving them, stale-read policy and connection banners remain outside this slice. Web must retain `rank` while parsing (or pass raw lists directly). Reject a missing/invalid rank rather than inventing urgency; define a typed invalid-input response. No wall clock or I/O.

Suggested output fields:

```text
NeedsYouAggregateView {
  schema: 1,
  rows: [{ key: { runner_id, item_id }, item, group: yours | others, counted: bool }],
  yours: [RowKey], others: [RowKey], split: bool,
  counted_total: u32,
  runner_counts: [{ runner_id, count }],
  workspace_counts: [{ runner_id, workspace_id, count }],
  unclaimed_counts: [{ runner_id, repository_id, count }]
}
```

Use a structured row key or an unambiguous encoding at UI boundaries; keep raw item ID separate for dispatch. Preserve item payload/action IDs byte-for-byte. Sort by `(is_unknown, rank, runner_id, item_id)`, then deduplicate **only PR-review items by raw item ID**, keeping the first (lowest rank, deterministic ties). Count after dedup; FYI is visible and grouped but contributes zero. `also` never adds a count. Missing concern means yours; `concern.yours == false` means others; split iff others is nonempty. Counts use runner-qualified workspace/repository identities, including unclaimed items only when workspace is absent and repository present. Emit zero runner totals for empty input runners; omit absent workspace/repository buckets.

This adopts the agreeing Swift/Kotlin ordering and dedup policy, deliberately correcting the browser disagreement. It does not promise full Needs You parity: browser old-runner derivation, action support, display words, age, banners, busy/notices and dispatch are separate candidates/local state. Replace only the aggregate functions cited in row 5; native UIs still choose layout and group-header casing.

Tests to add when implementing (none run for this report):

| Fixture/test | Behavior to pin |
|---|---|
| Existing `test/fixtures/needs-you.json` | Decode every existing kind and preserve redacted/missing optional payload fields; present the same runner list without changing action IDs. |
| Two runners, inverted arrival order | A later runner's lower-rank ask appears first; reversing runner/item input order yields identical rows and buckets. |
| Ties and future kind | Known beats unknown even when unknown rank is zero; ties resolve runner ID then item ID. Preserve unknown raw kind/payload. |
| Same PR on two runners | One row and one count, from lower rank; equal ranks choose runner ID. Different PR IDs remain distinct. Same non-PR ID on two runners stays two rows. |
| FYI, `also`, and concern matrix | FYI does not count; an ask plus decision in `also` counts once. Yours/others/nobody groups preserve sorted order; others-only FYI still causes split. |
| Workspace/repository isolation | Reused workspace IDs on different runners have separate buckets; unclaimed counts require no workspace; absent repository has no repository bucket. |
| Empty/derived/invalid input | Empty runners produce total zero; native derived blocks keep supplied tiered ranks; no inferred asks. Missing rank, negative/overflow rank, duplicate runner IDs and duplicate item IDs within one runner return explicit invalid-input errors. |
| Adapter goldens | One aggregate fixture consumed through native and wasm entry points and decoded by Swift/Kotlin/TS. Verify displayed total, keys, order and grouped rows; tap dispatch still receives raw runner/item IDs. |

Start with Rust pure fixtures, then expose one response shape and switch adapters, then remove the old functions and shrink the baseline only for violations actually removed. Do not update broad baseline entries merely because the new helper exists. The move is medium rather than small because global merging belongs above each per-runner Session and therefore needs an explicit callable bridge for native and browser clients.

## ov-1337 implementation — aggregation moved

The first slice is implemented in `crates/client/src/needs_you_view.rs`. `aggregate` takes runner-qualified lists without a clock or I/O. It orders known kinds before unknown kinds, then by supplied rank, stable runner ID and raw item ID; removes only repeated PR-review IDs after ordering; partitions the retained rows; and produces the counted total and runner-qualified runner/workspace/unclaimed buckets. FYI rows remain visible and grouped but contribute zero. `also` contributes no extra count. Missing concern means Yours. An others-only FYI still splits the list. Empty input runners have explicit zero runner counts.

The schema-1 JSON envelope is `{"schema":1,"needs_you":[{"runner_id":"…","items":[…]}]}`. Its response is `{"schema":1,"needs_you":NeedsYouAggregateView}` or a typed `error` word: `invalid`, `duplicate_runner`, or `duplicate_item`. Missing, fractional, negative and overflowing ranks are invalid. Duplicate runner IDs and duplicate item IDs within a runner are invalid. No urgency is inferred. The output has the proposed fields; the opaque row payload is represented as `item_json` so the established typeshare workflow can generate its type in all three languages. Rust preserves unknown payload fields and raw kind/action ID strings. Structured row keys remain separate from the raw dispatch ID.

Swift's `NeedsYouAggregate.swift` and Kotlin's `NeedsYouPresenter.kt` use the existing schema-versioned view-model bridge (`PlanViewPresenter.ask` → `farcooler_client_plan_view`, and `NativeClient.nativePlanView` → the same C entry point). That bridge already carries shared non-plan requests; it now routes this envelope to `needs_you_view::json`. The browser installs wasm's `needsYouView`, which directly calls the same function. Output types are generated with `scripts/gen-wire-types.sh`. Native adapters resolve returned keys to their original items, preserving dates, derived markers and action payloads rather than rebuilding them from presentation metadata. Swift retains a newer runner's raw unknown kind for this bridge.

`NeedsYou.merge`, native count helpers and grouping helpers, and web `Fleet.publish`/`countOf`/`groups` now adapt Rust's answer. Global badge and all-clear counts use that answer. Item counted/group flags delegate to Rust as well. Native workspace membership, implicit workspace layout and stale-workspace placement still select a surface's rows locally; Rust counts that selection. Board-column fallback totals and plan theme asks are outside this aggregation contract.

### Android and web behavior changes

| Platform / case | Before | After |
|---|---|---|
| Web, urgency across runners | Enrollment/member iteration order, with each runner's received order | One global `(unknown, rank, runner_id, item_id)` order; a later runner's urgent ask can precede an earlier runner's review. Equal ranks have stable ties. |
| Web, unknown kinds | An unknown item could precede known actionable kinds because arrival order was retained; raw kind/rank were discarded by parsing | Unknown rows sort after all known rows; raw kind and supplied rank reach Rust. Unknown rows remain visible and counted. |
| Web, repeated PR review | One row and count per runner, including two counts for the same PR | One row and count per raw PR-review ID; the lowest-rank runner wins, then runner ID for ties. Groups and workspace/runner totals reflect only the retained row. Actions still go to that runner with the original raw item and action IDs. |
| Android and web, invalid rank | Android decoded a missing rank as `Long.MAX_VALUE`; web ignored rank entirely and accepted missing/invalid values | Android requires rank on decoded wire items. Both bridges reject negative or overflowing ranks; web also rejects missing/fractional ranks during parsing. Existing read-failure handling remains native. Android items constructed locally without an explicit rank use the valid maximum `u32` rank; derived older-runner ranks are unchanged. |
| Android, incidental deduplication | A single `distinctBy` namespace mixed raw PR IDs with slash-joined non-PR row keys; a collision could discard an unrelated item | Rust deduplicates only PR-review IDs. Non-PR items on different runners remain distinct. |
| Android and web, row identity | Slash-joined Android keys and pipe-joined web keys could collide if the separator appeared in IDs; web dispatch stripped an encoded prefix | Keys use an encoded `[runner_id,item_id]` pair, and web retains `rawItemId` explicitly for dispatch. Swift also encodes this pair, replacing its unit-separator key. Raw runner/item/action IDs do not change. |
| Android and web, grouping a directly supplied unsorted list | Partitioned the caller's order | Group keys follow the canonical Rust order. Normal native lists were already merged in that order. Missing concern remains Yours; others-only FYI still splits. |

For ordinary valid daemon lists, Android's visible urgency order, PR deduplication, Yours/Everyone Else membership and FYI counts already matched AgentKit and remain the same. The change establishes one implementation rather than changing those rules.

### Deliberately left alone

The observed disagreement about old-runner/read coverage remains: native blocked-only derivation and Android's unread fallback stay native; web still clears and labels `runner_too_old`. No new decisions, asks or actions are inferred. Connection state, polling, busy/refusal state and dispatch stay native. Action support and fallbacks, decision option identity, whose/name copy, group-header casing, age words and GitHub-link eligibility are unchanged. The other observed disagreements in task records, task/note words, rulings and refusal copy are separate slices and were not changed. Native board fallback/theme counts and workspace placement are also unchanged. In particular this work does not normalize the browser's main-workspace ownership policy or Android's placement of items whose workspace is no longer listed.

### Test changes and evidence

The shared `test/fixtures/needs-you-aggregate.json` is exercised through the C/native bridge, Swift, Kotlin and web adapters, and the wasm wrapper. It pins `z:ask, a:pr, a:fyi, a:future`, total 3, the FYI-only Others group, empty-runner totals and unchanged raw action IDs. Rust table tests additionally cover reversed arrival order, ties, unknown kinds, identical non-PR IDs across runners, PR rank/tie winners, `also`, derived ranks, runner-isolated buckets, absent workspace/repository buckets, payload retention and explicit invalid inputs. The existing daemon wire fixture is checked for payload preservation.

Existing test expectation changes are limited to:

- Web `fleet.test.ts`, “lists both runners’ items, each row naming its runner, in the order they were enrolled”: renamed to “…in shared urgency order” and now expects interleaving by urgency rather than enrollment order.
- Web `needs.test.ts`, “leaves out an item with no id or question, and reads a kind it doesn’t know as one that still counts,” “counts every item but a quiet note,” and “has no button for a Claude sign-in, which a browser can’t run”: their synthetic valid items now supply rank. Fleet’s “sends an answer to the runner the item is on, even when both number an item alike” and “marks only the item answered as busy, not the other runner’s item with the same id” also supply rank. Their existing action, count and copy expectations are unchanged. New tests separately assert that missing/invalid rank is refused.
- Android `NeedsYouItemsTest`, “two runners merge by rank not by clock,” “a review request on two runners shows once from the lowest rank,” and “equal ranks hold still by runner, then by id”: expected key strings now encode a pair rather than concatenate with `/`. The expected item order, PR winner and count are unchanged.
- No existing AgentKit test expectations changed. The adapter golden is added. Android adds the same golden and rank rejection tests; web adds the golden, dedup/count/group/action assertions and a separator-collision case.

Initial build validation (superseded by the fix-round evidence below), foreground only, no more than four jobs, no full gate or formatters:

- `cargo test -p farcooler-client -j 2 -- --test-threads=2`: all **370 library tests passed**, including five aggregation tests; one existing test is ignored. The full command reaches four existing `against_a_real_daemon` permission-hook failures (“no Permission reached the ring”). These exact four tests were already reproduced on main and recorded in `reports/ov-1284/README.md`; this ticket changes none of their daemon/hook/transport code. They were not weakened or edited.
- Rerunning the client crate with only `a_held_ask_reaches_needs_you_as_the_shared_fixture_spells_it` and `a_permission_` skipped: **455 passed**, one ignored, four filtered out, including all remaining integration suites and doc tests. Thus the unfiltered client command is still red on the pre-existing hook failures.
- AgentKit: `swift test --package-path apps/shared/AgentKit --jobs 2 --filter <all function names from NeedsYouTests.swift and NeedsYouWhoseTests.swift, plus NeedsYouRunnerCountTests>`: **27 passed**. Tests linked the built debug client archive via an untracked `target/release/libfarcooler_client.a` symlink. Mac and iOS use this same adapter.
- Android: Java 17, `./gradlew testInstrumentedUnitTest --no-daemon --max-workers=1 -Pfarcooler.nativeLibDir=<worktree>/target/debug --tests 'com.farcooler.model.NeedsYou*'`: **39 passed** across the five Needs You test classes.
- Web: `npm run test:needs`: **55 passed** in needs, fleet and scope. These three suites run in Node so their injected bridge can execute the actual Rust JSON presenter, without duplicating the policy in a stub. The command builds the Rust example first; the default web test command includes these suites after its Worker suites. `npx tsc -p tsconfig.client.json` passed.
- `cargo test --manifest-path crates/web/Cargo.toml -j 2 needs_you_tests`: the wasm wrapper parity test passed on the host. `cargo check --manifest-path crates/web/Cargo.toml --target wasm32-unknown-unknown -j 2` also passed (using the installed LLVM archiver for ring).
- Wire types `--check`, shared-core lint, its self-test, baseline-not-grown against the ticket's base, and `git diff --check`: passed. The baseline adds an empty `needs-aggregate` rule inventory. The new guard rejects the retired native merge/group reducers; unrelated baseline shrink opportunities were deliberately left untouched.

### ov-1337 fix round 1

The Android native boundary now returns no reading when the core is unavailable or Rust rejects the request. `Connection.readNeedsYou` validates decoded items through that boundary before publishing, retaining the last good reading and requesting a retry on failure. Negative and overflowing ranks therefore follow the existing read-failure path rather than reaching composition. Rendering helpers return an empty unavailable view and optional row flags when the native library is absent, so startup can reach the existing core diagnostics. No merge, count or grouping policy is duplicated in Kotlin.

Android test changes: “missing and invalid ranks are refused instead of inventing urgency” now expects a nullable rejected presenter result rather than an exception for invalid ranks. The missing-rank decode expectation is unchanged. New `NeedsYouPresenterTest` cases cover an unavailable core through the composition helpers, invalid ranks through the read boundary and composition, and a valid read through Rust. This corrects error handling; valid list ordering, grouping, counts, actions and copy are unchanged.

The browser capture harness now initializes the real wasm `needsYouView` bridge before constructing its Board. `needs-capture.mjs` takes `WEB_WASM_DIR` (the same built wasm/glue directory as the web build), bundles that glue and initializes it from embedded wasm bytes. Captures assert a visible badge and the existing scope-refusal notice and reject page errors. No screenshot-only policy stub is used.

The earlier four permission-hook failures were caused by the workflow environment's `FARCOOLER_NATIVE_HOOK_SOCKET`: the CLI intentionally prefers that socket over the test daemon's private hook socket. Run the required client suite with this workflow-only variable unset, after building the local daemon and CLI helpers. No tests are skipped or weakened for that environment correction.

Fix round 1 validation supersedes the initial red/skipped client evidence above. Foreground only, at most four jobs across concurrent commands; no full gate or formatters:

- Built the local helpers with `cargo build -p farcooler-daemon -p farcooler-cli -p farcooler-jni -p farcooler-client -j 2`.
- `env -u FARCOOLER_NATIVE_HOOK_SOCKET cargo test -p farcooler-client -j 2 -- --test-threads=2`: **459 passed, zero failures, one existing ignored test, zero filtered tests**. All four formerly failing hook tests passed. Only the workflow socket override was removed from the environment; no daemon, CLI, hook, or test code changed.
- AgentKit's same targeted Needs You filter with `swift test --package-path apps/shared/AgentKit --jobs 2`: **27 passed**, linked to the local debug Rust archive through the untracked release-path symlink.
- Android's same `testInstrumentedUnitTest` command with Java 17, `--no-daemon --max-workers=1`, the local debug native library and `--tests 'com.farcooler.model.NeedsYou*'`: **41 passed** across six classes. New `NeedsYouPresenterTest` covers an unavailable bridge through the composition-facing rows/groups/count helpers, and negative/overflowing ranks through the real Rust read boundary. The existing “missing and invalid ranks are refused instead of inventing urgency” test now expects `tryView` to return null for invalid ranks, replacing its crash expectation; missing-rank decoding still fails. This corrects error handling without changing valid-item aggregation or reference behavior.
- `npm run test:needs`: **55 passed**; `npx tsc -p tsconfig.client.json` passed.
- Built the real browser module with `cargo build --manifest-path crates/web/Cargo.toml --target wasm32-unknown-unknown -j 2` (LLVM archiver for ring), then generated web glue using matching wasm-bindgen **0.2.129**. `WEB_WASM_DIR=<generated directory> CHROME_PATH=<local Chrome> node test/browser/needs-capture.mjs <output directory>` passed all **four** desktop/phone, light/dark captures. Each title was `(6) Far Cooler`; badge and scope-refusal assertions passed with no page errors. The phone capture was visually checked.
- Shared-core lint, its self-test, baseline-not-grown against the ticket base, and `git diff --check`: passed. The existing aggregation guard and empty aggregation baseline remain applicable; no new baseline exemption was added.

### ov-1337 fix round 2

Read the ticket history again: its latest finding is the Android boundary, browser capture bridge and unfiltered client-test finding addressed in `d8804b8e4`; no additional reviewer finding is recorded. Retained that production fix and added `NeedsYouPresenterTest`'s “bridge failures reject a populated reading and leave composition safe.” Unlike the earlier empty-list unavailable-core case, it uses a populated list and tests an unavailable bridge, a thrown native linkage error, Rust's typed invalid-input response, malformed JSON and an incomplete view. Each case must reject the read and safely serve the composition-facing rows, groups, count and item flags. It uses the real Rust bridge to establish the valid rows before injecting failures. No existing test expectations or valid-item behavior changed.

The earlier explicit AgentKit filter omitted `aClaudeLoginLineIsAKindOfItsOwn`. This round includes it along with every other test from `NeedsYouTests.swift`, `NeedsYouWhoseTests.swift` and `NeedsYouRunnerCountTests.swift`; the test itself is unchanged.

Round 2 validation, foreground only, at most four jobs; no full gate or formatters:

- Rebuilt the local daemon, CLI, JNI library and client archive with `TMPDIR=/private/tmp cargo build -p farcooler-daemon -p farcooler-cli -p farcooler-jni -p farcooler-client -j 2`. The explicit temporary directory avoids the sandbox's restriction on the default macOS temporary directory. Swift tests link the debug archive through an untracked release-path symlink, as in round 1.
- Client: `env -u FARCOOLER_NATIVE_HOOK_SOCKET TMPDIR=/private/tmp cargo test -p farcooler-client -j 2 -- --test-threads=2`: **459 passed**, zero failures, one existing ignored test, zero filtered tests. All four permission-hook cases passed. The command ran with local daemon/socket access outside the sandbox.
- AgentKit: `swift test --package-path apps/shared/AgentKit --jobs 2 --filter <all test function names in NeedsYouTests.swift and NeedsYouWhoseTests.swift, plus NeedsYouRunnerCountTests>`: **28 passed**, including Claude sign-in.
- Android: Java 17 and the installed Android SDK, `./gradlew testInstrumentedUnitTest --no-daemon --max-workers=1 -Pfarcooler.nativeLibDir=<worktree>/target/debug --tests 'com.farcooler.model.NeedsYou*'`: **42 passed**, zero failures, errors or skips across six classes.
- Web: `TMPDIR=/private/tmp npm run test:needs`: **55 passed** through the real Rust presenter; `npx tsc -p tsconfig.client.json` passed.
- Browser: reran `needs-capture.mjs` using the real wasm/glue built in round 1 and local Chrome. All **four** desktop/phone, light/dark captures passed the badge, refusal-notice and page-error checks; every title was `(6) Far Cooler`.
- Shared-core lint, its self-test and baseline-not-grown against the ticket base passed. No baseline change or exemption was needed in this round.
- `git diff --check` passed.

### ov-1339 fix round 1

The four Needs You rejection messages are developer diagnostics, not displayed feature copy. Each now has a reviewed `platform copy` exemption on its logging line; the baseline records two Android exemptions and one each for AgentKit and web, without raising any feature-copy ratchet.

Validation: `python3 scripts/shared-core-lint.py`, `python3 scripts/shared-core-lint.py --self-test`, `python3 scripts/shared-core-lint.py --baseline-not-grown origin/main`, and `git diff --check` all passed. The reviewer also reported 56 passing targeted web needs/fleet/scope tests on the recovery commit. This round changes comments and lint accounting only; runtime behavior is unchanged.
