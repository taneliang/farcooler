# ov-1110: faster builds and a smaller test run

Measured at source `d986f3083730db7cda3b91caf5e536ba321b7c1e`, 12 November 2026.
This commit contains research and captured evidence only. No product, profile,
test inventory, gate or CI behavior changes.

**13 November resume:** [fresh local results](resume-local/summary.md) now include
another cold/warm Cargo timing pair and a completed 6,928-test local nextest run
at sampled load below 20 (6,916 passed, 12 failed). The fresh full landing gate
was interrupted after 464.77 s when load reached 21.37 / 11.70 / 8.65. Its
completed-step timings are retained, but acceptance 1's complete controlled
full-gate baseline remains blocked. Earlier sections below retain the original
research and historical evidence; the resume results supersede their statement
that no local test execution completed.

## Findings and recommendation

A fresh default workspace build costs **152.61 s**, while the unchanged warm
build costs **0.40 s** on this Mac. Reusing valid artifacts matters much more
than tuning the last link. But compilation is only part of the problem: in the
latest green macOS Rust CI job, the workspace test step spends **8m24s building
and 20m16s executing**. An unchanged warm build does not avoid executing tests.

Start with a shadow changed-crate selector, then integrate prospective exact-tree
gate reuse with ov-903. Both can avoid minutes of redundant execution, but neither
may skip anything until input provenance and consumer coverage are demonstrated.
Next benchmark compiler-cache misses against the fold-target handoff already in
source. Finally investigate Mac UI contention before splitting tests. Keep full
coverage while these experiments establish their safety and actual savings.

The ticket's initial assumptions have partly aged: nextest **is installed locally**
(0.9.148), job-family selection already exists (ov-1021), lane debug info is already
off (ov-837), and engine fold gates already request `--keep-target` for handoff
(ov-1084). There is no installation-only nextest win on this machine. Ubuntu's
current five-run sample is faster than the older 25–31 minute comment in CI;
macOS Rust is now the longest CI job.

## Measurement conditions and limits

- Machine: Apple arm64, 10 CPU cores, 24 GiB RAM; Rust 1.98.1, LLVM 22.1.8.
- Fresh, previously nonexistent `target/ov1110`; registry downloads already cached.
  This is a **cold artifact build**, not a cold network or OS filesystem cache.
- Ran `cargo build --workspace --locked --offline --timings` twice, with
  `CARGO_BUILD_JOBS=3`, `CARGO_PROFILE_DEV_DEBUG=0`, and that isolated target.
  Three jobs deliberately leave headroom; do not extrapolate to a 3-core CI runner
  or a release/cross-target/Go-linked build.
- Used `scripts/heavy`'s machine lock. All 2-second build samples were below 20:
  maximum 1/5/15-minute averages **11.56 / 15.19 / 19.32**. The warm run was shorter
  than the sampling interval; its ending averages were **7.13 / 12.08 / 17.37**.
- Both builds exited 0. Cargo's HTML reports and logs are committed here, with the
  wall timer and load samples in [build-results.json](build-results.json).
  Extract `timings.zip` to open the original interactive Cargo HTML reports.
  [measure-build.py](measure-build.py) is the exact research driver used; it is
  not a gate or a general benchmark tool. Use a new target for another cold run.
- An optional local nextest baseline did **not** complete. First, a shebang hit
  a broken Python on PATH (exit 126, no tests started). Retried with explicit
  `/usr/bin/python3`, a sanitized tool PATH, the two leak wrappers and three
  build/test jobs. Machine load rose above the limit during compilation; only
  this task's nextest/Cargo descendants were interrupted. It exited 254 after
  57.78 s, before tests ran. [test-results.json](test-results.json) and
  [local-nextest.log](local-nextest.log) retain this excluded attempt. It is
  neither a performance baseline nor a passing test result. No other task's
  processes were signaled and no product fix was attempted.
- CI and landing data below are **historical observations**, downloaded/read
  without running more heavy jobs. Their host load was not recorded. They are
  useful elapsed-time evidence, not controlled under-20 experiments. A new full
  local landing baseline remains unmeasured; do not claim otherwise.

