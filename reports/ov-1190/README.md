# Free CI runners (ov-1190)

Runner/billing docs checked **2026-11-14**; concurrency docs rechecked **2026-11-15**. `gh api repos/taneliang/farcooler` reports
`visibility: public`. No self-hosted or larger runners are introduced.

## What GitHub makes free

[GitHub's runner reference](https://docs.github.com/en/actions/reference/runners/github-hosted-runners#standard-github-hosted-runners-for-public-repositories)
lists standard public-repository runners as free and unlimited: Ubuntu x64
(`ubuntu-latest`, `ubuntu-24.04`, etc.) and arm64 (`ubuntu-24.04-arm`, etc.)
have four CPUs and 16 GB RAM; macOS arm64 (`macos-latest`, `macos-26`,
`xcode-27`) has three M1 cores and 7 GB RAM. Standard Intel macOS and Windows
runners are also in that free table. ARM Linux is an alternative architecture,
not a documented CPU-count upgrade. Our Apple jobs already use free ARM Macs.
Moving them to a `-large` or `-xlarge` label would violate this ticket.

[GitHub's billing documentation](https://docs.github.com/en/billing/concepts/product-billing/github-actions#how-use-of-github-actions-is-measured)
explicitly bills larger runners even for public repositories. There is no
public-repository exemption for those machines. Free standard compute does
not exempt artifact/cache storage from its separate allowances; this change
adds no artifacts or retention.

[Actions limits](https://docs.github.com/en/actions/reference/limits#job-concurrency-limits-for-github-hosted-runners)
lists Free at 20 total jobs and five macOS jobs; Pro has 40 total but still
five macOS. These are plan/account limits, not a guarantee of immediate
capacity for this repository. The macOS limit includes larger runners too.
We use the conservative 20/five budget rather than assume an upgraded plan.

Audit of every checked-in workflow, including local reusable workflows:
the only concrete labels are `ubuntu-latest`, `ubuntu-24.04`,
`macos-latest`, and `xcode-27`; Rust's `matrix.os` expands to the second and
third of those. All are standard free public-repository labels. Label audit:
30 Ubuntu-latest job definitions, two Ubuntu-24.04, two macos-latest, twelve
xcode-27, counting matrix values and reusable definitions once.

## Change and minutes

The tunnel checks had two mutually exclusive Go builds tested serially.
`Tunnel (Go)` retains gofmt, cgo=1 vet and uncached tests, preserving its check
name. `Tunnel helper (Go)` independently vets and tests cgo=0 on another free
Ubuntu runner. Both retain module caching and the pinned Go version from
go.mod. The local `lane-gate --only tunnel` still runs BOTH variants.

The hosted baseline is [green main CI 43595750803](https://github.com/taneliang/farcooler/actions/runs/43595750803),
commit `73974f89898d716c36595eade2511ba5f8df7a8f`:

| Measurement | Before, minutes | After, minutes | Basis |
| --- | ---: | ---: | --- |
| Hosted tunnel job wall time | 1.45 | 1.13 estimated | 87 s total minus the 19 s cgo=1 step, assuming identical setup/teardown on each shard |
| Hosted summed tunnel runner time | 1.45 | 1.77 estimated | 87 s plus a second 20 s setup/teardown envelope |
| Hosted Go check critical path, setup excluded | 1.12 | 0.80 estimated | 19 s + 48 s becomes max(19 s, 48 s) |

These after figures replay the baseline timings; they are **not measured
post-change GitHub runs**. They predict about 0.32 minutes saved on this
check at the cost of 0.32 extra free runner-minutes. Tunnel is shorter than
Rust/Apple jobs, so this does not claim a faster whole CI critical path.
Changing workflow files and committing locally does not produce a hosted
run. The next green branch/main run must verify setup/cache effects and the
new helper job's timeout budget. No branch was pushed or workflow dispatched.

A separate local before/after experiment runs the exact vet/uncached-test
commands serially and with two workers, after warming both build variants.
The measured median fell from **0.388 to 0.204 minutes** (47.3%); median
summed check time was 0.388 versus 0.401 minutes (3.5% higher).
`measure-tunnel.py` alternates order across three repetitions and records wall
and summed check minutes in `tunnel-local.json`. This checks the scheduling
benefit, not Linux VM speed or GitHub queue latency. Host: macOS arm64,
Go 1.27.3. Reproduce with:

```sh
TMPDIR=/tmp GOCACHE=/tmp/ov-1190-go-cache python3 reports/ov-1190/measure-tunnel.py
```

## Reserve the macOS pool for landings

[ov-1134](../ov-1134/macos-queue.md) measured Nightly filling all five Macs,
with main CI waiting up to 70.3 minutes. The first revision limited Nightly
to one Mac, but that still let Canary take two more while a landing needed
four. The review correctly rejected that reservation.

All repository background Apple work now shares the constant
`background-macos` concurrency group: Nightly takes it per Apple job;
Canary, Release and Tunnel end to end take it per workflow. Canary and
Release order their two Apple jobs so each workflow uses at most one Mac.
CI does not join this group. Across different refs, scheduled and manual
runs, background work therefore occupies at most one of the five Macs,
leaving four for a landing's CI. This also covers overlapping Canary runs,
Release through Promote, and manual tunnel checks.

[GitHub's concurrency documentation](https://docs.github.com/en/actions/how-tos/write-workflows/choose-when-workflows-run/control-workflow-concurrency)
(checked **2026-11-15**) permits a shared group across workflow and job
scopes. `cancel-in-progress: false` protects work in flight; `queue: max`
retains up to 100 pending jobs/runs rather than replacing the previous pending
entry. FIFO is by entry into the group, not workflow dispatch time. The same
queue mode is added to Nightly's outer `nightly` group so manual runs wait
instead of replacing another pending night. The outer group is distinct from
the per-job group, avoiding a workflow waiting for a lock it already holds.

Nightly retains its phase chain, `max-parallel: 1` on each matrix,
`fail-fast: false`, and `!cancelled()` between phases. Canary/Release can
acquire the background slot between Nightly shards, rather than waiting for
the whole suite. Their Apple builds run sequentially. A failed or disabled
iOS Canary must not suppress its Mac shipment: the Mac condition explicitly
requires successful wire and Linux jobs and a build plan, while allowing the
iOS dependency to fail or skip. Release similarly retains iOS after a failed
Mac build. No lint, shard, ship step, timeout or artifact is removed.

| Maximum runnable jobs | Before ticket | First revision | Fixed revision |
| --- | ---: | ---: | ---: |
| One landing CI, all / macOS | 11 / 4 | 12 / 4 | 12 / 4 |
| One Nightly, all / macOS | 16 / 13 logical | 4 / 1 | 4 / 1 |
| Canary alone, all / macOS | 4 / 2 | 4 / 2 | 4 / 1 |
| Landing + Nightly + Canary, macOS | over five | up to seven | five |

`scripts/ci-macos-capacity.py` computes weighted dependency antichains from
these workflows, including matrix widths and the two-job Linux reusable.
It checks every background Mac is protected, each shipping workflow's Mac
width is one, and CI's Mac width is at most four. It also computes a
conservative **20/20 total-job upper bound**: landing 12 + all four Nightly
Linux jobs + the widest shipping workflow's four jobs. This overcounts
Nightly Linux jobs that depend on one another, but stays within the Free
limit. Canary, Release and tunnel cannot overlap each other because their
whole workflows share the group.

This validates the review's overlapping Canary/Nightly/landing scenario in
either arrival order: whichever background entry owns the group, all others
wait without a runner; landing CI still has four Mac slots. Mutation tests
reject a private Canary group, overlapping Canary/Release Apple jobs, a
Nightly group that cancels or replaces pending work, an unprotected tunnel
workflow, and an additional parallel Nightly Mac job. The check runs in CI's
iOS planning job, which runs even when paths skip the app builds.

Tradeoffs: shipping may wait for the current Nightly shard (up to its
45-minute timeout), and holds the shared slot during Linux/setup/quiet work
as well. Canary's Apple portion now has a 90-minute serial timeout envelope;
Release's is 150 minutes. Existing per-job supersession remains, but whole
Canary workflows now queue instead of overlapping. This spends no extra
runner-minutes on the queue and sacrifices background latency to preserve
landing capacity. Nightly's own Apple envelope remains 595 minutes before
queue waits; actual wall time must be measured after landing.

The reservation applies to one landing CI plus this repository's background
workloads. Other CI refs or other repositories in the account can still use
the account's runner limits; GitHub does not promise immediate hosted capacity.
Queues over GitHub's documented 100-entry maximum can still cancel excess
entries. No higher account limit or paid runner is assumed.

## Validation

- Both cgo modes: gofmt clean, `go vet ./...`, `go test -count=1 ./...` passed.
- `lane-gate.py --self-test` passed, including assertions for both Go modes.
- `ios-ui-shards.py --check` and `--self-test` passed: 62 classes placed once;
  all smoke coverage retained. New mutations reject unbounded matrices,
  overlapping capture/perf phases and success-only phase conditions. The
  original Nightly fails the new reservation checks.
- Nightly report, shard-budget, CI job-time and CI changes self-tests passed.
- Fix round: the capacity and wire-gate self-tests cover the shared background
  queue and explicit Mac shipping gates; relevant workflow/script checks rerun.
- Installed actionlint 1.7.12 predates `queue: max`. It passed on all five
  modified workflows with shellcheck disabled and only the documented queue
  key plus the existing `xcode-27` label diagnostic filtered. Unfiltered lint
  reports eight queue-key diagnostics; GitHub's current syntax docs support
  each of those keys.
- File-size budget and `git diff --check` passed.
- actionlint passed with shellcheck disabled and only its outdated
  `xcode-27` label diagnostic ignored. Unfiltered actionlint also reports
  existing SC2086 apt-option expansions in unchanged Rust setup steps.

Hosted after timing and a comparable three-day queue-p90 comparison remain
follow-up measurements after landing; local timings cannot substitute for
those observations.
