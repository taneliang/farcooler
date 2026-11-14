# ov-1184 Android CI timings

Baseline: [main run 43557233452](https://github.com/taneliang/farcooler/actions/runs/43557233452), successful Android job, 19m32s (04:33:44–04:53:16 UTC). Rust Android core and tunnel took 2m25s, unit tests 1m49s, debug and instrumented test compilation 57s, release APK 3m00s, and Robolectric 9m21s. These are step wall times from `gh run view --json jobs`.

The proposed split preserves the `Android` required check as an aggregate of both jobs. Build caching was already enabled in `gradle.properties` and `Swatinem/rust-cache` already cached Cargo; the UI job has its own host cache. Configuration caching is enabled for each invocation, and capture tests use two forks. The two jobs run concurrently. The UI tests still run on every push selected by the same Android changes filter, with the existing count and required-class guard.

First branch run: [43574035137](https://github.com/taneliang/farcooler/actions/runs/43574035137), commit `7ae2a8c`. Both jobs and the aggregate passed. Android UI took **16m28s**, with a cold host build 3m40s and Robolectric 11m58s; Android build and unit tests took **17m10s**. This run misses the under-15-minute target. The baseline's warm-cache timing and this first cold-cache run are not a controlled comparison.

Revised split, first run: [43576086903](https://github.com/taneliang/farcooler/actions/runs/43576086903), commit `9830d3bea`. All Android jobs and the aggregate passed: unit **3m52s**, release **11m14s**, UI A–M **3m57s**, UI remainder **14m48s**. The slowest job is narrowly under 15 minutes. The original initial-split second run was superseded to measure this revision.
