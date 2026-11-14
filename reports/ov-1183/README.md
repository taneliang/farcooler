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
There is no second architecture in this CI build. Each of the three serial
native invocations logs compilation of the daemon package with its own feature
graph; this is not a universal/two-architecture build.

The test job's Mac step took 964 seconds, including 284.39 seconds of Swift
compilation and 517.958 seconds of CeremonyTests execution. AgentKit took
165 seconds, including 123.59 seconds compiling and 21.914 seconds executing.

## Candidate

- CI's bundle job uses the existing debug Swift configuration; shipping
  workflows still call build-app.sh with its default release configuration.
- Cache Swift compilation separately by complete Xcode/Swift/SDK identity,
  architecture, job and package manifests, with a per-commit key and fallback.
  Restore an input timestamp only when its SHA-256 matches; delete executable
  products before the build so current Rust libraries always relink. Refresh
  Git's stat cache after restoring source timestamps, before Cargo records its
  watched index; otherwise the protocol build script's own git status dirties
  that input and causes a second native build. Keep the staged C header's source
  timestamp (`cp -p`); its bytes are unchanged.
- Use Rust opt-level 0 without cross-crate ThinLTO in the CI bundle job.
  Build the test job's two FFI libraries and real CLI/daemon/filter together
  with `build-test-native.sh`: one dev graph and no symbols, retaining debug
  assertions, overflow checks and test-only hooks. FFI/support code is opt-level
  1; CLI/daemon retain their original dev opt-level 0, avoiding costly LLVM work
  on binaries used for functional integration checks. The Mac
  wrapper revalidates the exact same target/feature graph. This avoids a
  release-library build followed by a separate debug-binary build. Every
  source and test remains; no additional macOS runner is requested.
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
- Fix the hover-delay test's impossible wait: after setting a one-hour delay,
  it polled 200 times for the pending wait to finish. First-candidate CI spent
  122.755 seconds there. Yield and briefly suspend once instead; all assertions
  about delayed visibility, pending state, cancellation and zero-delay display
  remain. The zero-delay branch still waits for actual completion.

## Local validation

- Mac test wrapper integration checks passed, including new parallel mode,
  build ordering, build failure propagation and filter refusal.
- Partition integration self-test passed; a deliberately omitted test fails.
- lane-gate self-test passed, including every full test command remaining wired.
- Updated wrapper checks fail against the original sequential-only runner.
- Unified native helper checks passed: complete library/binary target union,
  replacement of stale libraries, identical staged header bytes/timestamp,
  and build failure propagation. Default local wrapper behavior is preserved.
  A real, tiny offline Cargo workspace also verified the mixed library/binary
  target selection, debug assertions and overflow checks at dev opt-level 1.
  Independent review found no actionable correctness or coverage issue.
- Local gate environment parsing now matches GitHub for quoted scalars and
  comments. Regression checks pin both CI flags, the shared native preparation,
  and pre-Cargo index refresh ordering; the full lane-gate self-test passes.
- Completion guard, temp/defaults leak guard, change selection, file-size and
  doc-comment checks passed. actionlint has no diagnostics beyond the baseline's
  unknown xcode-27 label and SC2086 warnings in unrelated existing install steps.
- Cache self-test passed for unchanged/changed sources and both SwiftPM layouts.
  A real SwiftPM fixture under this worktree's target/ reused unchanged object
  mtimes, relinked a changed native archive (answer 1 to 2), and rebuilt changed
  Swift content even with its timestamp preserved (answer 2 to 3).

## Branch measurements

