# ov-1440 · Pause mobile CI and landing checks

The acceptance item explicitly retains Mac builds and test suites. This change follows that item where the ticket's intent paragraph calls for dropping Mac tests.

## Job selection

The scratch diff touched these four tracked files.

- `apps/ios/FarCooler/FarCoolerApp.swift`
- `apps/android/app/src/main/java/com/farcooler/ui/MainActivity.kt`
- `apps/shared/AgentKit/Package.swift`
- `apps/macos/Package.swift`

The change classifier selected Apple, Android, Relay, and non-docs coverage. The CI row applies the workflow conditions for a pull request or a push that requires CI. The landing rows came from running the real wrapper with `--dry-run` against that diff.

| Check | Before | After |
| --- | --- | --- |
| CI | `changes`, `wire`, `tunnel`, `tunnel-helper`, `swift`, `swift-tests`, `relay`, `site`, `ios-ui-plan`, `ios`, `ios-ui`, `android-build` (2), `android-ui` (2), `android` | `changes`, `wire`, `tunnel`, `tunnel-helper`, `swift`, `swift-tests`, `relay`, `site` |
| Landing | `wire`, `doc-comments`, `relay`, `site`, `tunnel`, `swift-tests`, `swift`, `ios`, `android` | `wire`, `doc-comments`, `relay`, `site`, `tunnel`, `swift-tests`, `swift` |

Each paused CI and Nightly job keeps its definition and has a job-level guard. The iOS build guard also covers the watchOS platform download and Watch targets. The comments at the top of each workflow give the restoration instructions. The local switch is `DISABLED_JOBS` in `scripts/lane-gate.py`. The retained Rust workspace checks still cover `farcooler-jni` on their existing targets. Wire covers shared contracts, including generated Kotlin and Swift, and uses stand-ins for the iOS runner script self-tests. Relay still reads its Android Kotlin fixture.

## Mac coverage

- CI `swift` checks native bundle isolation, builds Far Cooler.app, checks the bundled daemon and CLI tunnel linkage, validates the commit version stamp and CLI/daemon version agreement, and runs the icon, Sparkle key, GitHub download and entitlements checks. It keeps the unlock, Swift guard, macOS test partition, leak, capture-stop and duplicate-capture self-tests.
- CI `swift-tests` runs the Mac test-wrapper self-test, then the guarded AgentKit and Mac app suites via `apps/macos/test.sh`. Both Swift jobs retain their build-cache checks, and `swift-tests` retains its inventory and timing artifact.
- CI `rust` retains the macOS platform checks. `rust-macos-build` builds the runtime binaries and nextest archive. `rust-macos-tests` checks partition coverage and runs both native Rust shards.
- The landing gate keeps the Mac app and AgentKit test builds, both guarded Swift suites, native bundle isolation, the static entitlements check, and the Swift runner self-tests. Release bundle assembly and its runtime smoke checks run in CI under the existing landing policy. The local macOS Rust replay keeps platform checks, archive build, selected nextest coverage, its full-suite fallback, and doctests.
- Nightly keeps macOS Rust and the Mac performance jobs. The iOS build, UI shards, captures, budget, and their three yield jobs are paused for both schedule and dispatch.
- `canary.yml` and `release.yml` are byte-for-byte identical to the base. The successful main CI trigger and Mac build, signing, notarization, Sparkle, and publication steps retain their configuration.

## Before and after

