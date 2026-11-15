# ov-1261: carry four reviewed branches to main

## Result

Carried unchanged, plus one test-only fix commit (ov-1256 follow-up). Gate result: **all green**.

- Branch `ov-1261-carry-four`, based on origin/main `adfde91c0`; 137 cherry-picks (`-x`), no conflicts, no empty picks.
- Order and counts: ov-1257-mac-tests-not-load (1), ov-1254-engine-train-main (130), ov-1253-pipeline-design-3 (5), ov-1256-ci-shards-early (1).
- `git rev-list --merges --count origin/main..HEAD` prints 0.
- Tree diff against a plain in-order merge of the four branches onto origin/main: empty.
- Conflicts resolved: none.

## Change beyond the carry (fix round 1)

- `scripts/landing-gate-test.py`: `test_gate_covers_ci_job_conditions_including_unconditional_jobs` now accepts `needs:` as a scalar or a list, and passes when `changes` is among the entries. ov-1256 made several jobs `needs: [changes, rust-tests, android]`, which the old literal `needs: changes` check rejected. `ci.yml` and every carried commit are untouched.

## Checks (foreground; PATH with python3 = 3.14 first, then /opt/homebrew/bin:$HOME/.cargo/bin:/usr/bin:/bin:/usr/sbin:/sbin)

- `cargo clippy --workspace --all-targets -- -D warnings`: pass (first round; no Rust changed since).
- `./scripts/file-size-budget.py`: pass (first round; no source size changed).
- `python3.14 scripts/landing-gate-test.py`: 6 tests OK.
- `HEAVY_LOCK_HELD=1 ./scripts/lane-gate.py --self-test`: ok.
- `./scripts/lane-gate.py --dry-run --base origin/main`: runs; Linux-only web steps skipped as designed.
- `CI=true ./scripts/lane-gate.py --base origin/main --only wire,doc-comments < /dev/null`: pass (wire and doc-comments all pass). The stock PATH's `python3` is 3.9.6, but `macos-rust-tests.py` needs 3.11+, so python3 was pointed at python3.14.
