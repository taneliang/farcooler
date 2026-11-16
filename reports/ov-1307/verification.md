# ov-1307 verification

Reviewed ov-1305's original intent and acceptance and the carried receipt analysis and fixtures in `reports/ov-1305/report.md`. The original report records the full daemon and workflow nextest run (4,122 passed, 19 skipped).

On this branch, both `lane-gate.py --self-test` runs passed with default and empty Cargo homes, leaving git status clean. All 15 `scripts/*-lint.py` programs, `file-size-budget.py`, `daemon-layers.py`, `gate-evidence-test.py` (18 tests), and `engine-gate-test.py` (9 tests) passed. The self-tests and fixture tests required process inspection outside the sandbox; sandboxed runs were disrupted by denied `ps` access and Git `confstr` warnings in captured output. The system Python was used because the default Python executable was for a different architecture.

A fresh full nextest run on this branch completed 4,122 tests: 4,121 passed and `workflow_coding_native_real_pty_normalized_hooks_and_quiescence` failed; 19 were skipped. The new fold-base regression passed, and the leak check found nothing left behind. The failing native PTY test passed on an isolated rerun. The fresh full run therefore did not independently reproduce the earlier all-green suite; the isolated pass does not establish its cause.
