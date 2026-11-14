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

All repository background work now shares the constant `background-macos`
concurrency group, including Linux work. Nightly takes it per job; Canary,
Release, Tunnel end to end, Doc comments, Site, Web, Relay, both pricing checks,
DERP hosts and Site analytics take it per workflow. Promote takes it in its
`tag` and `baseline` jobs; its three reusable calls acquire it inside Release,
Relay and Site. Promote never holds a parent lock while a called workflow
waits for that same lock. CI does not join the group.

Canary and Release order their Apple jobs so each uses at most one Mac.
Across refs, scheduled and manual runs, background work therefore occupies
at most one Mac, leaving four for a landing's CI. The group also caps total
background jobs, including deployments triggered by that landing.

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
| One Nightly, all / macOS | 16 / 13 logical | 4 / 1 | 1 / 1 |
| Canary alone, all / macOS | 4 / 2 | 4 / 2 | 4 / 1 |
| Landing + Nightly + Canary, macOS | over five | up to seven | five |

`scripts/ci-macos-capacity.py` discovers **every** workflow and computes
weighted dependency antichains, recursively expanding local reusable calls.
It checks every independent background job or workflow is protected and
rejects a parent/child lock that would deadlock. It models Site's dynamic
matrix as two channels on a push (one on dispatch/call), and Linux binaries
as its actual two architecture jobs. Unknown triggers or unsupported matrix
shapes fail the check instead of silently disappearing from the budget.

The second review found that the previous **20/20** calculation omitted
Doc comments, Site, Web and Relay on the same landing. It also left Linux
Nightly jobs outside the queue. Those workloads now enter the same shared
group. With the simultaneous path-changing main push, Nightly, Canary and
deployments all eligible, the conservative bound is:

| Concurrent reservation | Jobs | macOS |
| --- | ---: | ---: |
| One landing CI | 12 | 4 |
| One shared background entry (widest is Canary) | 4 | 1 |
| Independent Canary wire-baseline recorder | 1 | 0 |
| Total | **17 / 20** | **5 / 5** |

The Canary recorder intentionally retains its existing no-concurrency rule,
so recording shipped wire compatibility cannot be dropped by a queue. The
reservation includes one such completion run and leaves three spare Linux
slots. All other repository scheduled, push, PR, dispatch and reusable work
shares the background entry: path filters and disabled deploy variables are
conservatively assumed to match. A Site workflow's two deployment channels,
for example, run inside that entry instead of adding two jobs beside Canary.
This includes Promote's direct jobs and its nested Release/Relay/Site calls.

Mutation tests reject escaping Doc comments, Site, Web, Relay, Nightly Linux,
maintenance or Promote jobs; the simultaneous deployment mutation must report
all four originally omitted workflows. A newly discovered unprotected push
workflow and a CI matrix that pushes total usage over 20 also fail. Earlier
mutations still reject private Canary groups, overlapping shipping Macs,
replacement/cancellation of Nightly work and unprotected tunnel work. The
check runs in CI's iOS planning job even when paths skip app builds.

Tradeoffs: all deployments, doc-comment checks and maintenance now wait behind
the current background entry, including across environments for Relay. The
queue retains those runs rather than replacing pending ones. Nightly still
releases the slot between jobs; a shard can hold it for its 45-minute timeout.
Canary holds it through its quiet wait (up to 70 minutes), Linux builds and
serial Apple builds (90-minute Apple timeout envelope); Release's Apple
envelope is 150 minutes. Nightly's own Apple envelope remains 595 minutes
before queue waits, with its Linux checks now serialized too. These changes
add queue delay, not compute minutes; post-landing runs must measure wall time.
No lint, shard, deployment, timeout or artifact is removed.

The reservation applies to one landing CI, the repository's queued background
work and one Canary completion recorder. Concurrent CI refs, multiple old
recorder runs or other repositories can still consume the account's limits;
GitHub does not promise immediate hosted capacity. Queues over GitHub's
100-entry maximum can still cancel excess entries. No higher account limit
or paid runner is assumed.

## Validation

- Both cgo modes: gofmt clean, `go vet ./...`, `go test -count=1 ./...` passed.
- `lane-gate.py --self-test` passed, including assertions for both Go modes.
- `ios-ui-shards.py --check` and `--self-test` passed: 62 classes placed once;
  all smoke coverage retained. New mutations reject unbounded matrices,
  overlapping capture/perf phases and success-only phase conditions. The
  original Nightly fails the new reservation checks.
- Nightly report, shard-budget, CI job-time and CI changes self-tests passed.
- Fix round 2: capacity self-tests passed for all discovered workflows,
  expanded reusable calls, the simultaneous deployment mutation, unknown new
  workflows and total-job overflow. Wire lint: 107 passed, zero failed.
- Nightly report, shard coverage, shard budget, job-time and change-planning
  self-tests passed; the file-size budget and `git diff --check` passed.
- Installed actionlint 1.7.12 predates `queue: max`. All workflows passed with
  shellcheck disabled and only its unsupported queue key and existing
  `xcode-27` label diagnostic ignored. GitHub's current syntax documentation
  supports the queue key; limits and queue semantics rechecked 2026-11-15.

Hosted after timing and a comparable three-day queue-p90 comparison remain
follow-up measurements after landing; local timings cannot substitute for
those observations.

## Nightly yields to main CI (ov-1226)

CI runs on different refs do not share a concurrency group, so Nightly could
still start Apple work while a landing's CI was queued. Each Nightly Apple phase
(`ios`, `ios-ui`, `captures`, `mac-perf`) now `needs` a Linux `yield-*` job that
runs `scripts/ci-yield-to-main.py`: it polls `gh run list --workflow ci.yml
--branch main` and waits while a run is queued or running, then lets the phase
start. Waiting happens on a free Linux runner, outside `background-macos`, so no
Mac is held. It gives up after 90 minutes and proceeds (a yield, not starvation).
The capacity model counts one gate (total bound now 18/20) and rejects a Nightly
Mac job with no gate. `python3 scripts/ci-concurrency-test.py` runs the capacity
and yield self-tests.

Remaining gap, stated plainly: the gate runs once per phase, so a main push
arriving mid-phase (for example during the eight-shard iOS UI matrix) still sees
one background Mac in use; the four-Mac landing reservation covers that. PR-ref
CI runs are not yielded to. Follow-up ticket text: "CI: Nightly yields per shard
and to PR runs — move the yield check into each Nightly Apple matrix shard
(before it takes background-macos) and measure main CI queue time over a week."

Runner labels used and their public free-tier source (GitHub's standard
runners for public repositories, [runner reference](https://docs.github.com/en/actions/reference/runners/github-hosted-runners#standard-github-hosted-runners-for-public-repositories)):
`ubuntu-latest`, `ubuntu-24.04`, `macos-latest`, `xcode-27` (macOS arm64). New
gate jobs use `ubuntu-latest`. No self-hosted or larger runner appears in `.github/`.
