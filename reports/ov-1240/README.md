# ov-1240: gate and CI train, findings fixed

Branch `ov-1240-ci-train-2`: fast-forwarded from `ov-1239-ci-train`, then the six
non-merge commits of `ov-1237-quarantine-inventory` cherry-picked with `-x`.
No merge commits.

## Conflict resolution

- `.github/workflows/ci.yml`: kept both sides. ov-1239 moved the Linux Rust run
  into the `rust-tests` archive shards, so the quarantine runner
  (`python3 scripts/quarantine.py run --`) now wraps that shard command, and
  `quarantine.py --lint` sits beside `ci-concurrency-test.py`. The first
  cherry-pick left conflict markers; `bc9bdf4a8` removes them.
- `scripts/lane-gate.py`: ov-1237's changes applied cleanly. The shard-command
  matcher now accepts the quarantine wrapper, and `landing_policy.py` accepts
  the same wrapped command when it narrows the local run.

## Findings

1. `scripts/nightly-dispatch-test.py` now expects the merged dependency lists
   (`ios-ui` needs `[ios, yield-ui]`, `captures` needs
   `[ios, ios-ui, yield-captures]`, `mac-perf` needs `[captures, yield-perf]`).
2. actionlint: `concurrency.queue: max` is valid GitHub syntax, so the
   workflows keep it. No release knows it: the latest, 1.7.12, reports only the
   `queue` diagnostics (upstream issues #654, #657 open; #746 closed not-planned),
   and `.github/actionlint.yaml` cannot whitelist a key. The fix is now committed
   and reproducible:
   - `scripts/actionlint/queue.patch` against upstream
     `011a6d15e749bb3f2d771eed9c7aa0e7e3e10ee7`, built and run by
     `scripts/actionlint.sh` (needs git and Go).
   - The patch accepts `queue` only as the literal `single` or `max`. Any other
     value or a misspelled key still fails; `scripts/actionlint.sh --self-test`
     proves it (`queue: max`/`single` pass, `queue: all` and `queu: max` fail).
   - Command run: `scripts/actionlint.sh`, i.e.
     `actionlint -config-file .github/actionlint.yaml .github/workflows/*.yml`
     with the patched build `v1.7.13-0.20260419144658-011a6d15e749+dirty`.
     Exit 0 on every workflow, nothing ignored. Release 1.7.12 with the same
     command still exits 1 on `queue`.

## Scripts that read ci.yml or nightly.yml, run against the merged files

Fixed because the merge broke them: `landing-gate-test.py` (aggregate `android`
job, new `tunnel-helper`/`rust-tests`/`android-*` jobs, `process_guard`,
`landing_policy`, `gate_evidence` in its scratch repo), `ios-ui-shards.py`
(`rust-macos` joins the macOS reservation; self-test plants retargeted).

All passed, run in the foreground from the repo root:

- `scripts/ci-changes.py --self-test`, `ci-concurrency-test.py`,
  `ci-job-time-check.py --self-test`, `ci-yield-to-main.py --self-test`,
  `ci-macos-capacity.py --self-test`, `ci-flake-tally.py` (via
  `ci-flake-tally-test.sh`)
- `lane-gate.py --self-test` (with `HEAVY_LOCK_HELD=1`) and
  `--dry-run --base origin/main`
- `gate_evidence.py` (`gate-evidence-test.py`, `gate-evidence-mutation-test.py`),
  `engine-gate-test.py`, `landing_policy.py` (`landing-policy-test.py`),
  `landing-gate-test.py`, `landing-gate` (via that test)
- `ios-ui-plan.py`, `ios-ui-shards.py` (`--check`, `--self-test`),
  `ios-ui-shard-budget.py`, `macos-rust-tests.py --self-test --check`,
  `rust-shards.py`, `rust-shadow-test.py`, `test-leak-check.py`,
  `nightly-report.py`, `nightly-dispatch-test.py`, `canary-plan.py`,
  `proto-lint.py`, `swift-build-cache.py` (read by `workflow-review-timing-test.py`
  and the Swift cache check; no own self-test), `appcast.py` (`appcast-test.sh`)
- `tunnel-e2e.sh` mentions ci.yml only in comments; nothing to run.

ov-1237: `python3 scripts/quarantine-test.py` and `quarantine.py --lint` pass.
Not run: hosted CI.
