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

## Main draft

The Overnight Main workspace on Canary is
`01a0e3c7-3e2c-7ed0-b31c-0cc0d7401571`. Canary identified the workspace and
refused this session's workflow reads with `scope denied: known caller required`,
including the existing configuration-only read. Reading the live bundle and
filing its draft requires an authenticated Main orchestrator or owner session.
