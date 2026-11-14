# ov-1183: Swift CI under 15 minutes

## Fix round 1: restore required release coverage

Review correctly rejected the debug bundle measurements below: they do not
prove the required per-push release check. CI now calls `build-app.sh` with
its default release configuration and no Rust optimization overrides. Swift
whole-module optimization, release-only conditional code, Rust opt-level 3
and ThinLTO, tunnel linkage and every existing bundle check are restored.

The first release retry only aligned the daemon's forwarding feature. A full
`cargo tree` comparison showed additional crypto feature differences between
separate CLI and daemon invocations, so that is insufficient to share their
optimized dependencies. The revised bundle builds all three binaries in one
release Cargo invocation. Package build scripts add the absolute static archive path
and Security.framework only to the CLI and daemon final links.
The LFS filter and all rlibs receive no tunnel linker arguments. The FFI
libraries retain their separate original feature graph. No global RUSTFLAGS
or optimization override is used. The same optimized commands run in CI and
shipping; Canary/Release workflows and Cargo profiles remain unchanged.

A real offline fixture uses the production build scripts, opt-level 3 and
ThinLTO. It checks one daemon library compilation, executes both tunnel-linked
binaries, inspects symbols to prove the filter contains no tunnel code, and
checks the library contains no archive object. A repeated build is fresh;
changing the native archive relinks both binaries. Plain builds without the
bundle environment keep their original linkage. Artifacts stay under this
worktree's `target/`. The fixture also runs in the required app CI job.

The first restored-release retry, [43592678311](https://github.com/taneliang/farcooler/actions/runs/43592678311)
(1b78f32e3), passed the full test job in **738 s (12m18s)**. Its app build
was still running beyond 15 minutes and was superseded at **966 s (16m06s)**;
it is a missed target, not a successful app measurement. This confirmed that
aligning only the daemon's forwarding feature was insufficient. The release
libraries took 258 s, the daemon invocation 520 s, and the CLI had only just
started when the job was superseded. The revised single-graph candidate is
measured separately below.

The initial single-graph run [43593998335](https://github.com/taneliang/farcooler/actions/runs/43593998335)
stopped in the new fixture before building the app: CI's forced Cargo colors
broke its compiler-command counter. This reproduced locally with
`CARGO_TERM_COLOR=always`; the fixture now passes `--color never` explicitly.
That failed run supplies no app timing or acceptance evidence. Run
[43594410661](https://github.com/taneliang/farcooler/actions/runs/43594410661)
then exposed a second fixture assumption: Rust 1.100 changed the library
artifact naming relative to the local default Rust 1.98. The failure reproduces
with the installed 1.100 toolchain. The fixture now reads `compiler-artifact`
filenames from Cargo's JSON output instead of guessing filenames. Both binary
execution and symbol isolation had already passed on that runner; the app build
was never started. This run also supplies no acceptance timing.

The first complete single-graph release run,
[43594886875](https://github.com/taneliang/farcooler/actions/runs/43594886875)
(980a4c290), passed app **1,435 s (23m55s)** and tests **660 s (11m00s)**.
This is a cold release cache population run, not an under-15-minute result.
The release FFI build took 241 s and the unified native build 939 s while
Swift release compilation ran alongside it. The CLI compilation started at
12:37:45 UTC and the native build finished at 12:49:43 UTC. All bundle checks,
including both real tunnel checks, passed. The release Swift and native
caches were saved successfully. Tests completed all 2,113 AgentKit and 2,035
Mac definitions. Warm release-shaped branch measurements follow.

The first qualifying warm release run,
[43596969322](https://github.com/taneliang/farcooler/actions/runs/43596969322)
(f83d9adca), passed app **687 s (11m27s)** and tests **644 s (10m44s)**.
Release FFI preparation took 98 s and the unified native build 382 s. Every
bundle check passed, and completion guards confirmed 2,113 AgentKit plus all
2,035 Mac definitions (2,005 + 30). Job durations include setup and post steps
and exclude queue time, matching the baseline. The report-only repeat is
pending; production code and optimization settings remain identical. The prior two debug
runs are retained below for history and no longer count toward acceptance.


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

## Original candidate (before fix round 1)

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

The repeat (67162e0a3),
[43589173606](https://github.com/taneliang/farcooler/actions/runs/43589173606),
also passed both jobs: app **328 s (5m28s)** and tests **663 s (11m03s)**.
The app build step took 210 s, native test preparation 167 s, AgentKit 62 s
and the complete Mac build/test step 266 s. Complete job/step evidence is in
`run-43589173606.json`. These are two distinct branch pushes of the same code;
the second adds only the first run's report. Durations include setup and post
steps and exclude queue time, matching the baseline. The temporary branch push
trigger has now been removed.

## Superseded debug-build evidence (not acceptance)

| Job | Main baseline | Branch run 43588280875 | Branch run 43589173606 |
| --- | ---: | ---: | ---: |
| App build + bundle checks | 22m22s | 7m34s | 5m28s |
| AgentKit + Mac tests | 22m37s | 9m54s | 11m03s |

- Both jobs succeeded below 15 minutes in both branch runs. The surrounding
  workflow's unrelated jobs may be superseded by later pushes; the recorded
  Swift job verdicts include their successful post steps.
- Main pushes retain the full AgentKit command and the unfiltered Mac command.
  The Mac runner discovers every definition, checks the exact partition union,
  runs every parameter case, and requires each shard's exact completion count.
  Both qualifying runs completed 2,113 AgentKit definitions and all 2,035
  Mac definitions (30 + 2,005), with the existing completion and leak guards.
  Their guard summaries are retained in `test-completion.txt`.
- Canary/Release workflows, `build-app.sh` and Cargo profiles are unchanged
  from this branch's base. Their default release configuration still uses
  opt-level 3 and ThinLTO. Only CI sets the cheaper profile and debug Swift
  configuration; it retains tunnel linkage and all bundle checks. The shared
  header copy still stages identical bytes, preserving metadata for cache reuse.
- Local guard, wrapper, partition, cache and lane-gate self-tests passed.
  File-size checks and diff whitespace checks passed. Actionlint reports only
  the existing xcode-27 label and unrelated SC2086 warnings.
