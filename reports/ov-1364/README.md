# ov-1364 — native builders during a landing

## Canary incident

Group 4570's gate wrote `landing-gate: PASS` at 10:21:47 SGT and the
engine wrote its exit-0 process receipt at 10:21:55. The push was sent at
10:24:10. The losses therefore coincided with the gate returning and
releasing its builders, before the push was sent.

The Canary daemon log and the retained native owner/settlement records
identify two separate server sockets, each with its own pane `%0`.

| Effect prefix | Ticket | Native terminal / socket suffix | TTY | First dead read (SGT) |
| --- | --- | --- | --- | --- |
| `wf1:09c2aacc1956` | ov-1167 | `1ed24a4c-1d02-8af5-0c8a-63a1f150b837` | `/dev/ttys019` | 10:21:55.138208 |
| `wf1:0e9b8d75779a` | ov-1343 | `aef5d715-ea5f-1422-00f2-ad15d9db31a1` | `/dev/ttys003` | 10:21:55.137694 |

The two successful tmux reads for each pane reported the expected tag,
`pane_dead=1`, a blank pane PID, `pane_dead_status=0`, and a blank exit
signal. The engine logged “the native session was lost without a receipt”
after those reads. Read-only `list-panes` subsequently reported no server
running on either exact `farcooler-workflow-<terminal>` socket. The
macOS system log independently records both corresponding Codex processes
losing their XPC connections at 10:21:55.078 and 10:21:55.081, before tmux
reported the exits. Both retained launch receipts used `fish -ilc`.

## Cause and fix

When fish handles its foreground job's stop while the job is still stopped,
the interactive `fish -ilc` command can exit successfully. The gate used the
process table's order for both signals. Resuming fish ahead of its stopped
agent lets fish observe that stop and exit. The pane supervisor forwards
fish's zero exit status. Stopping the job ahead of fish can produce the same
exit. This accounts for a clean tmux pane exit with no turn receipt, and the
outcome depends on scheduling between the shell and its child. The incident
records preserve the exits and gate time. The signal interleaving was
reproduced on a private server.

In a scratch home, widening the gap between parent and child resumes made
all four real Codex 0.169.0 panes exit with the Canary signature. In the
same experiment, resuming descendants first preserved all four original
pane PIDs. Codex used a local fixture endpoint. The committed Rust
regression uses the production gate's signaling code and real fish/tmux.
Before the fix it failed with `fish pane after delayed resume: Dead`.

The gate now orders the native process tree by parentage. It checks that
each parent has stopped before sending stop to its children, and resumes
descendants before their parents. If a stop is still pending, the existing
gate sweep checks again. Gate admission also checks fresh process snapshots
before launching the gate command. The tty and supervisor identity checks
still guard signaling, and the tmux pane supervisor stays outside the pause.

