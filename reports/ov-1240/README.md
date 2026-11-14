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
   workflows stay as they are. actionlint 1.7.12 (Homebrew, the latest release)
   rejects it, and `.github/actionlint.yaml` cannot whitelist a key. I ran
   upstream actionlint main `011a6d15e749` with a local parser patch that
   accepts only scalar `queue: max` or `queue: single` (the ov-1239 build), with
   the repo config: `actionlint` exits 0 on all of `.github/workflows/*.yml`,
   nothing ignored. That is a patched build, not a release; the unpatched
   1.7.12 reports only the 50 `queue` diagnostics and nothing else.

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
