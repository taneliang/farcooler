# ov-1160: Mac Rust coverage selection

Per-push CI keeps the existing `Rust (ubuntu-24.04)` and
`Rust (macos-latest)` checks, their path gates, timeouts, feature checks,
Clippy passes, doctests and leak wrappers. Linux keeps the full workspace
nextest suite and its changed-crate shadow measurement. Mac compiles every
workspace test target, then runs the selection in
`scripts/macos-rust-tests.json`. Scheduled Nightly on main runs the full
leak-checked Mac workspace suite and doctests. Its report waits for Rust too.
The separately scheduled real-tunnel test and the per-push helper-feature
runs are unchanged.

The selection retains the CLI used by the Mac app, the native client,
transport, tmux and reachability packages; Darwin passwd/time/process-group
coverage in core; store ownership/recovery tests; daemon process/signature,
sandbox/Keychain-aware git, path/notification, FSEvents and native lifecycle
modules; and real daemon/PTY/sshd/sandbox/crash integration tests. Whole
modules and binaries are selected so an added test in one stays selected.
JSON entries record each scope's reason. Pure protocol, state, rendering,
parsing and workflow coverage continues on Linux and in the full Mac Nightly.

## Hosted job timing (fix round 1)

The hosted before/after measurement requested in review is **42m40s →
12m04s** (both jobs green). It is recorded in
[hosted-results.md](hosted-results.md), with job links, outcomes, complete
job wall times and per-step durations, plus the failed first baseline attempt. The local results below are retained
as supporting evidence from the initial build.

## Guard and validation

