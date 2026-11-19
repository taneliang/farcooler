# Main's checks before review

`.claude/agent/reports/ov-1071/workflow-main.json` configures
`scripts/daemon-layers.py` and `scripts/file-size-budget.py` after build and
before agent review (ov-1368). `workflow-main-draft.json` also runs the lint
gate (ov-1341) after those checks. Each Check routes a nonzero exit back to
build with the command's diagnostic.

The checked-in bundles have new adapter pins. Compare the bundle to the
currently published definition before asking the owner to approve publication.
Publish after the updated runner and scripts are available.

The lint gate discovers every `scripts/*-lint.py`, runs the CI channel variants for
proto, and reports each failed lint's output. It runs scans, not the lints'
self-tests or the separate CI baseline-growth check. The builder brief is bounded
to 4096 bytes, so exceptionally long output may be truncated; the full process
log remains in the gate receipt.

The two ov-1368 checks have 12-second timeouts and one attempt each. Five
sequential runs of both commands in a full worktree, using their configured
environment and the runner's PATH handling, took 4.41–9.07 seconds per pair.

The lint check pins HOME and PATH for the runner, matching the landing check.
Measured with `/usr/bin/time -p /usr/bin/env -i HOME=/Users/e-liang
PATH=/opt/homebrew/bin:/Users/e-liang/.cargo/bin:/usr/bin:/bin:/usr/sbin:/sbin
/bin/sh scripts/lint-gate.sh`: 10.80 seconds elapsed, all scans passed. Under
the same cleared environment, a synthetic failing lint returned nonzero and
printed its diagnostic. The workflow reducer test verifies that a rejected check
passes its brief to the next build.
