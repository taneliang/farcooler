# ov-1315: CI recovery test race

[CI run 43985211994, attempt 1, Rust tests (1/4)](https://github.com/taneliang/farcooler/actions/runs/43985211994/job/133713551594) failed the child test after 0.18 seconds at `workflow_service_over_socket.rs:361:97`: `Option::unwrap()` on `None`. The subsequent `common/mod.rs:242` assertion reports the child exit status 101 instead of 0; it is a consequence of that panic. The ticket reports that the rerun passed.

At that revision line 361 unwraps the item's candidate immediately after the loop sees `phase == "Uncertain"`. Enqueue can start with no candidate (`reducer.rs` initializes it from `initial_candidate`), and an uncertain outcome sets the phase for any effect (`reducer.rs`), not only CI. The runtime can mark an admitted effect uncertain when recovery cannot yet settle it (`workflow_runtime.rs`). Thus an earlier effect can expose `Uncertain` before the candidate is written; that phase alone does not establish that the pending CI invocation has reached the intended recovery point.

The test now waits for the same snapshot to have a candidate and an uncertain pending effect whose outbox row is CI and is itself uncertain. It still checks the CI dispatch state, original process fact, retained policy pin, completion, and single CI effect after reconfiguration and restart.

## Verification

- The targeted socket test passed 50 consecutive runs with `--test-threads=4` and `-j 4` on macOS.
- The first daemon suite run had 3,317 passes, 16 ignored, and three native test failures; two required a `farcooler` CLI binary. After `cargo build --locked -j 4 -p farcooler-cli --bin farcooler`, all 39 native tests passed on rerun.
- A full suite retry with the CLI built had 3,317 passes, 16 ignored, and three failures: the native startup reason assertion and native steering (both passed in the 39-test rerun), and a gate test reporting `process start identity unavailable`. The suite did not reach integration targets because Cargo stopped after the failed library binary.
- All daemon integration test targets passed in a separate `--no-fail-fast` run with four test threads, including the 18 active socket service tests. The gate test that failed under full-suite load also passed independently. These focused passes do not make the full library suite green.
