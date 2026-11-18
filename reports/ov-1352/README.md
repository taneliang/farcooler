# ov-1352 returned landing gate

The send-back for candidate `e6c9edb61f34891bb924948632376a7dd310b058`
came from landing group `group-00000000000000004320-landing`, folded as
`ff8839e8251245011758c84ff5649dc8ed331d67`.

The installed Canary engine retained these receipts in `workflow/receipts/`:

- `wf1:440f9e28bdbe6e495fea6dfd59348d2e887e74f459fbe381b6e8c1810b1daf42`:
  the first attempt ended as `SafeFailure`. Its output stops during startup;
  there is no completed check result or assertion failure in that log.
- `wf1:c44a69b1f476f2b00ac1a532c200b2eb69b38f96894995ad862351e1c4b593d9`:
  the retry was `GateRejected` at disk admission, with 60.61 GB free and
  61.00 GB required (36 GB gate budget plus 25 GB daemon reserve). No checks ran.
  Its binding explicitly names ov-1352 and the candidate above.

The refusal is an environment blocker, so no production-code repair is justified
by this send-back. The existing recovery implementation and regressions remain
in the preceding ov-1352 commits. Admission thresholds are unchanged.

On November 19, a read-only call to `gate_disk.admit(Path.cwd(), ['rust'])`
passed with 238.04 GB free against the same 61.00 GB requirement. This verifies
only current disk admission, not the full landing gate. No installed-engine
state or other runs' trees were changed, and `scripts/landing-gate` was not run.

Fresh targeted verification:

- `TMPDIR=/tmp cargo test --locked -p farcooler-daemon --lib
  workflow_runtime::fold_trees_tests -- --test-threads=4`: 15 passed, including
  interruption before, during and after deletion, lost `.git` links, partial
  filesystem errors, safety blockers and unknown-byte accounting.
- The same command with `workflow_runtime::build_cleanup_tests`: 36 passed,
  including scheduled retries, ownership fences, output policies and preserved
  commits.
- The same command with `workflow_runtime::coding_verbs_tests`: 46 passed and
  one failed at `coding_verbs_tests.rs:655`, where the named-tree restart test's
  immediate `Owner::reap` returned `process start identity unavailable` after
  aborting its gate. That exact test passed alone on a fresh rerun (4.18 s).
  Its file-preservation assertions passed on the rerun. This separate process
  identity flake does not implicate interrupted group-tree deletion.
- `PATH=/usr/bin:/bin:/opt/homebrew/bin:$PATH scripts/lint-gate.sh`: passed.
- `python3 scripts/file-size-budget.py`: passed.

The initial sandboxed test run compiled successfully but all 15 fold-tree tests
stopped in Git fixture setup (`coding_tests.rs:7`, `git invocation failed`).
The passing runs used native execution so the local Git and lsof fixtures could
run. No full-workspace build was performed.
