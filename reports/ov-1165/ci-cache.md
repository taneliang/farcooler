# CI cache hits and setup (ov-1165)

Source: the 10 most recent successful `ci.yml` runs on main, 43399367057 through 43542692133 (Nov 13 09:12 to Nov 14 01:47 UTC). One of the ten (43410702134) skipped the heavy jobs, so per-job figures cover 9. Step timings from `actions/runs/<id>/jobs`, cache lines from each job's log.

## Cache hit rate before

Every cache restored on every run: 100% (9/9) for rust-cache on the Rust (both OSes), Swift build and Swift tests jobs, for SwiftPM, tmux and setup-go. A hit is not the same as useful, though:

- The rust-cache key (manifest hash plus rustc hash) was identical across all 10 runs, and every run ended `Cache up-to-date`. An exact-key hit never re-saves, so a cache that was saved incomplete stays incomplete.
- **Swift (AgentKit + Mac tests)** recompiled `aws-lc-sys`, `aws-lc-rs` and `russh` in 9/9 runs, despite the hit. The Rust and Swift build jobs compiled 0 third-party crates. That job's `cargo build --release -p farcooler-vt -p farcooler-client` took a median 124 s (max 180 s).
- Third-party crates are otherwise fully cached; the remaining Rust time is the workspace's own crates, recompiled by design on every commit.

## Where the macOS time goes (median, 9 runs)

| Job | Job median | Biggest steps |
| --- | ---: | --- |
| Rust (macos-latest) | 45.3 min (range 25.3 to 56.5) | nextest + shadow 30.0 min; clippy workspace 115 s; clippy daemon features 99 s and 89 s; nextest daemon feature 85 s; cargo check 70 s; CLI put-in-place 80 s; rust-cache restore 24 s |
| Swift (app build) | 22.6 min | Build Far Cooler.app 19.0 min; setup-go 35 s; rust-cache restore 21 s |
| Swift (AgentKit + Mac tests) | 26.8 min | Test the Mac app 19.4 min; Test AgentKit 238 s; Rust client build 124 s; rust-cache 19 s |

Per-run job durations (min), Rust macOS: 45.3, 25.3, 44.8, 56.5, 53.5, 51.6, 29.7, 46.0, 41.4.

Duplicated setup found: each Swift job and the Rust job install the toolchain and restore a separate ~475 to 790 MB rust-cache. These are different build sets (release bundle, release client plus debug tests, dev/test), so merging them would share nothing but dependencies already cached; not changed.

## Change

`ci.yml`: each `Swatinem/rust-cache` now has an explicit `key` naming its build set (profile, feature sets), on top of rust-cache's own rustc-version and manifest hash. A changed key forces a fresh save, which heals the Swift tests cache. Toolchain separation: rust-cache's key includes the rustc hash (`f997da26` above), so a toolchain bump never shares outputs; the explicit key keeps the three jobs' differing feature sets and profiles apart.

## Not done: acceptance 2 (after-timings on two branch runs)

The sandbox denied `git push` and PR creation, so no CI run of this branch exists. To finish: push the branch, and compare "Build the Rust client library for AgentKit's tests" on the second branch run against the 124 s median above (the first run cold-saves). Expected: aws-lc/russh no longer compile, about 1 to 1.5 min saved per Swift tests run. The 30 min nextest step is test execution plus workspace compilation, which no cache can remove.
