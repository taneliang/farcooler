# ov-1261: carry four reviewed branches to main

## Result

Carried unchanged. Gate result: **NOT all green** (one real failure, reported below, not fixed).

- Branch `ov-1261-carry-four`, based on origin/main `adfde91c0`; 137 cherry-picks (`-x`), no conflicts, no empty picks.
- Order and counts: ov-1257-mac-tests-not-load (1), ov-1254-engine-train-main (130), ov-1253-pipeline-design-3 (5), ov-1256-ci-shards-early (1).
- `git rev-list --merges --count origin/main..HEAD` prints 0.
- Tree diff against a plain in-order merge of the four branches onto origin/main: empty.
- Conflicts resolved: none.

## Checks (PATH=/opt/homebrew/bin:$HOME/.cargo/bin:/usr/bin:/bin:/usr/sbin:/sbin, foreground)

- `cargo clippy --workspace --all-targets -- -D warnings`: pass.
- `./scripts/file-size-budget.py`: pass (2138 files, 27 grandfathered).
- `CI=true ./scripts/lane-gate.py --base origin/main --only wire,doc-comments < /dev/null`: **FAIL**, two wire steps:
  - `scripts/macos-rust-tests.py --self-test --check`: environment only. With that PATH `python3` is `/usr/bin/python3` 3.9.6 and the script needs 3.11+. Run with `python3.14` it passes. Fix the PATH (python3 on it), not the code.
  - `scripts/landing-gate-test.py` (`test_gate_covers_ci_job_conditions_including_unconditional_jobs`): **real**. ov-1256 changed `.github/workflows/ci.yml` so the `relay` job (and others) have `needs: [changes, rust-tests, android]`. main's test (from ov-1240) asserts `"    needs: changes"` in each conditional job block at scripts/landing-gate-test.py:93. ov-1256 did not update that test, so the combination fails. Not changed here, per the ticket.

## Needed from the owner

Decide the fix for the landing-gate test (relax the assertion to accept `needs: [changes, ...]`, or amend ov-1256). Both are code changes outside "carry only". Doc-comments and the other wire steps pass.
