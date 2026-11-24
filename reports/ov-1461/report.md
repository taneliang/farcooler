# ov-1461 review range after a landing send-back

## Cause and incident

The landing queue can send a ticket back to Build with a specific landing SHA in the rebase brief. It keeps the original Provision and its base pin. Before this fix, the review process read that same pin into `FARCOOLER_BASE`, and the agent's changed-path model selection read it as well. The candidate was correct. The lower bound was stale.

I checked the ov-1410 history locally. Git reports `a07686688` as the common ancestor of the new landing base and `d10f09dfa`. The engine's reported range `1c0c2027a..d10f09dfa` contains 66 commits and changes 495 files. The range `a07686688..d10f09dfa` contains six commits, all for ov-1410, and changes 75 files.

## Change

A fresh review reads the configured landing target and pins its unique Git merge base with the candidate. A further advance on main leaves the replayed ticket's base at the common ancestor. The reviewer command, agent brief, and changed-path model selection use the same pin. The brief asks the reviewer to include the range in a rejection summary, and the agent verdict receipt records the base with the candidate. Recovery reads the original binding for reviews launched before the review pin existed.

If the engine cannot establish the range, the review stops before launch with a step failure. That uses the step's retry budget instead of the ticket's review back-edge budget. Historical rejection receipts have a pass bit and a free-text finding. Git can establish that a historical range included unrelated commits, but neither the receipt nor Git can establish that the range was the sole cause of rejection. The engine therefore applies the existing send-back accounting to recorded verdicts.

## Verification

The new regression, `review_after_a_landing_send_back_only_sees_the_rebased_ticket`, failed against the unpatched engine on `origin/main` (`ac9cd8616`). The fixture advanced main twice, triggered a real queue-conflict send-back, advanced main again, and rebased the builder onto the SHA from the send-back. The second reviewer received the original SHA. Its commit list had two `other landing` commits, and its diff included `independent.txt`. The expected range started at the rebase SHA, listed the ticket's two build commits, and changed only `shared.txt`.

After the fix, 25 daemon tests passed, including the new regression, landing and fold tests, review rejection and recovery, and the updated ticket-brief test. The ticket-brief test verifies that the agent sees the pinned range and that a rejection receipt stores the same SHAs. The rebase regression and ticket-brief tests both passed again with the final prompt wording.

`cargo clippy --locked --offline -p farcooler-daemon --all-targets -- -D warnings` passed. The daemon layer and file-size checks also passed.

The native PTY fixture could not initialize because the scratch path exceeded the macOS Unix socket path limit. Automatic approval review rejected rerunning that fixture with `TMPDIR=/tmp` under the scratch-only constraint.

## Landing-gate follow-up

The landing gate passed the rebase regression and the ticket-brief test, but eight existing tests failed again when rerun individually. All eight used the parallel-review fixture, which initialized an empty bare repository. Review now resolves the configured `refs/heads/main` before launching the reviewer or its parallel check. The empty target explains both the failed review phases and the cancellation test's wait for processes that never started.

The fixture now publishes its initial commit to `refs/heads/main` in that private repository. The original landing-gate log (`rust-55372114.log`) records all eight repeat failures. The live `workflow get` command returned `known caller required`.

`scripts/landing-gate --quick-only`, the daemon layer check, and the file-size check passed on the repaired worktree.

All eight tests that failed at landing passed in the focused rerun, as did the rebase regression and the verdict-brief test. The rerun used two test slots at the runner's inherited nice value of 10 and a Git config with `init.defaultBranch = master`.

The focused run passed 34 of 35 tests. `a_recorded_verdict_settles_a_turn_whose_end_never_arrives` exceeded its four-second assertion by 0.45 seconds and passed when rerun alone. `cargo clippy --locked --offline -p farcooler-daemon --all-targets -- -D warnings` passed before the follow-up commit.

## Parallel check after rebase

`review_check` derives a Run operation from the admitted agent review. The parallel check took the ordinary Run fallback from `process_base`, which gave it the original provisioning SHA even when the reviewer used the pinned merge base. A new launch now records the parent's review base for the child before either process starts. That child receipt also supplies the base when recovery verifies the check's environment. Older check facts without a child pin still verify against their original launch context.

The new `agent_review_and_parallel_check_see_the_same_rebased_ticket` regression failed before the fix at `6ebff79bb`. The real queue rejected the conflicting candidate, returned the landing SHA to the builder, and the builder rebased onto it while main advanced again. The agent saw the rebased SHA, two ticket commits, and only `shared.txt`. The parallel check instead reported the original SHA, two extra `other landing` commits, and `independent.txt` in its diff.

The first unit-only build omitted the daemon executable the native PTY tests use to collect pane output. With the installed Canary executable supplying only that helper, the pre-fix test reached the range assertion and reproduced the mismatch.

A historical check's recorded exit and output can establish that it rejected a candidate, and its process environment can show that it used the old base. Those facts cannot establish that the old base was the sole cause of the rejection. The existing send-back accounting still applies to recorded outcomes.

Both rebase regressions passed after the fix with the daemon helper built from this branch. The two-slot daemon run passed 119 of 120 tests, including parallel review launch, rejection, cancellation, recovery, the verdict brief, and folded landing gates. The legacy holder fixture picked an Intel Python binary on the ARM runner. It passed on a serial rerun with system Python, along with six additional native review recovery tests.

`cargo clippy --locked --offline -p farcooler-daemon --all-targets -- -D warnings`, the daemon layer check, the file-size check, and `git diff --check` passed.
