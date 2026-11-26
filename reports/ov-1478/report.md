# ov-1478 · Build cleanup on Cargo 1.100

The three cleanup tests failed in CI because Cargo 1.100.0 moved the fingerprints and compiled files that the cleanup scanner uses to prove ownership, while the macOS landing check used Cargo 1.98.1, whose layout the scanner already recognized.

Both CI archive jobs logged Rust 1.100.0 (`0ef1bc386`). The test fixture invokes the Cargo binary recorded when the archive is built. The local landing gate receipt `wf1:6c29f748977954e5705a90465d539e3b7aaccdba11ae5dfbbc92aa98344cf907` records exit 0 for `ebc4b7867` and the archived Rust test run passed. The receipt's PATH resolves to Rust 1.98.1 (`48a229cea`) and Cargo 1.98.1 (`797e8a9bc`). The job logs show the same three cleanup failures on Linux and macOS. [Linux archive](https://github.com/taneliang/farcooler/actions/runs/45745920454/job/139661921130) · [macOS archive](https://github.com/taneliang/farcooler/actions/runs/45745920454/job/139661921104) · [Linux tests](https://github.com/taneliang/farcooler/actions/runs/45745920454/job/139671862875) · [macOS tests](https://github.com/taneliang/farcooler/actions/runs/45745920454/job/139671862707)

On macOS, I built the tests' `rlib` and `staticlib` fixture with its build script into separate fresh targets using `cargo +1.98.1 build --offline --lib` and `cargo +1.100.0 build --offline --lib`. Both versions wrote `.cargo-build-lock`. The older Cargo wrote fingerprints under `debug/.fingerprint` and rustc dep-info under `debug/deps`. Cargo 1.100.0 (`3acc79c0c`) wrote them under `debug/build/<package>/<hash>/fingerprint` and `out`. Calling the cleanup planner from `ebc4b7867` removed the 1.98.1 fixture target. It removed only 1,970 bytes from the 1.100.0 fixture and left the lock and both library exports. The patched planner removed the complete disposable fixture target with each version. [Cargo build-cache documentation](https://doc.rust-lang.org/cargo/reference/build-cache.html)

This was a product bug. Cleanup now checks fingerprints, dep-info, and build timestamps in both layouts before releasing artifacts. Cargo locks still require verified build metadata. The adopted-worktree test still checks that replacement archives and personal files survive.

## Verification

- Linux arm64 in the `fcdiag` VM with Cargo 1.100.0. All 41 build-cleanup tests and all 12 builder-resource tests passed, including the three tests from CI.
- macOS arm64 with Cargo 1.100.0. All 42 build-cleanup tests and all 12 builder-resource tests passed, including the three tests from CI. Those three also passed with Cargo's legacy layout enabled.
- Daemon Clippy with warnings denied passed, along with the daemon-layer, shared-core, and file-size lints.

## Android replacement fixture follow-up

Commit `b74589a66` already contained the Cargo product fix in `83afd84f2`. All three originally failing tests passed in the first macOS 1/2 job on `b74589a66` and in Linux 3/4 on the later main commit `6268f267a`. [macOS test log](https://github.com/taneliang/farcooler/actions/runs/45785462344/job/139798031945) · [Linux test log](https://github.com/taneliang/farcooler/actions/runs/45795667586/job/139842116602)

Linux CI on `b74589a66` failed at Android fixture preservation assertions in the startup cleanup test on attempt 1 and in the adopted-worktree test on attempt 2. The adopted-worktree assertion recurred on main at `1d4ef97b1`. The fixture restored the old mtime and relied on the replacement's ctime advancing past the manifest timestamp, even when consecutive writes could share a clock tick. The replacement was the only personal file with both the manifest entry and the extension required for cleanup. The fixture now waits for the ctime to advance while preserving the old mtime. [Linux attempt 1](https://github.com/taneliang/farcooler/actions/runs/45785462344/job/139792973098) · [Linux attempt 2](https://github.com/taneliang/farcooler/actions/runs/45785462344/job/139805720378) · [Later main failure](https://github.com/taneliang/farcooler/actions/runs/45803209931/job/139853887004)

- Linux arm64 in the `fcdiag` VM on Cargo 1.100.0 after the fixture change. All 46 build-cleanup tests and 12 builder-resource tests passed, including the three named tests.
- macOS arm64 on Cargo 1.100.0 after the fixture change. All 47 build-cleanup tests and 12 builder-resource tests passed, including the three named tests. The six quiesce-order tests also passed with the matching `farcoolerd` binary and a short directory for Unix sockets.
- Daemon test targets passed Clippy with warnings denied. The daemon-layer, shared-core, and file-size lints passed on the rebased tree.

## Verification on the final base

The branch was rebased onto `c30ad999c`. That base already includes the product fix in `83afd84f2` and the identical Android fixture patch in `555682ae6`.

CI run `45830019686` succeeded on `c30ad999c` on attempt 2. The Linux 3/4 and macOS 1/2 logs both record a pass for each of these tests. [CI run](https://github.com/taneliang/farcooler/actions/runs/45830019686) · [Linux 3/4](https://github.com/taneliang/farcooler/actions/runs/45830019686/job/139963714897) · [macOS 1/2](https://github.com/taneliang/farcooler/actions/runs/45830019686/job/139971122144)

- `startup_reclaims_unrecorded_private_output_and_keeps_live_group_output`
- `waiting_adopted_worktree_releases_build_output_and_reclaims_older_output_on_startup`
- `waiting_adopted_worktree_checks_source_readers_and_external_processes`

A fresh macOS arm64 run with Cargo 1.100.0 passed all 47 tests matching `build_cleanup` and all 12 matching `builder_resources::tests`. Both runs used `cargo +1.100.0 test --locked --offline -p farcooler-daemon --lib` with the respective filter. The daemon-layer, shared-core, and file-size lints also passed.

The fresh `fcdiag` build on this base ran out of memory while compiling the daemon test binary. The Linux CI log above supplies the Linux verification for this base.

## Separate macOS flake

Attempt 2 reached and passed the quiescence refusal assertion, then failed at the assertion that the detached command was still writing. This test exercises a hold and a Perl writer, independently of Cargo cleanup. It passed on the same shard in attempt 1. On macOS, the six quiesce-order tests passed using short Unix socket paths, followed by ten isolated passes of the detached-writer test. The Far Cooler board tracks the flake as **ov-1479**. [Attempt 2](https://github.com/taneliang/farcooler/actions/runs/45745920454/job/139671862481) · [Attempt 1](https://github.com/taneliang/farcooler/actions/runs/45745920454/job/139666781608)

The detached-writer test also passed on `c30ad999c` in macOS 2/2. [macOS 2/2](https://github.com/taneliang/farcooler/actions/runs/45830019686/job/139971122185)
