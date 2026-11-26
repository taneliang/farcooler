# ov-1478 · Build cleanup on Cargo 1.100

The three cleanup tests failed in CI because Cargo 1.100.0 moved the fingerprints and compiled files that the cleanup scanner uses to prove ownership, while the macOS landing check used Cargo 1.98.1, whose layout the scanner already recognized.

Both CI archive jobs logged Rust 1.100.0 (`0ef1bc386`). The local landing gate receipt `wf1:6c29f748977954e5705a90465d539e3b7aaccdba11ae5dfbbc92aa98344cf907` records exit 0 for `ebc4b7867` and the archived Rust test run passed. The receipt's PATH resolves to Rust 1.98.1 (`48a229cea`) and Cargo 1.98.1 (`797e8a9bc`). The job logs show the same three cleanup failures on Linux and macOS. [Linux archive](https://github.com/taneliang/farcooler/actions/runs/45745920454/job/139661921130) · [macOS archive](https://github.com/taneliang/farcooler/actions/runs/45745920454/job/139661921104) · [Linux tests](https://github.com/taneliang/farcooler/actions/runs/45745920454/job/139671862875) · [macOS tests](https://github.com/taneliang/farcooler/actions/runs/45745920454/job/139671862707)

On macOS, I built the tests' `rlib` and `staticlib` fixture with its build script into separate fresh targets using `cargo +1.98.1 build --offline --lib` and `cargo +1.100.0 build --offline --lib`. Both versions wrote `.cargo-build-lock`. The older Cargo wrote fingerprints under `debug/.fingerprint` and rustc dep-info under `debug/deps`. Cargo 1.100.0 (`3acc79c0c`) wrote them under `debug/build/<package>/<hash>/fingerprint` and `out`. Calling the cleanup planner from `ebc4b7867` removed the 1.98.1 fixture target. It removed only 1,970 bytes from the 1.100.0 fixture and left the lock and both library exports. The patched planner removed the complete disposable fixture target with each version. [Cargo build-cache documentation](https://doc.rust-lang.org/cargo/reference/build-cache.html)

This was a product bug. Cleanup now checks fingerprints, dep-info, and build timestamps in both layouts before releasing artifacts. Cargo locks still require verified build metadata. The adopted-worktree test still checks that replacement archives and personal files survive.

## Verification

- Linux arm64 in the `fcdiag` VM with Cargo 1.100.0. All 41 build-cleanup tests and all 12 builder-resource tests passed, including the three tests from CI.
- macOS arm64 with Cargo 1.100.0. All 42 build-cleanup tests and all 12 builder-resource tests passed, including the three tests from CI. Those three also passed with Cargo's legacy layout enabled.
- Daemon Clippy with warnings denied passed, along with the daemon-layer, shared-core, and file-size lints.

These runs verify the ticket branch. GitHub main CI has yet to run the fix.

## Separate macOS flake

Attempt 2 reached and passed the quiescence refusal assertion, then failed at the assertion that the detached command was still writing. This test exercises a hold and a Perl writer, independently of Cargo cleanup. It passed on the same shard in attempt 1. On macOS, the six quiesce-order tests passed using short Unix socket paths, followed by ten isolated passes of the detached-writer test. I filed **ov-1479** for the flake. [Attempt 2](https://github.com/taneliang/farcooler/actions/runs/45745920454/job/139671862481) · [Attempt 1](https://github.com/taneliang/farcooler/actions/runs/45745920454/job/139666781608)
