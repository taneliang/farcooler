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
