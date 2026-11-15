# ov-1270: Swift jobs on the PR path

## What changed

- The `swift` job (app build + bundle checks) builds the bundle's Rust with
  `CARGO_PROFILE_RELEASE_LTO=false` and `CARGO_PROFILE_RELEASE_OPT_LEVEL=0`
  (job-level `env`, PR path only). Swift still builds `-c release`.
- Rust cache key renamed `release-bundle` -> `release-bundle-fast`.
- `scripts/macos-bundle-native-test.py` strips `CARGO_PROFILE_RELEASE_*` from its
  fixture build. Without that its `opt-level=3` assertion failed on the new env
  (CI run 43751761408, "Check native bundle linkage isolation", 96 s in).
- `scripts/lane-gate.py --self-test` used to forbid any override on the bundle.
  It now requires LTO `false` and opt-level `0` on the PR bundle, forbids
  `RUSTFLAGS`, and asserts canary.yml, release.yml and nightly.yml contain neither
  `CARGO_PROFILE_RELEASE_LTO` nor `CARGO_PROFILE_RELEASE_OPT_LEVEL`. (Run
  43751761408 "Wire compatibility" failed on this before the update.)

First attempt was opt-level 1: it left the `farcooler-daemon` crate compiling for
about 4 minutes (log lines 753-762 of run 43752262893 attempt 2), so the job was
11.8 min. Opt-level 0 took it to 5.6-7.3 min.

## Wall minutes, real runs (GitHub-hosted `xcode-27`)

Before: three recent green main CI runs (ThinLTO bundle, warm caches).

| Run | commit | Swift app build | Swift tests |
| --- | --- | --- | --- |
| main 43745907131 | 19bf27fd9 | 16 (Build step 848 s) | 19 (native build 483 s) |
| main 43732000446 | e0b125237 | 10 (479 s) | 11 |
| main 43728800641 | d612b7e42 | 10 (477 s) | 9 |

After, on the final commit 908acce9b (opt-level 0), run 43756701004:

| Attempt | Swift app build | Swift tests |
| --- | --- | --- |
| 1 | 5.6 min | 11.2 min, failed: `TranscriptScrollerTests` "Resizing the window mid-session ... knob within 2%" (timing assertion; unrelated, this job's inputs are unchanged) |
| 2 (failed job rerun) | 5.6 (kept from attempt 1) | 12.3 |
| 3 (full rerun) | 7.3 | 11.3 |

Earlier commit (opt-level 1), run 43752262893: app 10.5 / 11.8, tests 13.1 / 9.5.

Result for acceptance 1: **the app build is under 12 minutes in every run, and
under 8 in all three runs of the final commit.** The tests job passed under 12 in
one of two green runs of the final commit (11.3 and 12.3); the opt-level 1 runs
were 13.1 and 9.5. **I did not meet "three runs, each under 12"
for the tests job.** Floor, from attempt 2's steps: checkout and cache restores
about 1.3 min, five script self-tests about 1.5 min (swift-test-guard-test 39 s),
shared native build 148 s, AgentKit 91 s, Mac tests 279 s, post-cache save 20 s.
That is 10.7 min of steps before runner start and checkout, and the same steps
varied by 20-30% between runs on the same cache (native build 93 s - 483 s across
main runs).

## What I did not do, and why

- **Split Mac tests across runners.** `FARCOOLER_MAC_TEST_PROCESSES=2` already
  partitions them into two processes (ov-1182); the test step is 4.6 min of a
  11-12 min job. A second runner would repeat the 150 s native build and about 2
  min of setup to save about 2.3 min of tests: no net gain, and an extra macOS
  runner on every push. Not done.
- **Build once and share between the two jobs.** The bundle needs release-profile
  Rust (`build-app.sh`); the tests need the dev-profile graph
  (`build-test-native.sh`). They share nothing a download could save. Not done.
- **Universal slices.** `build-app.sh` builds the host architecture only; there
  is no `lipo`, `--arch` or universal step on the PR path or in the script, so
  nothing to drop.

## Test counts (acceptance 2)

`swift-test-guard: N tests ran` lines from the job logs:

| Run | AgentKit | Mac (process 1) | Mac (process 2) | Mac total |
| --- | --- | --- | --- | --- |
| main 43745907131 | 2114 | 2007 | 30 | 2037 |
| main 43732000446 | 2114 | 2007 | 30 | 2037 |
| main 43728800641 | 2114 | 2007 | 30 | 2037 |
| PR 43752262893 attempt 1 | 2114 | 2007 | 30 | 2037 |
| PR 43756701004 attempt 3 | 2114 | 2007 | 30 | 2037 |

Identical. The "1970" in the `swift-tests` timeout comment is stale; this change
touches no tests, so the count is unchanged either way.

## Self-checks (acceptance 3)

- `python3 scripts/ios-ui-shards.py --check`: `62 classes, each placed once; smoke 8 tests, 171 s`
- `python3 scripts/lane-gate.py --self-test`: `ok`
- `python3 scripts/macos-bundle-native-test.py` with the PR env set: all `ok`
- `python3 scripts/ci-concurrency-test.py`: `OK`
- `python3 scripts/macos-test-partitions.py --self-test`: `ok`
- CI: Wire compatibility (runs the self-tests) green on 908acce9b.

## Release, canary, nightly

`git diff 19bf27fd9..HEAD -- .github/workflows/{release,canary,nightly}.yml` is
empty. None sets `CARGO_PROFILE_RELEASE_*` (grep, and the lane-gate self-test now
asserts it). `release.yml` and `canary.yml` call `apps/macos/build-app.sh`, which
uses the workspace `[profile.release]` (`lto = "thin"`, `Cargo.toml:83`); the
bundle there is the shipping profile. `nightly.yml` runs Swift tests, not the
bundle. No workflow (including release.yml) has a universal/`lipo`/`--arch` step
today, so there is no universal build here for this change to have removed.
