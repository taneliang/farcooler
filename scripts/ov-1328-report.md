# ov-1328 — selected-family disk budgets

Started at origin/main `10618439e`, then cherry-picked ov-1327's `a466b6a78`
and `3a61e01cc`. The existing ov-1326 full-family map entry for `gate_disk.py`
is retained. This report supersedes ov-1327's 70 GB admission requirement.

Admission uses named constants: 36 GiB when `rust` is selected and 8 GiB
otherwise, plus the unchanged 25 GiB daemon reserve. Thus the thresholds are
61 and 33 GiB respectively; messages label GiB (2^30 bytes) as GB and name
free space, needed space, the selected budget, and the daemon reserve. Both
preflight and the check after acquiring the queue use the selected jobs.

The Nov 17 full-gate observations were 57 -> 21 GB and 62 -> 27 GB free,
about 35 GB used. The doctest tail is unmeasured. These are admission budgets,
not disk reservations; no full gate was run for this ticket.

The real-wrapper regression tests both Rust at 60 GB and web at 32 GB with
60-second timeouts and deletion audit hooks. Unit tests cover one byte below,
exactly at, and one byte above each threshold, including a mixed Rust/web plan.
The inherited archive tests prove successful extraction deletes only the
run's `target/rust-tests.tar.zst`, preserves an unrelated adjacent file, keeps
the archive on extraction/metadata failure, and refuses other names/symlinks.
No stale output or log cleanup was added or performed.

Validation passed, 41/41 foreground commands:

- `lane-gate.py --self-test`, including the policy, wrapper, process and benchmark suites.
- `landing-gate-test.py`: 14 tests, including both timed deletion-free refusals.
- `landing-policy-test.py`: 24 tests.
- All 15 `scripts/*-lint.py` entry points: self-tests and default scans.
- `actionlint.sh`: self-test and default scan.
- Additional stable and Canary baseline proto scans, test-leak and quarantine lints.
- `bash -n scripts/landing-gate` and `git diff --check`.

Used native Python 3.14 via a temporary PATH entry because the default Python
binary is incompatible with this CPU. Fixture processes dropped inherited
`FARCOOLER_*` context and capped Cargo jobs/nextest threads at four. The first
restricted wrapper-suite run hit the sandbox's `ps` denial; all final checks
ran with process inspection/local fixture sockets available and passed.
Temporary validation logs are outside the checkout and uncommitted. No
`cargo fmt`, stash, reset, clean, or platform/full-gate build was run.
