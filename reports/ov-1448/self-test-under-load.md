# ov-1448 gate self-tests under CPU load

The Nov 23 wire log (`wire-20649886.log`) shows the benchmark test timing
out while waiting 10 seconds for the collector to exit. The nested benchmark
failure made the normal lane lifetime test receive exit status 1.

The benchmark fixture now publishes its PID atomically and waits for a signal.
The test waits for the trial and the collector's protected wait before sending
`SIGTERM`, then allows up to 120 seconds for the collector to exit. The lane
lifetime test waits for readiness, the stopped runner in the killed case,
owner exit, and fixture reaping. It keeps observing descendants so it can
detect survivors after reparenting.

## Failure checks

All injected changes ran in scratch copies. Each selected test exited 1.

| Test | Injected change | Failure observed |
| --- | --- | --- |
| Benchmark collector termination | Send `SIGKILL` instead of `SIGTERM` to the trial group | Trial evidence recorded `-9`, expected `-15` |
| Normal lane self-test | Return 1 after the nested self-tests finish | Expected exit 0, got 1 |
| Killed lane self-test | Ignore the closed owner pipe in the outer supervisor | Reported live `cargo test` and `guard_fixture` processes |

A scheduling-delay harness also stopped the collector before sending
`SIGTERM` and resumed it 11.5 seconds later. The original test hit
`TimeoutExpired` at 10 seconds. The revised test passed in 11.735 seconds.
The separate `gate-test-fixture-test.py` low-disk regression passed.

## Loaded runs

Both suites passed 20 consecutive rounds on arm64 macOS with 10 logical
CPUs. For each round, 40 Python processes ran tight CPU loops from before
the benchmark suite started until after the lane suite finished. The driver
checked all workers were alive after each suite and sampled the host's
one-minute load at the end of the round. It ran
`python3 scripts/landing-benchmark-test.py -v` followed by
`python3 scripts/lane-gate-lifetime-test.py -v` in each round. Each benchmark
run executed nine tests, and each lifetime run executed the normal and
killed cases.

The validation used native Python 3.14.10 on `PATH` so the nested test
scripts could launch `python3`. Temporary build output went under
`$FARCOOLER_SCRATCH` via `CARGO_TARGET_DIR`. The table starts at round 02
because the benchmark's deadline fixture received its failsafe during the
first round.

| Round | Benchmark suite | Lane lifetime suite | 1-minute load |
| ---: | ---: | ---: | ---: |
| 02 | PASS (1.52 s) | PASS (314.23 s) | 125.71 |
| 03 | PASS (3.54 s) | PASS (320.55 s) | 177.04 |
| 04 | PASS (1.52 s) | PASS (168.82 s) | 127.11 |
| 05 | PASS (1.79 s) | PASS (196.63 s) | 109.35 |
| 06 | PASS (1.13 s) | PASS (167.28 s) | 107.04 |
| 07 | PASS (1.24 s) | PASS (139.81 s) | 87.23 |
| 08 | PASS (1.13 s) | PASS (195.01 s) | 89.33 |
| 09 | PASS (2.56 s) | PASS (204.13 s) | 102.29 |
| 10 | PASS (1.75 s) | PASS (187.77 s) | 73.89 |
| 11 | PASS (1.34 s) | PASS (164.76 s) | 82.13 |
| 12 | PASS (1.24 s) | PASS (174.78 s) | 81.87 |
| 13 | PASS (1.43 s) | PASS (215.25 s) | 84.62 |
| 14 | PASS (1.23 s) | PASS (163.73 s) | 92.54 |
| 15 | PASS (1.16 s) | PASS (161.19 s) | 77.88 |
| 16 | PASS (1.19 s) | PASS (190.95 s) | 106.02 |
| 17 | PASS (1.80 s) | PASS (153.33 s) | 79.65 |
| 18 | PASS (1.25 s) | PASS (154.26 s) | 102.28 |
| 19 | PASS (1.19 s) | PASS (149.26 s) | 269.76 |
| 20 | PASS (1.12 s) | PASS (124.46 s) | 191.28 |
| 21 | PASS (1.23 s) | PASS (118.50 s) | 135.63 |
