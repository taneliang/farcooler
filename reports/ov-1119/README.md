# ov-1119: Mac UI contention and simulator budget

The guarded two-process prototype reduced median test critical path **30.3%**,
from **171.256 s to 119.315 s**, while summed runner time rose **6.2%**.
Three sequential and two parallel full-inventory runs were green. The default
Mac/CI runner stays unchanged; this is an opt-in measured prototype.

## Results

All five controlled runs used the same direct built-bundle driver and exactly
2,035 definitions. Parallel runs split 30 transcript methods from 2,005 others.
Every shard met its exact completion count, and all leak checks were green.
Raw timings, load peaks, case timings and inventory hashes are in
[`results.json`](results.json).

| Configuration | Repetition | Critical path, s | Summed runner time, s |
| --- | --- | ---: | ---: |
| Sequential | 1 | 171.256 | 171.256 |
| Sequential | 2 | 171.088 | 171.088 |
| Sequential | 3 | 171.594 | 171.594 |
| Two processes | 1 | 119.412 | 182.241 |
| Two processes | 2 | 119.217 | 181.653 |
| Median | Sequential | **171.256** | **171.256** |
| Median | Two processes | **119.315** | **181.947** |

Runner time means summed shard wall time, not CPU time. The measured saving
is 51.941 seconds on the critical path, at 10.691 extra runner seconds. The
parallel shards took about 119 and 63 seconds; their safe state boundaries,
rather than equal method counts, determine this conservative partition.
Individual UI method durations remain largely unchanged in the parallel
configuration: the gain comes from overlapping the serialized transcript
tail with the remaining work, not adding up overlapping tests' durations.

| UI method | Median in full sequential suite, s | Its suite alone, s |
| --- | ---: | ---: |
| Tiled pane, all heights/widths | 41.161 | 6.598 |
| Lone pane, all heights/widths | 6.773 | 7.631 |
| Hover delay | 41.514 | 2.568 |
| Button hover/pressed sweep | 37.657 | 9.084 |
| Composer one/two-line sweep | 32.569 | 2.266 |

The hover method also passed literally alone: one selected test, 2.647 s.
The tile and composer methods run serially within their isolated suites;
all size and argument cases remain. The transcript suite alone completed
30 methods in 124.414 seconds.

Across the tiled pane's 58 windows, median measured layout/read work was
1.046 s in the full suite versus 1.245 s isolated. Suspension was 28.144 s
versus 3.637 s, including 2.900 s of requested polling in each. This isolates
scheduling/timer waits from synchronous layout; it does not call all of that
excess pure MainActor wait. Every controlled run recorded all 116 tile windows.

Earlier exploratory full runs used an extra one-minute ceiling and are all
excluded, even where they met the repository's five-minute criterion. The
SwiftPM-serialized pilot is excluded too. Their observations are retained in
`excluded_runs` in the JSON. No work belonging to another task was stopped.
This shared-Mac result supports explicit use of the prototype; a default CI
rollout would need measurements on its runner hardware.

## What is being measured

Measured on the shared Mac on 2026-11-13, with Xcode 27.1 build 27A9275 and
Apple Swift 6.4. The production sources are base `9f6cbfb39`; timed runs use
the same opt-in test-helper instrumentation in both configurations.

The first unmodified Mac run was green: CeremonyTests reported 1,976 tests
in 164.177 seconds; the other four Swift Testing bundles account for another
59 definitions. Swift compilation took 90.60 seconds, separately from test
execution. The Rust library and binary builds preceded that compilation and
are not included in the 164.177-second execution figure.

The tile composition sweeps **every** height from 360 through 560 in steps of
7, at widths 480 and 1,000, for tiled and lone panes: 116 window cases. Its
two tests took 42.514 and 6.797 seconds in that suite run. A hover-delay test
took 43.006 seconds while other UI tests were running. These wall durations
include suspension and the queue back onto MainActor.

A one-second sample of this task's test helper found active transcript
fixture construction, including PNG encoding in `RealSession.Photos`, as
well as AppKit layout and idle run-loop time. The serialized transcript
scroller suite was the 124.344-second tail of NativeSelectionTests, whose
total was 164.176 seconds. This is a candidate for process isolation, not
evidence that 124 seconds can simply be removed.

`FARCOOLER_LAYOUT_TIMING=1` records synchronous layout/read time and elapsed
suspension separately for `Settle.still`. Suspension includes the requested
10 ms polling intervals, timer lateness, and scheduling back onto MainActor;
it is an upper bound on actor contention, not a pure actor-wait measurement.
No clock or deadline is replaced, and no sweep or parameter case is removed.

## Reproduce

Build once, before collecting timings, under the machine queue:

