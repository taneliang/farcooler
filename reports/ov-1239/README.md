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
