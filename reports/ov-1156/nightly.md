# Nightly dispatch and one-slot estimate (ov-1156)

Nightly serializes iOS build, the UI matrix, the capture matrix, and Mac perf.
Each matrix has `max-parallel: 1`; the next job waits even when the preceding
job fails. `python3 scripts/nightly-dispatch-test.py` dry-checks every choice,
the full schedule and default, the matrix exclusions, and the dependency chain.
A targeted dispatch runs the iOS build and one selected shard or capture set.
Only full runs perform the Linux maintenance jobs and update the Nightly issue.

At one slot, a rough full-run estimate from [ov-1134](../ov-1134/macos-queue.md)
is **350 minutes (about 5 hours 50 minutes)**: the 6.8-minute median iOS
build plus 13 jobs (12 UI/capture jobs and Mac perf) at the observed
26.4-minute median Nightly macOS job duration. This is a coarse estimate,
not a sum of per-job medians: the sample includes different job types and
branch versions, and actual runtime, queue delays, and failures can vary.
Linux jobs overlap this chain; runner queueing and workflow concurrency wait
are additional. Targeted runs take approximately the build plus one job.
