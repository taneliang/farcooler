# ov-1185 — fix round 2

**Acceptance 1 remains unmet.** There are still no three completed representative
gates with a measured median below 900 seconds. This round fixes benchmark
cancellation and incomplete timing reports, verifies the actual engine entry
point, and retains a real admission failure without claiming a speedup.

## Actual engine entry point

[Engine entry evidence](engine-entry.json) was read from the local runner's
immutable workflow bundle selected by this ticket's definition pin. Its check is
`/bin/sh -c scripts/landing-gate`, with a three-hour command timeout.
The candidate wrapper invokes `lane-gate.py --landing --keep-target`.

The active engine tree had the older wrapper, whose policy line was just
`gate=("$here/lane-gate.py")`. Its serial gate held the shared heavy queue for
over an hour, including release builds and repeated load waits between families.
After it finished, the same engine tree started another serial gate ahead of
this probe. A sibling iOS UI run was also ahead. These were live processes;
the old dead queue ticket was pruned normally.

This corrects the previous report's implication that the published definition
must be rewritten: it already names the right wrapper. Folding this branch
supplies the optimized wrapper from the candidate checkout. No live workflow
definition or another builder's checkout was changed.

A subprocess regression executes that published shell command against the real
wrapper with recording stand-ins at its boundaries. It verifies planning and
execution both receive `--landing --keep-target`, unlock precedes heavy
admission, base/log arguments survive, and exit 7 remains a failed gate.

## Real measurement

Command, with native Python, Android SDK/JDK and the engine's four Cargo jobs:

```sh
python3 scripts/landing-benchmark.py --base origin/main \
  --out /tmp/ov-1185-round-2-final --deadline-seconds 1800
```

The probe used normal unlock, five-minute load admission (limit 20), shared
queue admission, and the existing warm target. No admission or selection
override was set. Artifacts were written outside the checkout.

| Trial | Wrapper seconds | Completed steps | Outcome |
| --- | ---: | ---: | --- |
| 1 | 974.99 | 0 | Interrupted during admission; exit -15 |
| 2 | not run | not run | Collector stopped after interruption |
| 3 | not run | not run | Collector stopped after interruption |

The five-minute load samples ranged from 17.36 to 75.67. After the load wait
cleared, the probe queued behind the new serial engine gate and the iOS run.
Only this benchmark was terminated. The repaired collector reaped its trial,
wrote [benchmark metadata](benchmark.json) and [trial result](trial-1/result.json),
and returned 130 without starting another trial. The [raw log](trial-1/gate.log)
retains admission messages. This is an interrupted observation, not a timeout,
completed gate, or valid three-run median. The collector's one-observation
`median_wall_seconds` field is not an acceptance statistic.

The observed source was stable during the probe; its HEAD and dirty digest
are in the metadata. A subsequent change only fixes missing-trial table cells
and their regression assertions. The execution policy is unchanged.

[Per-step comparison](step-timings.md) retains every captured historical step
and explicitly marks this trial's steps as not reached and trials 2–3 as not
run. There are no new execution timings to compare. Completing the requested
measurement needs a window in which these normally admitted gates can run;
this round did not establish that window or meet the performance target.

## Benchmark fixes and verification

Previously SIGTERM terminated the collector without unwinding its subprocess
cleanup, potentially leaving its trial in admission or holding the build queue.
SIGTERM now follows the interruption cleanup path, preserves failed evidence,
and prevents subsequent trials. The regression launches a real collector and
sleeping process group, terminates the collector, and verifies the child is
gone, evidence remains, acceptance is false, and trial 2 does not exist.

Timing comparisons previously discarded historical steps absent from every
trial. They now retain those steps and keep the three trial columns aligned
even when collection stops early. Both new regressions fail against the prior
collector: [expected failures](regression-red.log).

- [Lane self-tests](lane-self-test.log): pass; 12 landing-policy tests and 9
  benchmark tests. Real Cargo fixtures show an affected planted failure fails
  the selected gate, an unaffected planted failure passes selection, and the
  full CI workspace command catches it. This exercises CI's command locally,
  not a hosted CI run.
- [Heavy-lock tests](heavy-self-test.log), [Rust selector tests](rust-shadow-test.log),
  and [summary tests](summary-test.log): pass.
- File-size budget, shell syntax and diff whitespace checks: pass.

Self-tests used native tools and command-scoped unsigned scratch commits;
heavy-lock tests required native process visibility outside the sandbox.
The actual branch commit retains normal repository signing.
