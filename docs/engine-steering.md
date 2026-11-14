# Steering engine agents

`farcooler message <key> "<direction>"` reaches the native Codex or Claude agent
working the ticket's current build, review, or agent gate. It queues the message
on the ticket. At the agent's next Stop hook, the host passes the direction back
as a continuation in the same session, before settling the step. Messages are
bound to that step's terminal identity; they never move to a replacement step.
Shared builds and group gates are reachable through each ticket they work.

Engine agents appear in `farcooler agent list` and the plan's agent rows with
their tickets. `farcooler agent stop <key>` stops the current native agent and
lets the engine settle the interrupted step. It does not cancel the ticket.

To suspend work temporarily, use the workspace UUID and the ticket key:

```sh
farcooler workflow pause <workspace-uuid> <key>
farcooler workflow resume <workspace-uuid> <key>
```

Pause suspends the current native process tree, its step timeout, and its quiet
periods. Resume continues the same processes and session in the same run. These
commands control a live native agent step; they do not pause CI, shell checks,
or landing. A daemon shutdown still uses the engine's existing interruption and
recovery rules. Stopping a paused agent works without resuming it first.

The workspace's owner, lead, maintainer, or orchestrator can steer its run.
Builders and reps cannot pause or stop another engine agent. Watching remains
read-only and retains its existing access rules.

An answer note is still recorded as an answer. The human CLI explains how to
send the running engine agent a message directing it to read that note. This
also works for direction too long to fit in a message:

```sh
farcooler task note <key> --kind answer --body "<full direction>"
farcooler message <key> "Read the answer note just added to this ticket"
```

Steering changes neither workflow progression nor the engine's ownership of
landing.
