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
gate sweep checks again. The tty and supervisor identity checks still
guard signaling, and the tmux pane supervisor stays outside the pause.

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
