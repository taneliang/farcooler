# ov-1184 Android CI timings

Baseline: [main run 43557233452](https://github.com/taneliang/farcooler/actions/runs/43557233452), successful Android job, 19m32s (04:33:44–04:53:16 UTC). Rust Android core and tunnel took 2m25s, unit tests 1m49s, debug and instrumented test compilation 57s, release APK 3m00s, and Robolectric 9m21s. These are step wall times from `gh run view --json jobs`.

The proposed split preserves the `Android` required check as an aggregate of both jobs. Build caching was already enabled in `gradle.properties` and `Swatinem/rust-cache` already cached Cargo; the UI job has its own host cache. Configuration caching is enabled for each invocation, and capture tests use two forks. The two jobs run concurrently. The UI tests still run on every push selected by the same Android changes filter, with the existing count and required-class guard.
