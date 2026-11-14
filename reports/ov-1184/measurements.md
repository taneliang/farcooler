# ov-1184 Android CI timings

Baseline: [main run 43557233452](https://github.com/taneliang/farcooler/actions/runs/43557233452), successful Android job, 19m32s (04:33:44–04:53:16 UTC). Rust Android core and tunnel took 2m25s, unit tests 1m49s, debug and instrumented test compilation 57s, release APK 3m00s, and Robolectric 9m21s. These are step wall times from `gh run view --json jobs`.

Gradle's build cache was already enabled in `gradle.properties`, and `Swatinem/rust-cache` already cached Cargo. Configuration caching is now enabled for each invocation; capture tests use two forks. Unit tests, release and the two capture shards run independently. The existing `Android` required check aggregates their results and fails if any part fails or is unexpectedly skipped. The unchanged Android changes filter still governs all four parts.

The initial split's [branch run 43574035137](https://github.com/taneliang/farcooler/actions/runs/43574035137), commit `7ae2a8c`, passed but missed the target: Android UI took 16m28s and build/unit tests 17m10s. The host build was cold (3m40s). Its 259 UI tests in 39 classes all passed without skips. The baseline's warm-cache timing and this first cold-cache run are not a controlled comparison.

| Revised split | Unit tests | Release build | UI A–M | UI remainder | Aggregate |
| --- | ---: | ---: | ---: | ---: | --- |
| [43576086903](https://github.com/taneliang/farcooler/actions/runs/43576086903), `9830d3bea` | 3m52s | 11m14s | 3m57s | 14m48s | passed |
| [43577348217](https://github.com/taneliang/farcooler/actions/runs/43577348217), `da514a90e` | 3m34s | 7m07s | 3m44s | 14m58s | passed |

Each job passed in under 15 minutes on both runs. These are execution times from each job's start to completion, excluding runner queue time; on the second run jobs started over two minutes apart and the aggregate completed 15m19s after the first started. The remaining UI shard's margin is just two seconds in the second run, so the target is fragile under normal runner variation. Each run's shards executed 68 tests in 12 classes and 191 tests in 27 classes, respectively, with zero failures or skips: the same 259 tests and 39 classes as the unsplit branch run. The original split's second run was superseded when the revised split was pushed.
