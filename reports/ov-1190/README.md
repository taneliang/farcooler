# Free CI runners (ov-1190)

Checked **2026-11-14**. `gh api repos/taneliang/farcooler` reports
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
with main CI waiting up to 70.3 minutes. Nightly now executes its Apple work
in phases: iOS build, UI matrix, captures matrix, Mac perf. Each matrix has
`max-parallel: 1`. `!cancelled()` retains later phases after a failed earlier
phase; `fail-fast: false` retains every matrix entry. Reporting still waits
for all jobs. Full scheduled and manual coverage and all shard names remain.
No job concurrency group discards pending matrix entries.

| Peak runnable jobs, ignoring path skips | Before | After |
| --- | ---: | ---: |
| One main CI: all / macOS | 11 / 4 | 12 / 4 |
| One Nightly: all / macOS | 16 / 13 logical, capped at five Macs by GitHub | 4 / 1 |
| One main CI plus one Nightly | Exceeds five Macs | At most 16 total / five Macs |

The peaks follow the workflow dependency graph, counting independent jobs
that can overlap; matrix entries count separately and `max-parallel` caps
each matrix. A single Nightly cannot crowd a single main CI out of either
free limit. The existing workflow-level `nightly` group prevents two Nightly
runs from overlapping. Main CI still cancels superseded runs per ref.

This is deliberately a reservation, not a universal no-queue guarantee:
GitHub offers no workflow priority reservation. Canary, Release, tunnel E2E,
other PR refs and other repositories using the same account can still
compete with main. The claim in acceptance item 3 cannot literally be
guaranteed account-wide by these repository workflows. No higher concurrency
or paid account limit has been assumed.

Nightly takes longer while preserving its work: its 12 UI/capture jobs alone
have a serial timeout envelope of 540 minutes, plus 15 for iOS and 40 for
Mac perf (595 minutes total). Individual jobs retain their own limits, so
this does not violate GitHub's six-hour *per-job* limit. The suite can finish
overnight without occupying all five Macs. Runner-minutes are not reduced by
serialization. Artifact retention remains long enough for these phases.

## Validation

- Both cgo modes: gofmt clean, `go vet ./...`, `go test -count=1 ./...` passed.
- `lane-gate.py --self-test` passed, including assertions for both Go modes.
- `ios-ui-shards.py --check` and `--self-test` passed: 62 classes placed once;
  all smoke coverage retained. New mutations reject unbounded matrices,
  overlapping capture/perf phases and success-only phase conditions. The
  original Nightly fails the new reservation checks.
- Nightly report, shard-budget, CI job-time and CI changes self-tests passed.
- File-size budget and `git diff --check` passed.
- actionlint passed with shellcheck disabled and only its outdated
  `xcode-27` label diagnostic ignored. Unfiltered actionlint also reports
  existing SC2086 apt-option expansions in unchanged Rust setup steps.

Hosted after timing and a comparable three-day queue-p90 comparison remain
follow-up measurements after landing; local timings cannot substitute for
those observations.
