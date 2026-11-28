# ov-1526: engine concurrency tests on macOS CI

Main went red at a2926b900 (run 46113901247) with three failures. Two of them had the same cause, a race between
concurrent `git worktree add` calls in one repository. That race is now fixed in the engine. The third is a separate,
newer flake in a 500 ms `ps` deadline.

## The two concurrency tests: a `git worktree add` race

- `workflow_execution_phases_follow_claims_and_slots` panicked at `concurrency_tests.rs:98:51` with
  `Refused("configured worktree provisioning uncertain")`.
- `workflow_a_provision_starts_while_four_builds_hold_every_agent_place` panicked at `concurrency_tests.rs:218:5`.
  The state it printed shows one item `phase: Uncertain` at stage 0, meaning its Provision failed the same way.

The refusal is raised at `crates/daemon/src/workflow_runtime/coding_execute.rs:637`. That happens when
`crate::git::create_worktree_with` fails and `named_tree::unmade` (`named_tree.rs:381`) then finds something left
behind. I reproduced it locally with a temporary log subscriber: 3 failures in 300 runs, 8 runs at a time, at load
average 17–24. Git's own stderr from each failure:

```
Preparing worktree (new branch 'wf-7297…')
fatal: failed to read .git/worktrees/tree-wf1-3844…/commondir: Undefined error: 0
```

and then `the run's worktree is left uncertain … there=the branch wf-7297… exists`.

The `commondir` that git fails to read belongs to a sibling provision's worktree. Here is the sequence:

1. `worktree add -b` creates its branch.
2. It calls `die_if_checked_out`, which opens every worktree's ref store. For each one, `get_common_dir_noenv` reads
   `.git/worktrees/<id>/commondir`, and it dies if the file exists but reads as empty. errno is 0 in that case, which
   is where "Undefined error: 0" comes from.
3. A sibling `worktree add` creates that file a moment before it writes `../..` into it. A reader that lands in that
   gap dies.
4. The branch from step 1 already exists, so the daemon cannot tell this apart from a half-made tree, and it calls
   the Provision uncertain.

The gap is microseconds. Plain `git worktree add` (800 adds, 8 at a time) never hit it. The daemon's guarded,
sandboxed adds, four per sweep, hit it about 1% of the time.

**The seed step did not change slot or claim behavior, on macOS or elsewhere.** These tests build their config with
`dependency_seed: None` (`coding_tests.rs:126`). Both seed entry points return before doing anything in that case:
`refresh_seed` at `dependency_seed.rs:68` and `seed_scratch` at `dependency_seed.rs:221`. A scan of main's CI
(Nov 14–28: 108 runs, 51 failed macOS shard jobs) shows the same "provisioning uncertain" failure at 2f0b5f026
(Nov 18) and "four builds are next" at fef14a07a (Nov 26), both before the seed work. Every occurrence was on macOS;
no Linux shard failed. So this is not load flakiness in the test. It is a real engine race, and load makes it more
likely to hit.

One related finding: the seed does run its own `worktree add --detach` in the same repository
(`dependency_seed.rs:112`), as does the landing preflight (`waiting_join.rs:236`). In production either one could
race a provision this way. The fix below covers both.

### Fix

`crates/daemon/src/git/worktree_metadata.rs` adds a process-wide `RwLock`, which `run_bounded` takes for each git it
runs:

- `worktree add` takes it exclusively, because it writes the metadata.
- `worktree list`, `branch`, and a `for-each-ref` whose format prints `%(worktreepath)` share it, because they read
  every worktree's metadata. That last one is `git::list_branches` (`git.rs:587`), which
  `create_worktree_from_branch` (`git.rs:679`) calls, so a person making a worktree from a remote-only branch while a
  workflow provisions could hit the same race. Review caught it; git 2.54 dies on an empty commondir there too.
- Every other git runs without it, so a long fetch or a slow `worktree remove` never holds provisions back. A `fetch`
  into `refs/heads` also reads every worktree, but the daemon's fetches write only `refs/farcooler/*`.

The lock is taken before the git's own timeout starts. The fill (`reset --hard`) stays parallel, which keeps the
slow part of provisioning concurrent.

Limits: the lock only orders this daemon's gits. A person's own git can still race an add. `worktree remove`,
`checkout` and `switch` are left out of the lock on purpose.

### Evidence

On the fix, on macOS, 100 runs each, 8 at a time, while the load average climbed from 17 to 38:

- `workflow_execution_phases_follow_claims_and_slots`: **100/100 passed**. Before the fix: 99/100 and 100/100.
- `workflow_a_provision_starts_while_four_builds_hold_every_agent_place`: **100/100 passed**. Before the fix: 98/100,
  and both failures were the commondir race above.

I ran these loops before one last edit, which moved the lock's own unit test onto a private lock so tests running
beside it can't hold the global one. That edit changed test code only.

Other runs:

- The new unit tests pass: the classifier, and "a list waits while an add holds the lock".
- `git::` (25 tests) and `reconcile::` (19 tests) pass.
- Under nextest, `workflow_runtime::` passes except one test that needs more local setup.
  `coding_native::quiesce_order_tests::a_killed_hold_refuses_settlement_while_a_detached_command_writes` fails here
  with the lock removed too (an A/B rebuild), so the failure is in this machine's setup, not this change.

## The compose test: a separate flake

`watch::answer_wake::tests::compose_tests::a_draft_or_a_dialog_is_refused_untyped` panicked at `compose_tests.rs:230:5`
with `left: "process_unreadable"` and `right: "draft"`.

The cause is `pane_ps` in `crates/daemon/src/watch/answer_wake.rs`. It gives `ps` 500 ms (`PROCESS_READ_DEADLINE`,
line 1161) and refuses with `process_unreadable` when that runs out. On a loaded macOS runner, `ps` took longer.

This deadline came in with ov-1430 (6f07655ad, Nov 24). The CI scan found this test failing once. Since Nov 26, other
compose tests have failed with the same `process_unreadable` (`compose_pending_tests.rs:139`), so it is a known,
recurring flake. It is unrelated to the seed work and to the worktree race, and this ticket leaves it alone. It needs
its own ticket.