## What compilation is doing

The [cold timing report in timings.zip](timings.zip) contains 400+ compilation units; the
[warm report in the same archive](timings.zip) records no compilation. The full ranked table and
frontend/codegen sections are in [tables.md](tables.md).

| Unit | Wall seconds | Interpretation |
| --- | ---: | --- |
| aws-lc-sys native build script | 46.59 | Native cryptography compilation, not Rust proc macros or a final executable link |
| farcooler-daemon library | 16.27 | 9.06 frontend, 7.21 codegen; a large crate on the CLI dependency path |
| syn 2.0.119 | 14.21 | 1.41 frontend, 12.80 codegen; full parsing/visiting feature set |
| syn 3.0.3 | 13.40 | 1.01 frontend, 12.39 codegen; a second incompatible major version |
| russh | 10.82 | 2.41 frontend, 8.41 codegen, after the native crypto dependency |
| serde_derive / clap_derive | 9.16 / 8.34 | Procedural-macro compilation; their execution in consumers is not separately timed |
| bundled libsqlite3-sys native build | 8.94 | Compiles bundled SQLite |
| tokio | 8.38 | 2.47 frontend, 5.91 codegen |
| CLI final binary / daemon final binary | 4.64 / 2.55 | Whole compiler invocations including linking, not isolated linker measurements |

The native crypto unit runs from t=85.02 to 131.61; russh from 133.08 to 143.90,
client to 147.93, and the CLI finishes at 152.57. That is an observed long tail
through native crypto, SSH, client and CLI. It is not proof that removing 46.59 s
from crypto shortens the critical path by exactly that amount: other branches
can become the bottleneck. Units overlap; adding their durations is not build
wall time. The duplicate syn versions and derive crates explain real proc-macro
support work, but deleting a required major version is not a safe quick fix.

The workspace optimizes third-party dependencies at level 2 even in dev/test
(ov-993). That pays codegen cost once to reduce runtime repeatedly; dropping it
may make cold builds faster and test execution slower. Dev/test incremental
compilation remains enabled. Cargo defaults to 256 codegen units for incremental
builds and 16 otherwise; this source does not override them. Neither codegen-unit
count nor linker-only time was A/B measured here. Cargo timings expose codegen
sections for some units and do **not** isolate binary linking. The CLI's entire
4.64 s compiler invocation bounds the possible saving from changing only its
linker in this one build. Release ThinLTO is a different path.

