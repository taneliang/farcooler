# ov-1185 — fix round 1 evidence

**Acceptance 1 remains unmet.** There is no measured sub-15-minute median
from three representative completed gates on the final implementation.
The old scaled historical replay is not acceptance evidence.

## Real observations

| Observation | Wrapper seconds | Complete green | Meaning |
| --- | ---: | --- | --- |
| Earlier real gate | 1524.62 | yes | Before the final index fixes; not final-code proof |
| Earlier diagnostic | 1427.17 | no | Failed; not acceptance evidence |
| This round's probe | 1403.78 | no | Stopped during admission; no steps executed |

[Per-step historical/real comparison](step-timings.md) retains all reached
steps, results, and missing-step labels. The first two observations were
recovered from earlier attempts on this branch; their original source hashes,
load samples and commands are in [earlier green](earlier-green/benchmark.json)
and [earlier diagnostic](earlier-diagnostic/benchmark.json). They are different
dirty source snapshots at `1dcb25bd7`, not three trials of the final source.
The historical baseline also predates later CI steps, so this table is not an
identical-source speed ratio. The failed diagnostic includes substantial Rust
compile/test time and an Intel-only OpenSSL executable on this ARM host.

The [current probe](blocked-trial/result.json) invoked the real landing wrapper
with normal unlock, load and lock admission, against `origin/main`, using the
existing warm target. Its [raw log](blocked-trial/gate.log) records load waits
followed by two jobs ahead in the heavy queue. The five-minute load ranged
from 16.79 to 41.01.
The holder was a pinned engine fold running plain lane-gate and its full Rust
shadow step. After 1403.78 seconds without reaching a step, this
probe alone was terminated (exit -15); the other builders were left
running. This is an interrupted observation, not a completed gate or a timeout
that can establish acceptance. The collector was stopped before starting a
second trial, so it produced no three-trial benchmark summary.

This environment prevented completing this round's three-run measurement.
The ticket still needs three representative complete observations below the
median target after normal admission. No admission bypass, reduced test
inventory, or scaled replay was substituted for that evidence.

## Cache invalidation fix

Protocol watches the worktree index for its build stamp. Planning and benchmark
identity reads run before workers receive their environment, so suppressing
optional index writes only in workers did not protect the warm target.
Furthermore, this host's `git diff HEAD` still refreshes index stat data with
`GIT_OPTIONAL_LOCKS=0`. A real scratch-repository reproduction confirmed this.

Landing now sets both `GIT_OPTIONAL_LOCKS=0` and command-scoped
`diff.autoRefreshIndex=false` before planning and in worker environments.
The configuration appends to existing caller entries. Benchmark identity
reads use the same suppression. Dirty/staged/untracked content detection and
actual Git writes remain available; plain lane-gate and CI keep their behavior.

The real Git regressions exercise planning's first reads, a timestamp-only
source rewrite, dirty and untracked source identity, and add/commit. They assert
that observation leaves the watched index timestamp unchanged. Removing the
`diff.autoRefreshIndex=false` setting produces the retained
[expected regression failure](index-regression-red.log).

## Validation

- [Lane-gate self-tests](lane-self-test.log): pass, including 11 landing-policy
  tests and 7 benchmark tests. The planted affected crate fails locally; the
  unaffected selected run passes; the full CI workspace command catches its
  planted failure. This exercises real fixture builds, not hosted CI.
- [Rust selector](rust-shadow-test.log), [heavy lock](heavy-self-test.log), and
  [gate summary](summary-test.log) self-tests: pass.
- Source-size budget, shell syntax, and diff whitespace checks: pass.

Native tests used the installed Python 3.14 through a temporary PATH shim and
system utilities ahead of this host's Intel-only `/usr/local/bin` tools. Fixture
commits disabled inherited signing with command-scoped Git configuration.
The final branch commit uses normal repository signing.

Reproduce the outstanding measurement with native toolchains and normal
admission, writing evidence outside the checkout:

```sh
python3 scripts/landing-benchmark.py --base origin/main \
  --out /tmp/landing-measurements --deadline-seconds 7200
```

The engine must invoke `scripts/landing-gate` to use this policy; already pinned
engine definitions invoking plain lane-gate are not rewritten here.
