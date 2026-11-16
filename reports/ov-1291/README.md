# ov-1291: a gated group pushes over a red previous landing

## What happened on Nov 16 (times +08:00)

From the engine's events and receipts, saved in
`test/fixtures/landing/nov16-group2924.json`:

- **02:17** Group 2895 (ov-1266, ov-1210) pushed `19bf27fd9`. Its CI is run 43745907131.
- **02:17 to 02:43** Group 2924 (ov-1199, ov-1267, ov-1277) folded on `19bf27fd9`
  while that CI ran. Its gate passed at 02:43:36.
- **02:43** 2924's push waited for 2895's CI before sending anything.
- **02:47:52** Run 43745907131 came back `failure`. 2895's CI step failed.
- **02:49:41** 2924's push wrote `base_moved` ("Previous landing CI failed") and
  took the back edge `push:1` to its base step. Main was still at `19bf27fd9`,
  the head 2924 had just gated on.
- **03:07:54** The base step waited 13 polls while the run was re-run. It took the
  same `19bf27fd9`.
- **03:08 to 03:47** 2924 folded and gated the same change again. The second gate
  went over its wall-time budget and was rejected.

No member went back to build in this sequence. It cost an 18-minute wait and a
second 40-minute gate, and that second gate failed for an unrelated reason. If CI
had stayed red, the base step would have taken the red head at once, and the
group would have folded and gated the same change on the same tree again.

The 09:50 case first filed on this ticket (group 3227) wasn't caused by red CI.
Its gate left a tracked file changed, and ov-1295 fixed that. The commit I had
for it (an engine-only guard in `scripts/gate-flaky.py`) is dropped from this
branch.

## The change

`crates/daemon/src/workflow_runtime/coding_verbs.rs`, in the push step:

- When the previous landing's CI comes back red, the push goes ahead. This only
  applies while the target is still at the head the gate ran on. The existing
  `target != base` check still sends a group back to base when anything else
  has moved main.
- The push's `passed` fact says `red_base: true`. The group's own CI step then
  decides, as for any push.

Pushing can't make the target worse here. The candidate descends from the red
head and passed the gate on top of it. If the group is the fix, it lands now.

## Tests

- `coding_red_base_tests::nov16_group2924_pushes_over_the_red_ci_it_gated_on_and_sends_nobody_back`
  replays the sequence from the fixture.
  - A landing ahead holds main so 2895's two members share one group.
  - 2895's CI answers with the fixture's `in_progress` poll, then its
    `failure` poll once 2924 reaches its push.
  - It asserts:
    - 2924 gated on 2895's head.
    - 2924 has one base step and one gate.
    - Its push fact says `red_base`.
    - Its candidate is what landed, and its own CI decided.
    - 2895 still fails on its own CI.
    - The build step ran once per ticket.
  - With the old `base_moved` branch put back, it goes red: 2924 never lands
    (`Failed at base attempt 3`).
- `coding_verbs_tests::next_group_gates_during_ci_and_pushes_over_a_red_one`
  covers the same rule with a daemon crash and recovery during the pre-send wait.

## Acceptance

1. Previous CI red, group gated on that red head, gate passes. The group pushes,
   its own CI is watched, and no member is handed back. Covered by both tests
   above. The push is decided in the daemon's push verb, not the reducer, so the
   tests are runtime tests.
2. **Not built:** an orchestrator verb to mark a waiting group as the fix for a
   red main. After this change no group waits because main is red. The base step
   takes a finished red head (ov-1076), and the push now goes over a red
   predecessor, so the verb would have nothing to act on.
3. Replay of the Nov 16 sequence from a saved fixture, with no member sent back
   to build. This is the 2924 replay above.
