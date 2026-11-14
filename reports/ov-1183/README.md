# ov-1183: Swift CI under 15 minutes

## Baseline

[Main run 43557233452](https://github.com/taneliang/farcooler/actions/runs/43557233452).
Job times include setup and post steps, excluding queue time.

| Job | Seconds |
| --- | ---: |
| Swift (app build + bundle checks) | 1342 |
| Swift (AgentKit + Mac tests) | 1357 |

The bundle step took 1,222 seconds. The single host architecture's Rust
library build took 91 seconds, daemon 438 seconds, CLI 497 seconds, LFS
filter 75 seconds. These release builds use ThinLTO; the daemon, CLI and
filter run serially, alongside the Swift build after the libraries are ready.
There is no second architecture in this CI build.

The test job's Mac step took 964 seconds, including 284.39 seconds of Swift
compilation and 517.958 seconds of CeremonyTests execution. AgentKit took
165 seconds, including 123.59 seconds compiling and 21.914 seconds executing.

## Candidate

- Disable cross-crate ThinLTO only in the two CI Swift jobs. All sources,
  release optimizations, linked tunnel features and bundle checks still run.
  Canary/Release workflows, Cargo profiles and build-app.sh are unchanged.
- Run the previously measured ov-1119 two-process partition on the same runner,
  after building all test bundles once. Discover every test, preserve complete
  suites, validate the CeremonyTests checked-in floor and the exact partition
  union, then require every shard's exact completion count. Retain process,
  temp-file and defaults leak guards. No extra macOS runner is requested.
- Reuse the deterministic 2000-by-1500 noisy PNG bytes once per process rather
  than regenerate/encode them on every scroll session's MainActor setup.
  Each source still delays image delivery 400 ms; dimensions, bytes, session
  rows, parameter cases and production timing checks are preserved.

## Local validation

- Mac test wrapper integration checks passed, including new parallel mode,
  build ordering, build failure propagation and filter refusal.
- Partition integration self-test passed; a deliberately omitted test fails.
- lane-gate self-test passed, including every full test command remaining wired.

## Branch measurements

Pending. A temporary push trigger for this branch collects ordinary CI job
measurements without opening a PR or adding runner jobs. Remove that trigger
once two successful under-15-minute runs have been recorded.
