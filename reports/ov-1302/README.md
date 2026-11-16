# ov-1302: the plan loads with a long history (carries ov-1298)

Build: ov-1298's two commits cherry-picked onto this branch (`git merge --ff-only` was refused; the base had moved).
Run on a Mac, foreground, from the worktree, `PATH=$HOME/.cargo/bin:$PATH`.

## Test runs

| Command | Result | Duration |
|---|---|---|
| `cargo nextest run -p farcooler-daemon` (whole crate) | 3,856 run: 3,854 passed, 2 failed, 19 skipped | 520 s of test time (11 m 39 s wall, including the build) |
| `cargo nextest run -p farcooler-daemon -E 'test(accepted_native_claude_delivers_message_and_acceptance_before_turn_ends) or test(workflow_native_steering_continues_the_same_codex_and_claude_session_before_settlement)'` after `cargo build -p farcooler-cli --bin farcooler` | 2 run: 2 passed | 6.5 s |
| `env -i HOME=$HOME PATH=$PATH cargo nextest run -p farcooler-cli --no-fail-fast` (whole crate) | 705 run: 705 passed, 1 skipped | 5.8 s |
| `./scripts/daemon-layers.py` | ok (648 recorded edges in 55 layer pairs, none new) | 2.8 s |
| `./scripts/file-size-budget.py` | ok, 2,167 files, 27 grandfathered | 0.4 s |
| `./scripts/lane-gate.py --self-test` | OK | under 1 s |

The two daemon failures were environmental, not the change: both tests say "build the hook CLI with
`cargo build -p farcooler-cli`" and the binary had not been built in this worktree. They pass once it is.
The whole daemon crate was not rerun after building the binary (another 9 minutes); only those two were.

The CLI crate fails 8 hook tests (`a_hook_is_never_in_the_way`, `a_hook_names_its_panes_launch`,
`a_native_hook_follows_its_private_socket`) when run from inside a Claude Code session, because
`CLAUDE_CODE_MESSAGING_SOCKET`/`_TOKEN` and the lane's `FARCOOLER_*` variables leak into the hook binary.
With a clean environment (`env -i`) all 705 pass. Nothing in the hook code was touched by this change.

## Workflow reply size, 400 archived runs and 20 live ones, full log tails

Test: `workflow_runtime::process_logs::tests::workflow_get_with_a_long_history_carries_live_tails_only`
(passes; asserts under half of the 1,048,576-byte frame cap, live items only, newest line kept).

- After: **234,431 bytes** on the wire (22% of the cap), 20 tails, all for live items.
- Before: every one of the 420 items carried a tail. At the measured ~11.7 KB per tail that is about
  **4.9 MB**, nearly five times the cap (computed from the per-entry size, not run on the old code).
  On the owner's real board the reply was 1,355,690 bytes against the cap.

## Acceptance

1. Long-history reply under half the cap, live tails only: test passes (above).
2. Archived tail readable on request: `an_archived_items_tail_is_read_on_request` (CLI/library path) and
   `an_archived_items_tail_is_read_over_rpc` pass.
3. task.list paging/trimming with 1,500 tasks: `task_ops::tests::a_long_board_lists_within_the_frame_cap` (1,500 tasks, paged and unpaged),
   inside the whole-crate daemon run above.
4. Whole-crate runs and the three scripts: recorded above.
