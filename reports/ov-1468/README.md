# ov-1468 · Draft the live workflow with two pre-review checks

From the Main workspace orchestrator, file the saved bundle with this command.

```sh
farcooler workspace workflow-draft 01a0e3c7-3e2c-7ed0-b31c-0cc0d7401571 \
  --file /Users/e-liang/Dev/overnight/.claude/agent/reports/ov-1460/workflow-draft-live-plus-checks.json \
  --reason 'Run quick checks and Clippy before review.'
```

When the file changes the selected adapter, the daemon replaces pins matching
that adapter's previous pin, and accepts an empty string as a placeholder in
`operation.arguments[0]`. It computes the new pin from the submitted adapter
and validates every operation before filing the draft. A foreign pin still
fails validation.

## Scratch daemon

The scratch fixture reconstructed the baseline by taking out the two added
stages and their two check commands. It verified that the baseline adapter
hashed to the pin in the saved file, then seeded the scratch selection directly.
The CLI read the original saved file.

The following command ran with `FARCOOLER_HOME` set to the private fixture home.

```sh
"/Applications/Far Cooler Canary.app/Contents/Resources/farcooler" workspace workflow-draft 01a21380-493c-7931-8812-3def1f39467f \
  --file /Users/e-liang/Dev/overnight/.claude/agent/reports/ov-1460/workflow-draft-live-plus-checks.json \
  --reason 'Run quick checks and Clippy before review.'
```

The command exited successfully and returned pending draft `d9c7ad1ec52d`.
Its field changes are all additions under `/definition/stages/2` (quick checks),
`/definition/stages/3` (Clippy), `/adapter/config/checks/1`, and
`/adapter/config/checks/2`. See [the full CLI output](scratch-draft.txt).

All 13 operations in the stored draft carried the same recomputed adapter pin,
`wf1:1dc6642a826826382638060d5b1634938c7304d5823077e74a28d9d04ebee859`.
The scratch selection stayed at
`wf1:9583d51a8cc63361dfb04cd2c7934fd19720b273a031fd105c54d10bd28f747c`,
with zero publication receipts.

## Regression tests

Before the fix, the live-plus-checks socket test failed with
`Draft definition stage "provision" field operation.arguments[0]: must match the adapter pin.`
After the fix it files a pending draft with either the old pins or empty
placeholders and checks the full stored definition and the field changes on the
card. The second regression verifies that a foreign pin in a landing-group step
still refuses the draft.

The three daemon socket suites passed 33 tests with one ignored helper. All
29 focused draft, summary, and decider unit tests passed. The CLI workflow and
template unit tests passed 32 tests with one ignore. The CLI presentation
integration test and all 31 client workflow presentation tests passed. The
broader daemon workflow run passed 591 tests
with 9 ignores and 11 initial failures. All 11 passed on retry, with lower
concurrency, the system Python first on PATH where needed, and the matching
hook CLI built for the native-hook fixtures.

Workspace Clippy with warnings denied, `daemon-layers.py`, `file-size-budget.py`,
and `shared-core-lint.py` passed.