```sh
scripts/heavy -- apps/macos/test.sh
scripts/heavy -- env FARCOOLER_LAYOUT_TIMING=1 \
  scripts/macos-test-partitions.py --processes 1 --repeat 2 --out /tmp/mac-before-1
scripts/heavy -- env FARCOOLER_LAYOUT_TIMING=1 \
  scripts/macos-test-partitions.py --processes 2 --repeat 2 --out /tmp/mac-after-1
scripts/heavy -- env FARCOOLER_LAYOUT_TIMING=1 \
  scripts/macos-test-partitions.py --processes 1 \
  --suite CeremonyTests.NativeAgentInTileTests --out /tmp/mac-tile-alone
```

Use a fresh output directory per invocation. Two repetitions are the default;
each `run-N/` keeps its own inventory, logs, floors and `timing.json`. Compare
those wall times (critical path) and summed runner times.
Every shard uses the repository's existing five-minute load criterion from
`scripts/heavy`: below 20 throughout, with all three load averages sampled
each second and retained. A five-minute average that reaches 20 is rejected as a
valid measurement; it is allowed to finish rather than killing any work.
Builds, discovery, and the outer file-leak grace period are outside the
reported test-command wall time. Shard guard/tmux-check overhead is included.

The two-process prototype isolates only
`CeremonyTests.NativeSelectionTests/TranscriptScrollerTests`. All other
suites stay together, including tests that use the general pasteboard or
shared defaults. The isolated suite uses its own session windows, models and
scratch image cache. AppKit and NativeAgents singletons are process-local.
This conservative partition targets the measured tail rather than balancing
by method counts or summing durations of overlapping UI tests.

`inventory.json` contains every exact discovered method ID, partitioned once.
The selected suite's anchored prefix is checked against the complete exact
inventory. One shard includes it and the other excludes the identical
selector, so a parent cannot also run a child assigned to the other process.
Runtime IDs append source locations, unlike SwiftPM's displayed method list:
an end anchor selects nothing, while a bare class name can select a skipped
capture through its source filename. This matches [Swift Testing's ID
implementation](https://github.com/swiftlang/swift-testing/blob/main/Sources/Testing/Test.ID.swift).
The prefix check refuses ambiguity if discovery changes. Each shard has an exact
completion count across **all five** Swift Testing summaries and its own tmux
leak check. A file/defaults/system-temp leak check surrounds their union so
one shard cannot accuse a sibling of leaking a still-live file. The runner's
stand-in integration tests prove missing methods fail their shard guard.
All tmux/file-event tests stay in the same shard. Rust nextest exclusivity
groups and resource settings are unchanged. Parallelism is capped at two.

Two `swift test --skip-build` commands still serialize on SwiftPM's workspace
lock; the pilot caught this in the log and is excluded from the comparison.
The prototype therefore discovers the built products and active Xcode paths
once, then invokes that toolchain's `swiftpm-testing-helper` directly for each
built bundle, with the platform XCTest framework/library paths SwiftPM supplies.
Both the sequential baseline and parallel configuration use this same driver.

## Simulator contention

Different device leases did not prevent an iPhone, its clone and an iPad
running together. Direct local `ios-ui-tests.sh` callers now enter the same
machine queue as the landing gate **before** project generation or simctl.
Nested gate calls reuse their parent's lock; per-device leases remain.

Before boot, the script counts all booted devices across models and runtimes.
`IOS_UI_BOOT_MAX` defaults to one. A device already booted can be reused,
but a new device waits for budget and times out with a diagnostic. Cloning
requires both an explicit clone allowance and enough boot budget. Only a
successful boot by this run gives it shutdown ownership; pre-existing
devices are never shut down. An unresolved local destination fails instead
of letting xcodebuild choose an unleased device. `CI=true` retains its
separate-runner path.

The budget coordinates this repository's runner. External simctl/Xcode
invocations do not take the machine queue, and this script never stops them.

## Validation

- Mac baseline: green, existing completion and leak checks passed.
- Simulator lease/budget tests: green; running the updated test against the
  original runner fails at the new boot-budget assertion.
- Simulator retry/headless tests: green.
- Swift completion-guard tests: green, including missing bundles and extra
  tests under exact partition counting. The updated bundle-union test fails
  against the original guard, which reads only the final zero-test summary.
- Partition self-test: exact inventory union, nested parent/child filters,
  green stand-in shards, and a deliberately omitted test turning red.
- Full direct-helper runs: three sequential and two parallel green repetitions,
  identical exact inventory union, production-clock and end-to-end cases retained.
- CI lane-selection/wiring self-test: green.
- File/defaults leak-check self-test and leak lint: green.
- File-size budget and `git diff --check`: green.