These interpretations use Cargo's official [timings guide](https://doc.rust-lang.org/cargo/reference/timings.html)
and [profile reference](https://doc.rust-lang.org/cargo/reference/profiles.html).

## Where gate and CI time goes

All job medians/ranges and raw per-step local rows are in [tables.md](tables.md)
and [historical-gate.txt](historical-gate.txt). The local receipt is v1, source
`5b7a40d4e1a8d5647aba6522b57a70c03ce50904`, copied from the engine fold's Git
worktree directory. Its executed steps total **2806 s / 46m46s**, excluding lock
queue time and orchestration overhead. Rust accounts for 911 s, Swift tests
722 s, Android 564 s, bundle build/checks 341 s, wire 193 s, web 53 s, relay
22 s, and doc-comments rounds to zero. Within Rust, the workspace nextest step
is 665 s; the raw log separates a 411.750 s execution summary from compilation
and wrapper overhead. The Mac test wrapper costs 429 s, release core build
150 s, Android Robolectric 324 s and release assembly 106 s.

That receipt has no tunnel, site or iOS rows. This means **not measured there**,
not zero-cost or safe to omit. It deliberately leaves Linux web downloads and
some browser/wasm steps to CI. It is timing evidence only: its v1 provenance
cannot justify reusing a successful gate. No new full gate was run for this spike.

Downloaded the latest five successful main CI runs available at measurement:
[43205819484](https://github.com/taneliang/farcooler/actions/runs/43205819484),
[43193367002](https://github.com/taneliang/farcooler/actions/runs/43193367002),
[43172002972](https://github.com/taneliang/farcooler/actions/runs/43172002972),
[42950509104](https://github.com/taneliang/farcooler/actions/runs/42950509104),
[42942492365](https://github.com/taneliang/farcooler/actions/runs/42942492365).
Job receipts include step timestamps and conclusions; skipped jobs are excluded
from medians and their sample counts are explicit. These are separate changes
with varying cache states, not repeated trials of one tree.

| CI job | Median | Observed range |
| --- | ---: | ---: |
| Rust macOS | 41m55s | 38m13s–48m25s |
| Swift tests | 24m03s | 21m27s–24m47s |
| Android | 22m01s | 21m50s–23m58s |
| Rust Ubuntu | 21m51s | 17m59s–23m03s |
| Mac bundle | 18m48s | 15m10s–22m34s |
| Web | 11m59s | 11m36s–16m45s |
| iOS UI smoke | 9m01s | 8m42s–10m34s |
| iOS | 7m06s | 6m26s–7m46s |

The other six cheap/job-planning families are included in the complete table.
CI jobs overlap on different runners, so their sum is **not** pipeline latency.

For latest run 43205819484, macOS's workspace nextest step is **1728 s**:
504 s test compilation plus 1216.270 s execution of 6928 tests, with 33 skipped
(the normal ignored/live tests). Ubuntu's is **686 s**: 252 s compilation plus
429.117 s execution of 6905 tests, 33 skipped. Platform-specific inventory
explains different counts. Clippy's first pass costs 102/52 s on Mac/Ubuntu;
feature-specific checks and builds add separate invocations. Those checks
validate distinct helper/linked/stub paths; combining flags indiscriminately
would delete coverage. Android's latest Robolectric step costs 638 s, greater
than its 165 s Rust/native cross-build and 207 s release assembly.
[ci-rust-phases.log](ci-rust-phases.log) preserves the build/execution boundaries.

## Slow tests and selection model

[tables.md](tables.md) lists the **slowest 20 observed individual Swift/Rust tests**
from the latest green run and, separately, the slowest 20 Rust observations.
[slow-tests-extract.log](slow-tests-extract.log) retains timestamped original
lines after removing terminal color codes. Swift suite/run summaries are excluded;
parameterized test aggregate durations remain identified. This is the timed
Swift/Rust inventory, not an assertion that every Android/UI test has an
individual duration in the downloaded text.

The top observed test is Mac UI's hover-delay test at 102.906 s, followed by the
tiled composer-height sweep at 101.273 s and scrolled-session layout at 92.259 s.
They overlap; these are elapsed observations, not exclusive CPU seconds. Many
are `@MainActor`, and `.serialized` orders a suite rather than all suites in the
process. The height sweep constructs 58 windows, and the hover test calls settle
around UI events. Its configured one-hour delay is canceled, not slept through.
An isolated/suite comparison is needed before blaming the tests' own work or
promising linear gains from splitting them.

The longest Rust observation is the remote-download timeout/retry test at
18.134 s (16.436 s Ubuntu). It intentionally tests a five-second timeout with
retry and cleanup. The remote-target security test costs 15.474 s, every-transport
rows test 15.210 s, and workflow socket lifecycle 14.067 s. Do not remove timeout,
security or restart assertions to make a benchmark green. The tmux 300-round
stress test is **already split into four interleaved 75-round tests**; another
blanket split is not the starting point.

A conservative reverse dependency model using captured workspace metadata,
including declared dev/build dependencies, gives the following **Mac CI
observations**. The denominator 6973 includes 6928 default-suite tests and 45
feature-step observations, so some tests appear more than once. This is a
package-graph model only; it is not yet a sound selector for this repository.

| Changed crate | Tests selected | Summed observed seconds retained | Potential observation-seconds avoided |
| --- | ---: | ---: | ---: |
| CLI | 688 / 6973 | 70 / 3649 | 98.1% |
| review | 4764 / 6973 | 3396 / 3649 | 6.9% |
| store | 5375 / 6973 | 3524 / 3649 | 3.4% |
| core | 6355 / 6973 | 3625 / 3649 | 0.7% |
| protocol | 6640 / 6973 | 3633 / 3649 | 0.4% |

A low-level shared change reaches almost all expensive work even when it omits
many tiny tests. Savings concentrate in leaf changes. Do **not** multiply these
percentages by job wall time: scheduler groups, compilation, platform variants
and out-of-graph consumers affect the actual result. In particular Mac tests and
daemon shim tests execute the CLI without a Cargo dependency edge identifying
all those test consumers. The CLI-only row would be unsafe as a gate today.

Nextest's official [filterset reference](https://nexte.st/docs/filtersets/reference/)
supports `rdeps(package)` for transitive dependents. Explicit package selection
can avoid compiling omitted test binaries; runtime filtering alone should not
be assumed to save compilation. Unknown/global paths, fixtures, manifests,
lockfiles, build scripts, generated inputs, CI/toolchain/config changes, deleted
paths and untracked inputs must fall back to full coverage. Record the required
feature/target matrix and doctests too. Existing path-family selection and the
last-green-ancestor rule must remain intact.

## Options, expected benefit, risks and checks

| Option | Evidence and saving | Risk | Check that keeps it honest |
| --- | --- | --- | --- |
| Changed crates plus reverse dependents | CLI model avoids 98.1% of observed test-seconds; core only 0.7%. Filtered wall savings unmeasured. At most the 1216 s latest Mac execution budget, before required consumer tests. | Omits consumer/fixture/feature tests with no package edge; bad base skips unverified changes. | Shadow full run first; explicit cross-read manifest, planted failures, last verified green base, unknown inputs run all; retain full main/nightly suites. |
| Exact-tree gate reuse/resume | Duplicate gate could avoid up to 2806 s of observed local executed steps. Hit frequency unknown; no benefit on a changed tree. | Treats stale/dirty/partial/environment-mismatched or untrusted receipts as green. | Build on ov-903 prospective provenance; exact base/tree/plan/tools/inputs, complete required rows, immutable originals, independently checked receipts; missing evidence reruns. |
| Preserve verified fold artifacts | 152.61 s cold versus 0.40 s no-op warm on default build; existing handoff already aims at this. Larger test/release savings not measured locally. | Stale source freshness, incompatible native/profile/feature artifacts, disk growth. | Verify ov-1084 hits/misses and source mtimes/content across trees; toolchain/flags keys, bounded retention and uncached control builds. Do not claim the whole warm/cold difference on every edit. |
| sccache across separate targets | Cacheable Rust dependencies offer reuse across worktrees; actual hit rate/savings unmeasured. Largest 46.59 s native script is not automatically cached by RUSTC_WRAPPER. | Untrusted cache writers; proc-macro hidden file/native inputs; disabled incremental can slow edits. | Private trusted writers, hit/miss stats, bounded cache, uncached output/test comparisons, native input mutation; compare preserving workspace incremental versus cacheable deps separately. |
| One shared mutable target directory | ov-837 recorded 28.3 GB for two lanes versus about 30 GB separate: small disk saving. It also reproduced a cross-tree E0063 freshness failure. | Cargo mtime freshness gives one tree another tree's outputs; incremental thrash and shared lock waits. | Do not deploy this design; any renewed experiment must reproduce and prevent the earlier A/B failure with different mtimes/features, not merely observe Cargo's lock. |
| Nextest locally | Already installed/configured. ov-540's repository measurements: six threads 180–207 s versus ten at 150–170 s, but 3/5 ten-thread runs flaked. CONTRIBUTING reports cargo-test serial binaries 1.5–2x slower. These are older evidence, not today's controlled result. | Extra tmux/file-event contention; process-local mutexes stop excluding other tests; no doctests. | Keep machine lock and nextest resource groups; do not raise concurrency for this spike. Run doctests separately, verify skip counts and no-fail-fast coverage. |
| Linker / codegen / profile tuning | This cold build's final CLI compiler invocation totals 4.64 s; linker-only saving is less. Existing ov-837 measurement: no debug info cut 212→132 s and 12.6→7 GB for daemon+client; already applied. Disabling incremental made edits 2.4x slower there. | Breaks native/platform linking or worsens incremental/runtime speed; dependency opt-level changes invalidate caches. | A/B cold, edit and suite wall time at low load on each supported target; preserve release ThinLTO separately; no linker installation justified by current evidence. |
| Split/partition slow tests | Mac suite execution 498 s, largest individual report 103 s; no measured split saving. Rust largest 18 s and main tmux stress already split. | Drops parameter cases, hides timeout races, multiplies global AppKit/tmux state and setup. | Isolate first; balanced process shards with union inventory and per-shard floors/leak checks; keep resource groups and real-time end-to-end timeout coverage. |

The cache limitations above come from upstream [sccache Rust support](https://raw.githubusercontent.com/mozilla/sccache/main/docs/Rust.md):
incremental compilation conflicts with caching; linker-invoking units are not
cacheable; hidden procedural-macro filesystem/native dependencies can escape
keys. See also Cargo's [build-cache documentation](https://doc.rust-lang.org/cargo/reference/build-cache.html).
These are reasons for a bounded experiment, not a projected percentage saving.

## Follow-up order and verification

Follow-up ticket IDs are recorded in [followups.json](followups.json). The order
ranks plausible time saved per implementation effort, conditional on frequency;
it is **not** a measured return-on-investment ranking because duplicate-tree and
leaf-change frequency have not been counted:

1. **ov-1116 — shadow changed-crate selection**, 1–2 engineering days. Largest leaf-change
   execution opportunity; no gate skipping until counterexamples are covered.
2. **ov-1117 — prospective exact-tree engine integration with ov-903**, 2–3 days. Can avoid
   an entire repeated gate; ov-903 source work already exists, operational
   integration and independently checked real input declarations remain.
3. **ov-1118 — compiler-cache/fold-handoff experiment**, 0.5–1 day. Small, measurable default
   build opportunity and possible larger test/release reuse; do not duplicate
   already-implemented target retention.
4. **ov-1119 — isolated Mac UI contention and safe partitioning experiment**, 0.5–1 day to
   establish whether the apparent slow tests are useful split targets.

A nextest install ticket and blanket linker/profile change are not recommended.
Use full required coverage as the control for selector/cache experiments.

For this research-only change, both workspace builds passed. No product code or
CI job was changed. The optional local full test attempt was interrupted and is
explicitly excluded. Validation recomputes the evidence tables, verifies raw
slow-test observations, checks saved CI identities and monotonic durations, and
checks cold/warm/load receipts. Run `python3 reports/ov-1110/validate.py`; results are in
[validation.log](validation.log). `analyze.py` fails if expected job families,
unit inventory, ranked test rows or the historical step total disappear.

## Fresh baseline preflight, 12 November 2026

Re-read the live ticket after research commit `78d559cd5`. Acceptance items 2–4
are marked met; item 1 explicitly remains open for a fresh controlled local
full-gate/test baseline. The historical receipt above does not close that gap.

The [fresh gate plan](fresh-gate-plan.txt) was generated with
`/usr/bin/python3 scripts/lane-gate.py --base d986f3083730db7cda3b91caf5e536ba321b7c1e --keep-target --dry-run`.
It selects wire, doc-comments, relay, tunnel, Rust, Swift tests, Mac bundle,
iOS, Android and web; its listed CI-only omissions remain explicit. This is
planning output, not execution or a passing gate receipt. It includes the
runner-generated untracked `.codex/hooks.json` among the changed inputs.

The [fresh preflight samples](fresh-gate-preflight.json) record all three host
load averages every 30 seconds for 150 seconds. Load remained above the ticket's
limit of 20 throughout, so no heavy baseline was started. No unrelated process
was signaled. The host needs a quiet measurement window before item 1 can be
completed; neither historical timings nor a load-rejected preflight establish a
fresh full-suite result.

Re-ran the evidence validator, file-size budget and doc-comment check successfully.
No product, CI, build-profile or test-selection behavior changed. To resume,
use the planned landing-gate command only when the host is below the limit,
retain its full logs and per-job summary, and monitor load throughout execution.
If load crosses 20, interrupt only processes started by this measurement and
exclude the attempt from the baseline, as with the earlier local test attempt.


## Resume preflight, 13 November 2026 (Singapore)

Read the live ticket again at source `a64f8c9b471eecfd859e2ffde8f8c8b8b6523458`. Acceptance items 2–4
remain met; item 1 still requires a fresh controlled local full-gate/test
baseline. The regenerated gate plan selects the same ten job families as the
saved plan; only the changed-file count differs.

The [resume samples](resume-preflight.json) record all three load averages at
30-second intervals for 300 seconds, from
`2026-11-12T16:01:30.355542+00:00` to `2026-11-12T16:06:30.448535+00:00`. Load started at **9.63 / 73.77 / 77.60** and
ended at **2.76 / 29.99 / 56.08** (1/5/15 minutes). Every sample exceeded the limit
of 20. No heavy measurement was started and no unrelated process was signaled.
A fresh full-suite result remains unmeasured.

Extended `validate.py` to check both rejected preflights' decisions, command,
load samples and time order, and to verify the failed/interrupted nextest
attempts remain nonzero. Counterexamples change the preflight decision, make
a rejected sample fall below the limit, or turn the interrupted test exit green;
each must fail validation. This protects the distinction between a blocked
measurement and a passing baseline. The evidence validator, these
counterexamples, file-size budget, doc-comment check and `git diff --check`
passed. No product or CI behavior changed.

## Controlled resume, 13 November 2026, 00:57–01:21 Singapore

Read `farcooler task show ov-1110` using the installed Canary CLI. Its latest
review request leaves acceptance 1 open and permits recording a renewed load
blocker. Initial `uptime` was 4.50 / 5.25 / 9.42, below 20. The
[capture driver](resume-measure.py) took the machine-wide lock and checked all
three averages immediately before each command, waiting in 30-second loops
when necessary. None of these four command preflights needed to wait. The
driver sampled every two seconds throughout execution. It used a previously
nonexistent isolated target, three compile jobs, no dev debug info, the existing
nextest CI resource groups with three test threads, and both leak wrappers.

The [new summary](resume-local/summary.md) includes the fresh longest compile
units, slowest 20 local Rust tests, all 12 failed tests, and completed gate-step
totals by job. Cold build: 146.62 s outer wall, exit 0; warm: 2.00 s outer wall,
exit 0, with Cargo itself reporting 0.31 s. The outer timer polls at two-second
intervals, adding up to two seconds of observation latency; do not interpret
the warm outer time as compilation work. Original Cargo HTML reports are in
[cold-timing.zip](resume-local/cold-timing.zip) and
[warm-timing.zip](resume-local/warm-timing.zip).

The nextest command finished all 6,928 tests at sampled load below 20 in
811.51 s outer wall: Cargo reports 1m47s compilation and nextest 700.841 s
execution. Its result is **6,916 passed, 12 failed, 33 skipped**, exit 100;
it is a completed timing observation, not a passing baseline. Failures include
hook verdict assertions and permission/held-hook integration tests. This spike
does not change product tests or attribute those failures to a proven cause.
Both leak wrappers completed; the temp/preferences wrapper reported no leftovers.

The full planned landing-gate command started at 01:13:10 Singapore. Wire,
doc-comments, relay and tunnel completed; Rust completed workspace/feature
clippy checks before interruption in the Linux musl cross-target cargo check.
At **01:20:55 Singapore**, the monitor sampled **21.372 / 11.699 / 8.653** and
sent SIGINT only to its own newly created process group. It exited 130 after
464.77 s. No unrelated process was signaled. The blocker and exact timestamps
are in [results.json](resume-local/results.json); the load later rose to
35.78 / 16.55 / 10.56. This is an incomplete gate attempt, not full gate latency.
Swift tests, Mac bundle, iOS, Android and web were not reached. Acceptance 1
therefore still needs a quiet full-gate run; the new local top-20 test evidence
and fresh build pair no longer depend on the interrupted earlier nextest run.

[logs.zip](resume-local/logs.zip) preserves the four command logs, including
the interrupted gate's per-step summary and full nextest durations/failures.
[gate-step-logs.zip](resume-local/gate-step-logs.zip) preserves the gate's own
step logs. Reproduce the local summary with `python3 reports/ov-1110/resume-summary.py`.
No product or CI behavior changed.
