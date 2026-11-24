# ov-1456 · Draft a changed workflow

`farcooler workflow bundle <workspace-uuid> --json` exports the selected
`definition` and `adapter` as a file accepted by
`farcooler workspace workflow-draft <workspace> --file <bundle.json>`.
The runner reads the selected configuration directly, so runs, logs, publication
history, and runner suggestions do not count toward the response size. An
owner can export the adapter values. Other callers receive the existing
environment masking, and the draft command rejects a file with masked values.

Draft refusals identify the offending field and rule. A stale adapter pin,
for example, identifies the stage and `operation.arguments[0]`. Refusals
leave the draft state and the published selection untouched.

## Verification

Both new socket regressions ran before the production changes.

| Regression | Before | After |
| --- | --- | --- |
| `ov1456_bundle_exports_live_settings_and_can_draft_a_change` | The export request returned `response_too_large` with over 1 MiB of run history. | The bundle fit under 60 KiB, matched the published fixture, and served as the base for a pending quick-checks draft. The published pin stayed selected. |
| `ov1456_draft_refusal_names_the_stage_and_operation_pin` | The stale pin returned `invalid argument: definition`. | The refusal named the stage, `operation.arguments[0]`, and the adapter pin rule. |

The workflow and protocol suites passed 339 tests. The three daemon socket
suites passed 31 tests, with one ignored helper. On the final code, 106
targeted daemon unit tests and 26 CLI tests passed, and both socket
regressions passed again. The targeted suites retained four existing
ignores. The daemon layer and file size checks also passed.

## Landing repair

The landing gate's Rust run reported 7,451 passed, one failed, and 40 skipped.
Both ov-1456 socket regressions passed. The sole failure,
`a_draft_naming_a_stranger_is_refused_and_one_naming_a_member_is_filed`, also failed
on its isolated retry. The test still expected `InvalidArgument { what: "decider" }`.
It now verifies that each rejected principal gets a refusal identifying the
`build` stage, the `progression.AwaitApproval.principal` field, and the
requirement that the named decider can answer the gate. The valid member's
draft is still accepted.

After the repair, all 14 decider tests passed under the nextest `gate` profile
at reduced priority with four test threads. The daemon layer and file size
checks passed again. The full landing gate was not rerun for this test-only
repair.

## Main draft

The Overnight Main workspace on Canary is
`01a0e3c7-3e2c-7ed0-b31c-0cc0d7401571`. Canary identified the workspace and
refused this session's workflow reads with `scope denied: known caller required`,
including the existing configuration-only read. The read was retried during the
landing repair and returned the same refusal. The live draft remains unfiled.
Reading the live bundle and filing its draft requires an authenticated Main
orchestrator or owner session.
