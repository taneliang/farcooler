# ov-1160: hosted macOS Rust job wall times

Measured on 14 November 2026, on branch `ov-1160-mac-rust-only` at
[`c735b5f3a`](https://github.com/taneliang/farcooler/commit/c735b5f3a).
Both comparison jobs passed.

| Hosted job | Outcome | Complete job wall time |
| --- | --- | ---: |
| [Full baseline, first attempt](https://github.com/taneliang/farcooler/actions/runs/43563607545/job/132352489601) | failure | 39m57s (2,397 s) |
| [Full baseline, unchanged rerun](https://github.com/taneliang/farcooler/actions/runs/43563607545/job/132363691556) | success | 42m40s (2,560 s) |
| [Selected (after)](https://github.com/taneliang/farcooler/actions/runs/43563607545/job/132352489463) | success | 12m04s (724 s) |

The observed wall-time difference between the baseline rerun and the selected
job is **30m36s (1,836 s) (71.7%)**.
This is one hosted comparison with different runner conditions, not a causal
estimate or a promised saving. The failed first baseline is retained above.

| Phase | Full baseline rerun | Selected |
| --- | ---: | ---: |
| Setup, checks, feature tests and binary build before default suite | 12m01s (721 s) | 6m04s (364 s) |
| Default suite step, including test compilation and wrappers | 30m18s (1,818 s) | 5m53s (353 s) |
| Nextest execution (from log summary) | 1291.957 s | 231.255 s |
| Default suite result | 7000 tests run: 7000 passed, 35 skipped | 2601 tests run: 2601 passed, 3887 skipped |
| Workspace doctests | success | success |

The per-step timestamps and outcomes are in `hosted-step-timings.log` and
`hosted-jobs.json`. They separate common setup from selection-related work.
The selected job explicitly compiled every workspace test target with
`cargo nextest run --profile ci --workspace --no-run` before filtering.
All helper-feature tests and Clippy/target checks are the original CI steps.

## What ran and how it was measured

[Hosted workflow run](https://github.com/taneliang/farcooler/actions/runs/43563607545).
A temporary branch-only push workflow ran the complete `ci.yml` Rust job on
GitHub-hosted `macos-latest` runners, in two coverage variants. Both checked
out the same commit. The exact executed workflow is `hosted-timing.yml` here;
the temporary workflow was removed from `.github/workflows/` afterward.
Required CI and Nightly were not changed in this fix round.

The full baseline restores the pre-selection Mac behavior: green-base lookup,
Rust shadow planner and inventories, the full leak-checked workspace nextest
command, shadow artifact upload, and doctests. Because this branch changes
workflow/scripts, shadow conservatively proposes the full workspace; it does
not run a narrow replay. Its load waits and inventory/build work are included
in the job and default-suite step wall times. The selected variant uses the
exact per-push Mac steps in `ci.yml`, including the explicit full test build.
The experiment removes only the `changes` dependency/path gate, runs only Mac,
and switches the before/after coverage steps and their bookkeeping.

`verify-hosted.py` reverses those switches and requires a byte-for-byte match
with the complete current CI Rust job. It also checks the baseline uses the
exact full-suite command (including shadow) retained by Linux, both jobs used
one source SHA, and both completed successfully. Run:

```
python3 reports/ov-1160/verify-hosted.py
```

Wall time is GitHub's `completed_at - started_at` for the actual job, including
checkout, tool installation, cache restore, Clippy, target/feature checks,
builds, test execution, doctests, uploads and post-job cleanup. Runner queue
wait is excluded. This is not a local reproduction or summed test duration.
The selected job ran once in attempt 1; GitHub carries its successful result
and original timestamps into attempt 2. The table links its actual original
job, not the carried-forward job entry.

Both comparison jobs used image `macos-26-arm64` and restored identical Rust
and pinned-tmux cache keys:

- `tmux-rust-macOS-ARM64-a8c4363c9e4c3d98fd9ee7d06ce6840701b0b904ac57d42ca9995f14b4efd8c8`
- `v0-rust-macos-latest-rust-Darwin-arm64-60948658-f997da26`

Cache hits do not make these cold-cache measurements. Common setup durations
and the shadow load waits differ, so the total difference cannot all be
attributed to skipping platform-independent tests. The logs and phase table
expose those differences.

## Failed first baseline and rerun

The first full job ran all 7,000 default-feature tests: 6,999 passed, one failed,
35 ignored tests were skipped. The failure was
`workflow_runtime::coding_native::recovery_tests::workflow_native_review_vanished_server_keeps_output_and_recorded_verdict`:
`coding_native.rs:799` observed `Uncertain` where the fixture expected `Failed`.
That exact test is retained by the selection and passed in the selected hosted
run (3.713 s). The baseline log also records a shadow load wait at 25.9.
No product code, test, timeout, concurrency setting or test expectation was
changed for the rerun. The whole failed baseline job was rerun at the same SHA.
Its first-attempt metadata, logs and shadow artifact remain checked in.
The original temporary shadow artifact was downloaded and removed before the
rerun so its unchanged upload step could reuse the artifact name.

Two preliminary setup runs (43563450652 and 43563528748) were cancelled before
measurement while restoring the baseline's shadow bookkeeping and artifact
upload; they are excluded from all reported comparisons.

## Evidence and validation

- `hosted-run.json`, `hosted-jobs.json`: GitHub API metadata after the rerun.
- `hosted-step-timings.log`: verifier output with all completed step durations.
- `hosted-timing.yml`: exact executed workflow at the measurement commit.
- `hosted-attempt-1-run.json`, `hosted-attempt-1-jobs.json`: original attempt.
- `hosted-attempt-1-logs.zip`: complete first full and selected job logs.
- `hosted-attempt-1-shadow.zip`: first baseline inventories/load/timing data.
- `hosted-baseline-rerun.log.zip`, `hosted-baseline-rerun-shadow.zip`: rerun evidence.
- `fix-round-1-validation.log`: selector red/green, CI selection, actionlint,
  local gate, Rust shadow, Nightly and iOS shard compatibility checks.

Linux still runs the full workspace suite. Nightly on main still runs the
full macOS suite. No test was removed from every platform and no required
check was weakened. The earlier local comparison is retained in `README.md`
and is separate from these hosted results.