Sources on the test host were the Canary `farcoolerd.log`, workflow
receipts, the two native tmux sockets, and the macOS unified log. The
fields in the pane observations follow the [tmux 3.8 format definitions](https://github.com/tmux/tmux/blob/3.8/format.c)
and [child reaping](https://github.com/tmux/tmux/blob/3.8/server.c).

## Four-builder landing

`four_native_builders_keep_their_panes_through_a_landing` queued a real
engine landing group only after all four native fixture builders had
started. The gate observed all four agents stopped. The engine pushed
to a scratch bare Git remote and finished the landing. All four builders
retained the same terminal, pane, and supervisor PID through the push,
then finished their original builds.

```text
before the gate {"builder-four": (0945f34e-2924-c77e-d84a-143523f028aa, "%0", 17373), "builder-one": (a57664d8-d09e-a1e4-25bd-f7922bb3e7e3, "%0", 17066), "builder-three": (6c8886ff-7717-5f41-5ba3-1966b058fc16, "%0", 17328), "builder-two": (b4167d9b-f838-d3d8-50a6-866b52eb5818, "%0", 17044)}
after the push {"builder-four": (0945f34e-2924-c77e-d84a-143523f028aa, "%0", 17373), "builder-one": (a57664d8-d09e-a1e4-25bd-f7922bb3e7e3, "%0", 17066), "builder-three": (6c8886ff-7717-5f41-5ba3-1966b058fc16, "%0", 17328), "builder-two": (b4167d9b-f838-d3d8-50a6-866b52eb5818, "%0", 17044)}
landing gate held four builders: [('17450', 'TN+'), ('17184', 'TN+'), ('17414', 'TN+'), ('17181', 'TN+')]
landing pushed a475da75c3ee22a5bc6cdb84ad9e302ee916bb0c -> 3ba067c7e2089fa0ca5955a0fc8bd7ce0a43ef4e; all four original builds passed
```

The landing and failure reproduction ran with scratch `FARCOOLER_HOME`
and private tmux socket directories. The landing used a fixture gate and
local Git remote.

## Regression log

```text
Before: fish pane after delayed resume: Dead
After:  fish pane after delayed resume: Live
        fish pane after delayed stop: Live
```

The findings are also on ov-1364 in comment `37c8c6d3`.

The affected workflow runtime, yielding, process, foreground, effect runner,
and tmux unit selection passed all 642 tests. The repository lint gate,
file-size budget, and Git whitespace checks passed.

## Linux CI loss during cleanup

Both attempts of main's `d4a239c45` Rust tests (2/4) failed in
`a_landing_gate_keeps_a_native_builds_pane_until_it_resumes_and_finishes`.
In the second attempt, `workflow_native_session_lost_without_receipt_leaves_running_in_one_scan`
was running at the same time and deliberately killed its own agent. Neither
failure log retained the raw panes. A concurrent Ubuntu scratch run also
showed a different live native pane unexpectedly exit with `signal=9`.

On Ubuntu with CI's tmux 3.4, a dead pane can retain its `pane_tty` after
the kernel assigns that terminal to another pane. The old cleanup walked
that tty even when tmux had already reported the original pane dead, then
killed the processes it found. In a controlled overlap of the two CI tests,
the dead pane still reported `/dev/pts/0` with exit 137, and the replacement
builder had a different supervisor PID on `/dev/pts/0`.

```text
Before: dead supervisor 157545, replacement supervisor 157703, both /dev/pts/0
        gate ended the builder: the native session was lost without a receipt
After:  dead supervisor 157952, replacement supervisor 158111, both /dev/pts/0
        both tests passed
```

Native owner receipts now retain the PTY supervisor's PID and start time.
Cleanup uses that session identity to reap remaining original processes and
ignore processes on a reused tty. With a legacy receipt, an occupied tty on
a dead pane leaves the original turn's quiescence unproven.
`a_dead_panes_reassigned_tty_cannot_end_another_native_pane` reproduced the
foreign pane's death on the CI base and passed with the fix.

## Immediate gate entry

The four-builder gate now takes a single `ps` sample as its first action.
On Linux, the test failed before admission waited for the native stop
signals. All four builder agents were still sleeping, with status `SN+`.

```text
landing gate entry: [('162645', 'SN+'), ('162513', 'SN+'), ('162626', 'SN+'), ('162555', 'SN+')]
```

Gate admission repeats the parent-first stop check until a fresh snapshot
shows no pending stops. A pause record still goes to disk before signaling.
If the record cannot be written or the pause cannot settle, no gate process
starts. Dropping the hold resumes the builders when the last gate ends.

The fixed Linux run logged the same terminal, pane and supervisor for each
builder before the gate {"builder-four": (5e31f16b-c04b-f4ce-8de7-f8e95cfb4207, "%0", 192264), "builder-one": (889a2149-c127-c0c6-c1b2-132b9cf3f1f7, "%0", 192000), "builder-three": (635a9939-d7f6-dd0a-f6af-e8b2564a8b5e, "%0", 192202), "builder-two": (4787191d-536c-55ce-83c2-6d0a97c3cd3b, "%0", 192086)}
after the push {"builder-four": (5e31f16b-c04b-f4ce-8de7-f8e95cfb4207, "%0", 192264), "builder-one": (889a2149-c127-c0c6-c1b2-132b9cf3f1f7, "%0", 192000), "builder-three": (635a9939-d7f6-dd0a-f6af-e8b2564a8b5e, "%0", 192202), "builder-two": (4787191d-536c-55ce-83c2-6d0a97c3cd3b, "%0", 192086)}
landing gate entry: [('192323', 'TN+'), ('192095', 'TN+'), ('192289', 'TN+'), ('192174', 'TN+')]
landing pushed f988417593437e32e45e4e62b87bcdfe9511a0b4 -> 632ab8307fdfebe43c9ab88e60d687ab9efd86a3; all four original builds passed
```text
before the gate {"builder-four": (d954a8af-ceb0-d2b4-bfbd-742e3014ce13, "%0", 170829), "builder-one": (1fdea9b8-b748-89f7-fa22-8a530c91b249, "%0", 170540), "builder-three": (14693451-c351-b5d1-1951-290600725025, "%0", 170790), "builder-two": (f8c604f4-db6a-f26b-15c4-64a685699c9b, "%0", 170646)}
after the push {"builder-four": (d954a8af-ceb0-d2b4-bfbd-742e3014ce13, "%0", 170829), "builder-one": (1fdea9b8-b748-89f7-fa22-8a530c91b249, "%0", 170540), "builder-three": (14693451-c351-b5d1-1951-290600725025, "%0", 170790), "builder-two": (f8c604f4-db6a-f26b-15c4-64a685699c9b, "%0", 170646)}
landing gate entry: [('170879', 'TN+'), ('170630', 'TN+'), ('170850', 'TN+'), ('170767', 'TN+')]
landing pushed 6c7adb3734b3acb925a0e2fd6da9e55a8ee69ca5 -> 8475a5b198527eb78fb11ba6ff9a82f85123f883; all four original builds passed
```

Round 1 findings are recorded on ov-1364 in comment `954f86ee`. The final
focused suites passed 53 tests on Linux and 38 tests on macOS, including
temp-file and tmux leak checks. In the broader macOS selection, 642 tests
passed and one test process exited with `SIGKILL`. That test passed in an
isolated rerun on the final code. The lint and file-size checks passed.
