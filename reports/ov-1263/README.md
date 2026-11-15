# ov-1263: four daemon tests fail on Linux CI

CI run 43699070430 on main at `99b2a020f`, job "Rust tests (3/4)" on ubuntu-24.04. Both pairs were real product defects. Neither was a flake.

## (A) builder_resources: the build cleanup deleted a person's `target/`

**Failing call.** `std::fs::read_to_string(tree.join("target/keep"))` at `builder_resources/tests.rs:207` and `:220` returned NotFound. Nothing failed to exist from the start. The file had been deleted.

**What deleted it.** The durable build cleanup (`build_cleanup.rs`, `release_build_checked`, ov-1203) sweeps the default build folders (`target`, ...) when a run reaches review, join or an end. It skipped engine item trees, symlinks and shared trees, but not *adopted* worktrees. An adopted worktree is a person's own tree (ov-968), and its `target/` held their build output. The older fallback sweep (`release_run_targets`) already skipped every named tree, and the module doc says nothing touches a tree a person works in. The durable path broke that promise.

**Why only Linux.** The cleanup refuses a tree whose path is not canonical (`tree.canonicalize()? != tree`, meant to skip moved or symlinked trees). On macOS, `tempfile::tempdir()` lives under `/var/folders/...`, and `/var` is a symlink to `/private/var`. So on a Mac the adopted test tree was never canonical, the cleanup skipped it as "not an exclusive linked tree", and the test passed by accident. On Linux `/tmp/...` is canonical, so the cleanup went on and ran `remove_dir_all` on the person's `target/`. In production the stored worktree path is canonical on both systems, so the deletion would have happened on macOS too.

**Fix.**
- Product: `release_build_checked` now skips any effect whose tree is adopted, and records why: "worktree is adopted; its output is a person's".
- Test: the `adopted()` fixture canonicalizes its temp dir. The macOS run now reaches the same code as Linux. Before the product fix it failed locally with the CI signature (NotFound at the same assertion). With the fix it passes. The assertions are unchanged.

## (B) answer_wake engine: a regression on tmux older than 3.7, not timing

**Classification: a real defect, deterministic on Linux CI.**

- The ticket's premise was that these tests passed on the two CI runs before. They didn't run there: neither test exists at `71e5fd150` or `adfde91c0`. `engine.rs` and its tests arrived with the engine train (`092ad24ca`, ov-1171), so `99b2a020f` was their first Linux run.
- The failure has a fixed signature. Both tests used the full 300-iteration wait (about 18 s), and the stand-in's log shows no `PASTE` line at all, only mode changes. Nothing was typed, and nothing was typed late either.
- Cause: before it types into an idle agent, `tell_engine` required `tmux display -p '#{bracket_paste_flag}'` to say `1`. CI pins tmux 3.4 (Ubuntu 24.04), which renders that flag as an empty string. `paste_mode.rs` already documents this. So the precondition can never hold there, and the pump waits for ever. The service's own terminals solve this with `paste_mode::Following`, which reads DECSET/DECRST 2004 from the pane's output. Engine panes run on a tmux server of their own and never got a follower. Real users on tmux < 3.7 (Ubuntu 24.04 runners) would never receive an engine message while the agent sat idle.

**Fix (product).**
- `answer_wake/engine.rs`: `bracketed()` uses tmux's flag when tmux reports it. Otherwise it uses the run's own `paste_mode::Following` record. `follow_bracketing(terminal, runtime, pane)` starts that follower, kept per terminal id because pane ids are per server. `forget_run` (renamed from `forget_context`) drops it when the run is unlisted. An unknown mode is still never typed into.
- `coding_native.rs`: the engine calls `follow_bracketing` after it publishes the pane and **before it releases the launch barrier**. The follower therefore sees the agent's first `ESC[?2004h`, with no race. The engine already subscribes to the same fanout before the barrier for output capture.
- The `watch_view` guard test (`this_module_has_no_way_to_type_or_resize` forbids "paste" in the watch path) stays as it is. The follower lives in `answer_wake`, not `watch_view`.
- Tests: after publishing, both tests follow the pane as the engine does. The scratch agent then redraws (working, then idle), which emits 2004h again, as `settle_stand_in` does for the service's stand-ins. Nothing the tests assert was changed or removed.
- `FARCOOLER_TEST_OLD_TMUX=1` now also forces `tell_engine` onto the below-3.7 path. That is how this was reproduced and verified on a Mac with tmux 3.8.

## Proven locally (macOS, tmux 3.8, all in the foreground)

- `cargo nextest run -p farcooler-daemon builder_resources`: 10/10 pass. Before the fix, with the canonical fixture, the 2 target tests failed exactly as on CI.
- `FARCOOLER_TEST_OLD_TMUX=1 cargo nextest run -p farcooler-daemon answer_wake::engine`: before the fix, the 2 target tests failed with CI's exact message (`message never submitted: START <pid>`, about 17 s). After the fix, 4/4 pass.
- `cargo nextest run -p farcooler-daemon answer_wake::engine watch_view coding_native builder_resources build_cleanup` (82 tests):
  - Native tmux path: 82/82 pass.
  - Forced old-tmux path: 81/82. The one failure, `workflow_native_session_lost_without_receipt_leaves_running_in_one_scan`, failed at `coding_native.rs:874`, the parse of the stand-in's `started` pid file. It passed 3/3 when rerun alone. It looks like a read of a half-written file under parallel load, not bracketing.
  - The two `coding_native` delivery tests need `cargo build -p farcooler-cli --bin farcooler` first, which this run did.
- `cargo clippy --workspace --all-targets -- -D warnings`: pass.
- `./scripts/file-size-budget.py`: ok.
- `CI=true ./scripts/lane-gate.py --base origin/main --only wire,doc-comments < /dev/null`: pass. Rerun after the commit, because the doc-comment check reads `origin/main..HEAD`.

## Only Linux CI can prove

- That (A) passes on ubuntu-24.04 with a canonical `/tmp`. The macOS run now uses a canonical path too, so this is expected.
- That (B) passes against the real tmux 3.4. `FARCOOLER_TEST_OLD_TMUX` simulates only the missing flag. The fanout and the DECSET follower are the same code the service's below-3.7 path already runs on that leg of CI.
