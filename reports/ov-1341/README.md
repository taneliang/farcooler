# ov-1341: lint before review

`workflow-main-draft.json` is a proposed replacement for the Main bundle in
`.claude/agent/reports/ov-1071/workflow-main.json`. It adds check 1 and a
`lint` Check immediately before the existing agent `gate-0`, routing rejection
back to build. The existing folded landing gate remains. The adapter pin has
been recomputed for the changed checks. This is a draft based on that checked-in
bundle: compare it with the currently published bundle before proposing it, and
rebase the change if that bundle has moved. The owner must approve publication
of the changed command. Publish only after this ticket's code and script are on
the runner's landing base and the runner has been updated.

The gate discovers every `scripts/*-lint.py`, runs the CI channel variants for
proto, and reports each failed lint's output. It runs scans, not the lints'
self-tests or the separate CI baseline-growth check. The builder brief is bounded
to 4096 bytes, so exceptionally long output may be truncated; the full process
log remains in the gate receipt.

Measured in this worktree with `/usr/bin/time -p scripts/lint-gate.sh`:
32.90 seconds elapsed (all scans passed). A synthetic failing lint also returned
nonzero, printed its diagnostic, and did not prevent a later lint from running.
