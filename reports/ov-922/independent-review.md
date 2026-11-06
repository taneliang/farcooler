# ov-922 independent code/data-safety review

Reviewed current dirty source read-only on 2026-11-06, including CONTRIBUTING.md,
public contract, definition/reducer/state, workflow transaction/outbox admission,
registry/compatibility and focused tests. No heavy tests or owner data actions performed.

## Findings

1. **P1 — Cross-workspace completion can deadlock on eager dependent activation.**
   `crates/workflow/src/reducer.rs:143`–150 auto-enters newly dependency-ready work
   using only its own aggregate resources. `crates/store/src/workflow.rs:75`–84
   then rejects the entire result transaction if a newly activated dependent wants
   a resource held by another workspace. Repro: workspace A has active item on rA
   and blocked dependent on rB; workspace B has active item on rB and blocked
   dependent on rA. Both dependencies are local and legal to enqueue. Each original
   item's completion becomes Resource refusal because the dependent activates,
   retaining the original reservation and admitted outbox operation. Neither
   completion can release its resource. Stable refusals also force new command IDs
   for repeated attempts. Defer conflicted dependent activation independently of
   settling the accepted result, or supply/persist resource availability in the
   deterministic admission input; retain replay identity. Add a two-workspace
   opposite-resource temporary SQLite/reopen regression.

2. **P2 — Validated sequences/rework can lack a candidate producer.**
   `crates/workflow/src/definition.rs:62`–73 validates only target identity/order;
   a coding definition with rework_target stage-2 (Check) passes. After Review
   rejection, `crates/workflow/src/reducer.rs:231` clears candidate, then Check
   evidence always fails matching because the candidate is None. Similarly a
   first Check/Evaluate stage is accepted by validate but cannot finish because
   enqueue disallows initial_candidate except for first Tracker. Reject missing
   producer paths (including the bounded rework path) or explicitly model a
   candidate-preserving rework policy; test the chosen contract. This is structural
   evidence validity, not a requirement to mandate review/Git stage order.

## Verified properties and limits

- New tables are appended at actual registry position and marked Older::Refused.
  Legacy module mutations/callers are unchanged; no external dispatcher is enabled.
- Immediate SQLite transaction covers reducer event/state/resource/outbox/receipt.
  Refusals consume no aggregate revision; exact duplicate receipt payload is stable.
- Claims/admission are database metadata only. Admission is CAS and admitted expiry
  cannot reclaim execution. Persistent resource generations retain tombstones.
- Cancellation retains reservation through the stop receipt; original admitted
  operations become uncertain and only correlated stopped reconciliation settles
  them. Runtime authentication, adapter fencing and receipt authenticity are
  explicit obligations and remain integration gates.
- The single-workspace blocked-resource fix is sound for two local waiters: the
  activation sweep rechecks ownership after each earlier waiter. It does not resolve
  finding 1 across workspace aggregates.

Initial verdict: address findings before claiming the focused core ready for review.


## Follow-up review — both findings resolved

Re-read the corrected implementation and regression source on 2026-11-06.
No implementation changes or test commands were run by this reviewer.

- Finding 1 resolved: `crates/store/src/workflow.rs:58`–69 derives relevant external
  occupancy under the same Immediate transaction, overwriting caller claims.
  `crates/workflow/src/reducer.rs:169`–172 excludes busy resources from dependent
  activation, so completion can settle and release its own resource while a
  conflicting dependent remains Blocked. The sweep rechecks local resources after
  each admitted waiter. `reducer.rs:20` authenticates Advance as Controller, giving
  the runtime an explicit re-evaluation command after another workspace releases a
  resource. `workflow.rs:103`–107 stores the effective input and effective hash for
  deterministic replay, while `workflow.rs:157`–160 preserves the original request
  and hash for receipt identity. The separately bounded snapshot is excluded from
  the caller request-size calculation (`reducer.rs:262`–266); unrelated external
  resource rows cannot inflate the replay snapshot beyond relevant item resources.
  The new `workflow_cross_workspace_dependents_cannot_block_completion_and_replay`
  test constructs the opposite-resource cycle, accepts both completions, reopens,
  advances the remaining waiter and verifies replay and forged-snapshot exclusion.
- Finding 2 resolved: `crates/workflow/src/definition.rs:43`–59 tracks candidate
  availability on the forward path, with exact Tracker input seeded only at the
  first stage. `definition.rs:76`–83 requires a Build/Action producer as the stable
  rework target and requires it to precede every evaluator. Thus clearing candidate
  at rework enters a producer before any evidence-consuming stage. The new
  `workflow_definition_rejects_missing_candidate_paths` test covers initial
  Check/Evaluate/Review and a Check rework target; existing generic Action/Evaluate
  and bounded rejection tests exercise an executable noncoding path.

Final source-review verdict: no remaining blocking correctness or data-safety
findings in this focused implementation. This is a source review, not execution
certification: use the builder's final focused/regression/mutation logs for test
outcomes. Runtime authenticity, adapter fencing, owner-data conservation/cutover,
full integration and main CI remain parent release gates as the public contract
states.

Reviewed implementation SHA256:

- `crates/workflow/src/definition.rs`: `eed8857208dce96ff62599eb59b64158323b68308106338b5a063203e87abaeb`
- `crates/workflow/src/reducer.rs`: `5ee233873dfdac91b57dc7b897bb99d1488a89cd6f9ecd2800f4d7ea6174b7cb`
- `crates/workflow/src/state.rs`: `1ece768d7ec08d14ecfdccb802f31388d233a41d41e8c416078b1a1030699e76`
- `crates/store/src/workflow.rs`: `75ea7ccd998ee187a5f948d6149614069a7cef2660fb44cdbce45949c9ab7d0a`
- `crates/store/src/workflow_effects.rs`: `430f052b0945b2e40662af5e1da5d470c3a7aa849b1dfe8209b7cee8ed418b5a`
- `crates/store/src/migrate.rs`: `d5d4e7e73fff21de397f2d779b2f97565240d81260a73a262145c39b036c31a9`
- `crates/store/src/compat.rs`: `756e54ca87c3345ea56fc63e302726f66a7daf8230ee856342cafb9258bf1940`


## Final frozen-source refresh

Re-read the final restored source after mutation tests. The occupancy query now
filters relevant resources directly in SQL (`workflow.rs:65`–69), using bound JSON
with `json_each(?2)` and a separate workspace parameter. This retains the reviewed
transaction/replay behavior while avoiding allocation of all unrelated external
reservations. The compatibility wording correction changes no admission behavior.
The implementation hashes above have been refreshed against these final files.
No new blocking findings.

Read the builder's final raw logs: store-regressions.log reports 486 passed,
0 failed, 1 ignored, exit 0; final-focused-tests.log reports 16 pure tests passed,
0 failed and wrapper exit 0; additional-checks.log records six expected mutation
failures (exit 101), six restored successes, store regressions exit 0 and clippy
exit 0. These were observed in saved evidence; this reviewer did not run tests.