| Run | Jobs | Wall time | Peak disk used |
| --- | --- | --- | --- |
| Full CI before | Changes, wire, tunnel and tunnel helper, Rust platform jobs (2), Linux Rust archive and test shards (4), macOS Rust archive and test shards (2), both Swift jobs, iOS plan/build/smoke, Android build and UI matrices and summary, relay, site, web | 29.2 min, successful run [44467578753](https://github.com/taneliang/farcooler/actions/runs/44467578753) | Runner peak absent from the checked-in baseline |
| Full CI after | Planned for a cross-cutting diff with all retained families enabled. `changes`, `wire`, `tunnel`, `tunnel-helper`, `rust` (2), `rust-build`, `rust-tests` (4), `rust-macos-build`, `rust-macos-tests` (2), `swift`, `swift-tests`, `relay`, `site`, `web-tests`, `web-bundles`, `web` | Requires a hosted run | Requires a hosted run |
| Landing before | `wire`, `doc-comments`, `relay`, `tunnel`, `rust`, `swift-tests`, `swift`, `ios`, `android`, `web` | 1,524.62 s total, including 1,453.78 s of worker time, passing historical run | Run peak unrecorded |
| Landing after, first attempt | Started `wire`, `rust` (including `rust-lints`) and `swift-tests`. Fail-fast stopped the remaining selected jobs. | 2,330.76 s total, including 2,027.02 s of worker time | 864.25 GiB used on the shared volume, up 44.23 GiB from the start of the attempt |
| Landing after, one-worker rerun | `rust` and `swift-tests` passed. `swift` stopped in the native bundle fixture. Fail-fast stopped the remaining selected jobs. | 4,778.64 s total, including 2,834.91 s of worker time | 10.05 GiB sampled across the checkout, temporary files, logs and build caches. Shared-volume use peaked at 837.47 GiB. |
| Landing after, remaining checks | `swift`, `wire`, `doc-comments`, `relay`, `site`, `tunnel`, and `web` passed. | 1,163.90 s total, including 1,161.39 s of worker time | Follow-up peak unrecorded |

The historical landing benchmark predates the site check. A separate earlier attempt reached 38.34 GiB across its checkout, scratch and logs before the doctest tail. The one-worker rerun sampled its files every 30 seconds during execution and the shared volume every two seconds. Worker count, cache state and the wait for the host load to fall affect these timings. The ticket's Nov 23 incident saw machine free space drop from 97 GB to 3 GB during a landing check and four concurrent builds. That observation covers all five processes. Hosted CI and end-to-end Mac publication require a remote run. [CI baseline](../ov-1387/main-ci-baseline.md) · [Landing benchmark](../ov-1185/fix-round-1/earlier-green/benchmark.json) · [Passing gate log](../ov-1185/fix-round-1/earlier-green/trial-1/gate.log) · [Earlier disk measurement](../ov-1327/report.md)

## Verification

- The new mobile-selection test failed on the original gate because it selected both `ios` and `android`. It passes with the guards in place. The real wrapper integration test replaces a passing wire command with `exit 42` and verifies that the wrapper exits 1.
- A path-by-path comparison across all 4,940 tracked files found identical selection for every retained landing job. Every retained CI and Nightly job definition matched the base.
- The full landing self-test, policy and evidence tests, evidence failure mutations, workflow lint, CI change and concurrency tests, Nightly dispatch checks, macOS Rust partition checks, and Canary plan and shipping detection self-tests passed.
- Wire compatibility checks passed, including the Canary baseline comparison. The Go tunnel tests passed with cgo enabled and disabled. Relay (841 tests) and Site (345 tests) passed TypeScript checks, tests and Wrangler bundle dry runs. Web passed its Vitest suite (255 tests), Rust-backed Needs tests (56 tests), workflow fixtures and TypeScript checks.
- The first scratch landing attempt passed workspace Clippy, host and Linux cross-target tunnel checks, the Rust tunnel-helper suite, AgentKit (2,117 tests), and both Mac app Swift partitions (2,031 Swift tests together). It stopped when Rust revocation tests exceeded macOS's Unix socket path limit and three wire self-test fixtures timed out waiting for Cargo's shared build lock.
- The one-worker rerun used a shorter path into scratch. It passed 7,518 Rust workspace tests, the doctests, and 4,148 Swift Testing plus 16 XCTest cases. The Mac bundle fixture expected its own checkout-relative Cargo build directory. The run-wide `CARGO_TARGET_DIR` redirected that fixture's output and made the step fail. The fixture and the remaining jobs passed with checkout-relative targets inside scratch. These results span separate invocations.