`python3 scripts/macos-rust-tests.py --self-test --check` runs in CI's always-on
wire job, without Cargo or a Mac. It scans every crate plus nonstandard
workspace member paths for Mac/Apple cfg predicates, including crate roots,
nested `all`/`any`, `cfg!`, `cfg_attr`, and platform-specific manifests. Custom
Cargo target roots are recognized as crate roots.
An uncovered source scope fails; crate roots and shared integration helpers
require the whole package. Explicit `#[path]` test modules follow their owner.
Native coverage without a source cfg (such as notify's FSEvents backend) is
also recorded in the JSON. Workflow checks reject narrowed Linux/Nightly
commands or removal of the full Mac build. The local lane-gate workflow reader
keeps the full unfiltered Rust command and skips the duplicate Mac CI replay;
its self-test requires exactly one full workspace command.
`lane-gate-red.log` records the expected rejection when the duplicate is restored.
The local self-test ran with `GIT_CONFIG_GLOBAL=/dev/null` and
`GIT_CONFIG_NOSYSTEM=1` so scratch-repository commits do not inherit this
host's global commit-signing configuration. Gate/evidence fixtures also run
with inherited Work-session variables removed, outside the sandbox: their
Git helper merges stderr into stdout, so the sandbox's `confstr()` warnings
otherwise corrupt fixture commit IDs. The excluded failed setup attempt is
retained in `validation-environment-attempt.zip`.

[selection-red.log](selection-red.log) records an actual exit-1 check after
removing FSEvents coverage, and an exit-1 check for a new Mac-only crate outside
`crates/`. The self-test plants uncovered cfg tests first, sees rejection,
adds the module/package/binary selection, and sees success. It also rejects
planted narrowing of Linux/Nightly and omission of the full Mac build.

Actionlint passes with `.github/actionlint.yaml` recording GitHub's `xcode-27`
public-preview label, which this installed actionlint does not yet know.
Three existing apt option strings were converted to equivalent Bash arrays
to satisfy its ShellCheck pass. The scoped command is
`actionlint .github/workflows/ci.yml .github/workflows/nightly.yml`. A full
repository actionlint run still reports pre-existing ShellCheck warnings in
untouched Canary, Release and doc-comment workflows (`actionlint-all.log`).
CI path-selection, local lane gate, Rust shadow, iOS shard
configuration/self-tests, shard budget and Nightly report self-tests pass.
Engine-gate compatibility tests (9) and gate-evidence fixture tests (16) also
pass in the isolated fixture environment described above.

## Measurement method

`measure.py` reproduces the Rust job's Cargo checks, feature tests, workspace
binary build, leak-checked nextest phase and doctests on this branch, using
this worktree's own `target/`. Run with:

```
/usr/bin/python3 scripts/heavy -- /usr/bin/python3 reports/ov-1160/measure.py
```

This is a **local Mac job reproduction**, not a GitHub-hosted wall-time
measurement. It excludes runner provisioning, package/tool downloads, cache
restore/upload, the green-base API call and shadow-planner bookkeeping.
The initial build did not push the branch or dispatch Nightly. The fix-round-1
hosted comparison above now supplies the separate end-to-end measurement. Historical hosted context is 41.5 minutes
median for Mac Rust in `reports/ov-1134/macos-queue.md`; it is not the baseline
of this local comparison.

The driver holds the shared heavy-job lock, uses three Cargo jobs and three
nextest threads, prewarms all feature/check artifacts, then runs full and
selected jobs sequentially with the same target and toolchain. Results record
per-step wall time, exit status, boundary load averages and commands. A warm
sequential comparison does not establish cold-cache CI savings. It does not
record continuous load, so it is not an under-20-load controlled experiment.

The driver removes inherited `FARCOOLER_*` Work session variables (retaining
`FARCOOLER_TEST_SHELL`, also set by Cargo). In particular,
`FARCOOLER_NATIVE_HOOK_SOCKET` overrides the explicit socket in hook tests
(`crates/cli/src/hook.rs:116`); leaving it set sends those tests to the agent's
private socket instead of their fixture. Names removed are recorded, never
values. The interrupted polluted run is retained in
`inherited-session-attempt.zip` and excluded from timings. An earlier
preparation interrupted to finalize native core coverage is in
`initial-prepare.zip`; no full baseline finished in that attempt.

## Results (14 November 2026)

| Measured local phase | Full Mac baseline | Selected Mac run |
| --- | ---: | ---: |
| Rust job reproduction wall | 967.10 s (16m07.10s) | 503.17 s (8m23.17s) |
| Leak-checked workspace test command | 951.69 s | 486.09 s |
| Nextest execution summary | 949.534 s | 484.452 s |
| Workspace test result | 6,999 passed, 1 failed | 2,601 passed |
| Listed default workspace inventory | 7,035 tests, 140 binaries | Same full workspace compiled |

The observed job-wall difference is **463.93 s (48.0%)**;
nextest execution fell by **465.082 s (49.0%)**. Selection avoids executing
4,399 of the 7,000 normally runnable Mac tests per push (62.8%). These tests
still run in full on Linux and in the scheduled Mac Nightly. Ignored/live
coverage retains its existing treatment. Nextest's filtered log says 3,887
tests and 25 binaries skipped because it does not enumerate test cases in
those wholly filtered binaries; that is not the total test-count difference.
Both test-command logs report a warm Cargo test build (0.31 s / 0.32 s).

All recorded Clippy, host/cross-target checks, helper-feature tests, binary
builds and doctests passed in preparation, before and after. The explicit
full-workspace `--no-run` build passed. Both wrappers passed on the selected
run; the baseline's file/preferences wrapper also reported no leftovers.

The full baseline is **not green**: the only failed test was
`a_resized_pane_says_so::a_grown_pane_is_announced_before_its_repaint_after_a_respawn_onto_a_new_tty`.
Its fixture failed at `crates/daemon/tests/a_resized_pane_says_so.rs:416`:
`/dev/ttys007 was never handed out again`. This same test is explicitly
retained and **passed in the selected run** (0.273 s). No Rust source or test
expectation was changed. This is a local PTY-allocation observation, not proof
of a product regression or a clean full-suite baseline. The full command
returned 100; the selected command returned 0. The research driver records
step failures and continues the comparison; it is not used as a CI gate.
CI's wrappers still propagate failures.

The shared host was busy: a mid-baseline spot check saw load averages
23.34 / 26.97 / 20.22. The recorded job boundary averages are:

| Run | Start load (1/5/15 min) | End load (1/5/15 min) |
| --- | --- | --- |
| Before | 9.41 / 11.83 / 12.49 | 7.80 / 16.97 / 17.71 |
| After | 7.80 / 16.97 / 17.71 | 7.56 / 10.67 / 14.12 |

Host contention differed, so 48% is an observed local reduction, not a
causal estimate for a GitHub runner or a promised pipeline improvement.
See the separate hosted comparison above for branch job timings; the local
figures in this section do not describe hosted runner performance.

## Evidence

- `timings.json`: exact commands, step wall seconds, statuses and load boundaries.
  HEAD at capture was `1b6373e`, with the guard/report dependency refinements
  committed afterward. No Rust product/test source changed in this ticket.
- `environment.json`: tool versions and source/selection/driver hashes.
- `logs.zip`: preparation, full, selected, doctest and inventory-build logs.
- `inventories.zip`: original nextest full and filtered inventories; the filter
  was parsed by nextest against reused build metadata without rebuilding.
- Individual validation logs retain selector, path-selection, shadow, shard,
  report, gate/evidence, actionlint and repository lint outcomes. `selection-red.log` records
  the deliberately failing guard cases.
