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
