# ov-1239 integration status

The branch is linear and contains the queued line followed by the six waiting branches' non-merge commits. Integration is **incomplete**: the checks below failed, and this train must not be landed as-is.

| Waiting ticket | Conflicts and resolution |
| --- | --- |
| ov-1218 | `scripts/landing-gate`: retained fail-fast, landing, and warm-target flags. |
| ov-1224 | No textual conflict. Its fixture now expects `process_guard.py`, which is absent from this train; the dependency must be integrated. |
| ov-1189 | `ci.yml`: kept both Mac coverage and landing gate tests; `ci-changes.py`: combined report patterns and boundary cases; `lane-gate.py`: kept landing options and engine base. |
| ov-1182 | `ci.yml`, `nightly.yml`, `macos-rust-tests.py`, `ios-ui-shards.py`, `lane-gate.py`: combined Rust archive/shard changes with prior Mac, Android, nightly and local gate coverage. The local gate's Mac assertion now fails and needs a coverage-preserving integration fix. |
| ov-1226 | `ios-ui-shards.py`, `lane-gate.py`: combined runner and gate checks; `nightly.yml`: retained Rust and yield jobs; `ci-concurrency-test.py`: retained main policy checks and added capacity/yield tests. The capacity checker cannot yet model the targeted nightly matrix. |
| ov-1156 | `nightly.yml`: combined targeted conditions with existing yield and reservation settings. The Mac performance job has duplicate `needs` and `if` keys requiring correction. |

Foreground verification: lane-gate dry-run and ci-job-time-check self-test passed. Lane-gate self-test failed on the Mac test assertion; gate-evidence and engine-gate tests failed because `scripts/process_guard.py` is missing; ci-concurrency failed on the targeted matrix; ci-changes self-test failed on expected `non_docs` values. The installed actionlint reported duplicate nightly keys and does not recognize `concurrency.queue`. The no-weakening and free-runner requirements have not been verified.

## Fix round 1 (incomplete)

Combined the Mac perf dependencies and dispatch condition, restored the ov-1138
process supervisor and wrapper wiring, updated evidence fixtures for the tunnel
helper, reconciled the local Rust assertion with the archive consumers, and
fixed the report-only `non_docs` expectations. The capacity reader now accepts
the targeted Nightly exclusion shape conservatively and counts the `part`
matrix; it rejects unrecognized exclusions. No waiting ticket was intentionally
omitted in this round.

The capacity audit found additional unresolved integration conflicts: the
current landing antichain reaches six Macs and 21 jobs before background work;
`ov1186-cache.yml/reuse`, Nightly `rust-macos`, and `rust-shadow` escape the
shared reservation, and the Nightly Mac Rust job has no yield dependency.
These must be reconciled without skipping coverage or exceeding the free runner
budget. The no-weakening and free-runner audit remains incomplete.

Foreground checks: lane-gate dry-run, ci-changes self-test and ci-job-time-check
self-test passed. ci-concurrency fails on the capacity issues above. Installed
actionlint 1.7.12 rejects `concurrency.queue`; duplicate perf keys are fixed,
but compatible actionlint validation remains outstanding. The process guard's
seven real-process tests pass with process inspection permitted; the remaining
lane-gate and evidence results must be checked after this fix.

The evidence and engine gate suites passed (16 and 9 tests) with the tunnel
fixture fix, before the final supervisor wiring change; rerun them on the final
candidate. Lane-gate self-test progressed past all seven process guard tests but
failed in landing-policy tests: stale Rust-shadow and fail-fast expectations,
a fixture missing `process_guard.py`, and a PATH without cargo. These remain to
be fixed and retested. This branch remains unsuitable for landing.

## Fix round 2

The prior status describes the intermediate candidate, not the final results below.
All six waiting tickets remain in the linear branch. In addition to the conflict
resolutions in the table above, the local landing policy now selects affected Rust
packages from the archive-derived local workspace command; its real Cargo failure
proof exercises both affected and unaffected packages. The fixture copies the
process supervisor and expects the wrapper's fail-fast argument.

The research workflow and both Nightly Rust jobs use the shared background queue.
Nightly's Mac Rust run yields to main CI, and the iOS yield follows it even when
Rust fails or is skipped. Landing's Mac archive waits for Swift tests, its two
native shards run serially, and Linux Rust shards wait for Android and run two at
a time. Android matrices run one at a time. These dependencies use explicit
`!cancelled()` conditions so failed or skipped unrelated predecessors do not
suppress checks. This trades latency for the reservation; it does not remove
shards or test commands. The capacity model's mutation tests cover the new
locks and dependencies. Its conservative antichain bound is 4 landing Macs + 1
background Mac, and 14 landing jobs + 4 background jobs + 1 recorder + 1 yield
job = 20. Queueing preserves pending runs up to GitHub's queue limit.

Foreground validation on this candidate, with Homebrew Python and process access:

- `HEAVY_LOCK_HELD=1 ./scripts/lane-gate.py --self-test`: passed, including seven supervisor and thirteen landing-policy tests.
- `./scripts/lane-gate.py --dry-run --base origin/main`: passed.
- `python3 scripts/gate-evidence-test.py` and `python3 scripts/engine-gate-test.py`: 16 and 9 passed.
- `python3 scripts/ci-concurrency-test.py`, `./scripts/ci-changes.py --self-test`, and `./scripts/ci-job-time-check.py --self-test`: passed.
- Actionlint upstream main `011a6d15e749` with a local parser patch accepting only scalar `queue: max` or `queue: single`: passed. Unpatched release 1.7.12 and upstream main reject the queue key; no diagnostic was ignored and workflow files were not transformed. This temporary validator patch is not an upstream release.

The runner inventory in `.github/` uses GitHub-hosted Ubuntu, macOS and the
GitHub-hosted Xcode 27 public preview labels; the capacity reader expands matrix
and reusable-workflow jobs. The conflict and local plan checks retain the full
workspace Rust run, filtered native shards, Swift, iOS, Android, and other
existing checks. This is a source and self-test audit, not a hosted-run proof;
the altered schedule and free-program eligibility of preview labels still need
hosted verification. No build beyond those invoked by self-tests was run.
