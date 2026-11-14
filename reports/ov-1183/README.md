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

- CI's bundle job uses the existing debug Swift configuration; shipping
  workflows still call build-app.sh with its default release configuration.
- Cache Swift compilation separately by complete Xcode/Swift/SDK identity,
  architecture, job and package manifests, with a per-commit key and fallback.
  Restore an input timestamp only when its SHA-256 matches; delete executable
  products before the build so current Rust libraries always relink. Keep the
  staged C header's source timestamp (`cp -p`); its bytes are unchanged.
- Disable cross-crate ThinLTO only in the two CI Swift jobs. All sources,
  release optimizations, linked tunnel features and bundle checks still run.
  Canary/Release workflows, Cargo profiles and build-app.sh are unchanged.
  build-vt.sh's only shipping change is preserving the copied header's metadata;
  the same source header is still copied to the same destination.
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
- Updated wrapper checks fail against the original sequential-only runner.
- Completion guard, temp/defaults leak guard, change selection, file-size and
  doc-comment checks passed. actionlint has no diagnostics beyond the baseline's
  unknown xcode-27 label and SC2086 warnings in unrelated existing install steps.
- Cache self-test passed for unchanged/changed sources and both SwiftPM layouts.
  A real SwiftPM fixture under this worktree's target/ reused unchanged object
  mtimes, relinked a changed native archive (answer 1 to 2), and rebuilt changed
  Swift content even with its timestamp preserved (answer 2 to 3).

## Branch measurements

Pending. A temporary push trigger for this branch collects ordinary CI job
measurements without opening a PR or adding runner jobs. Remove that trigger
once two successful under-15-minute runs have been recorded.