The initial no-ThinLTO candidate (ca144e87e),
[run 43573737154](https://github.com/taneliang/farcooler/actions/runs/43573737154),
missed the target. Its bundle job passed in 1,452 seconds: libraries 242 seconds,
daemon 378 seconds, CLI 481 seconds and LFS filter 119 seconds. These costs show
that CI also needs to avoid opt-level 3's per-crate optimization, not only LTO.
The profile change invalidated the existing dependency cache (cold Rust build).
Its test job was superseded by the revised candidate after all 2,113 AgentKit
and 2,035 Mac definitions passed their completion guards; the outer Mac leak
check was cancelled, so this is not counted as a successful test job. Mac
Swift compilation took 327.28 seconds. The two-process test region took
232.215 seconds (30 and 2,005 definitions), versus the baseline's 517.958-second
CeremonyTests region plus its other four bundles. Both shards met their exact
inventory counts and five-minute load criterion. This comparison motivates the
partition but does not substitute for two successful final job measurements.

The cache-population candidate before native profile unification,
[43576738289](https://github.com/taneliang/farcooler/actions/runs/43576738289)
(4c109287f), passed both Swift jobs: bundle **919 s (15m19s)**, tests
**1,622 s (27m02s)**. This is not an under-15-minute acceptance run. The app
step took 814 s; native libraries for the tests 257 s, AgentKit 294 s and
Mac build/tests 855 s. Both compilation and native dependency caches were
populated successfully. This records the cold-build cost of that candidate; it is not a cold-cache
15-minute guarantee. The final target is verified on cached branch pushes.

The first cached candidate,
[43579418253](https://github.com/taneliang/farcooler/actions/runs/43579418253)
(fbbc96efd), passed the app job in **574 s (9m34s)** but missed the test target
at **950 s (15m50s)**. Its Swift compilation was only 19.86 s (AgentKit) and
28.63 s (Mac), proving compilation reuse worked. Native preparation still cost
130 s up front and about 236 s again in the Mac wrapper. This motivated the
single dev-profile native target graph, rather than reducing tests.

The first unified trial,
[43582274241](https://github.com/taneliang/farcooler/actions/runs/43582274241)
(b08dc4964), passed app **702 s (11m42s)** and tests **1,185 s (19m45s)**.
Native preparation was 322 s and the supposedly no-op wrapper rebuilt for
278 s. A tiny real Cargo reproduction identified the watched `.git/index`:
restoring source timestamps caused git status inside the protocol build script
to refresh the index *during* the build. Cargo then correctly rebuilt on the
next invocation. Pre-Cargo git status makes the second invocation fresh without
removing or weakening any fingerprint input. The fixture reproduces the old
compile/compile/fresh sequence and verifies compile/fresh after the fix.

The index-refresh candidate,
[43584925203](https://github.com/taneliang/farcooler/actions/runs/43584925203)
(e26bdc4f8), passed both jobs but narrowly missed: app **919 s (15m19s)**,
tests **925 s (15m25s)**. The Mac wrapper now revalidated native targets without
recompiling; its complete build/test step took 310 s. Native preparation took
271 s (including dependencies newly cached after profile unification), and
AgentKit took 99 s. The bundle's four native invocations still took 85, 232,
254 and 121 s. The next candidate disables LLVM optimization only in the CI
bundle job; shipping's release profile, tunnel linkage and bundle checks remain
unchanged. The guard self-test now uses a checked caffeinate stand-in, avoiding
47 s of real display wake/hold calls while preserving the guard's assertions
and checking the wake/hold wiring itself.

The revised CI-only optimization candidate (57d2e55ae),
[43588280875](https://github.com/taneliang/farcooler/actions/runs/43588280875),
passed both jobs under 15 minutes: app **454 s (7m34s)** and tests
**594 s (9m54s)**. The app build step took 331 s (including populating the
new opt-level-0 native dependency cache); native test preparation took 130 s,
AgentKit 48 s and the guard self-test 29 s. Complete job/step evidence is in
`run-43588280875.json`. This is the first qualifying branch run.

The second branch measurement is pending. A temporary push trigger for this branch
collects ordinary CI job measurements without opening a PR or adding runner
jobs. Remove that trigger once two successful under-15-minute runs have been
recorded.
