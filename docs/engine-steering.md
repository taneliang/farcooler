# Steering engine agents

`farcooler message <key> "<direction>"` reaches the native Codex or Claude agent
working the ticket's current build, review, or agent gate. It queues the message
on the ticket. Codex and Claude receive it at their next supported `PreToolUse`
hook: that tool is refused with the direction as its reason, so the agent can
change course before running it. This does not interrupt a tool already
executing. Both also receive a continuation in the same session at Stop if they
reach it without another supported tool call, before the step settles. Explicit
messages to a running native engine agent work even when the workspace has
`wake_on_answer` disabled. That preference still controls automatic answer wakes
and message delivery into ordinary panes.

Native Claude settings and Codex hooks must include the installed all-tools
gating `PreToolUse` registration targeting this runtime's private host.
Lifecycle-only settings,
disabled hooks, and altered tool gates are refused when the configuration opens
and before a native step launches. Other user hooks may coexist with that entry.
Existing Codex workflow pins must be updated to the engine hook file containing
this registration. The engine also passes the pinned Codex hooks through a
per-launch `--config` override, so delivery does not depend on discovery of the
worktree's project layer. Codex must trust the updated hooks as it does other hooks.
Tool-boundary delivery was verified with Codex CLI 0.167.2. Codex does not fire
this hook for hosted tools or for `write_stdin` on an existing shell command.
`codex queue` starts a later turn and is not the engine steering route.
See [Codex hook coverage](https://learn.chatgpt.com/docs/hooks#tool-coverage).

Message admission and the final Stop queue check share a lock. Direction accepted
before that check extends the session; messages after the final check are
rejected even while the agent row remains visible during capture and quiescence.
If the step times out, stops, or otherwise exits with direction still queued,
the ticket records the undelivered message and asks the sender to send it again
to the current agent. Messages stay bound to that step's terminal identity;
they never move silently to a replacement step.
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

## When a provider is at its limit

An agent at its provider's usage or rate limit fails its step at once, with the
reason `usage limit until <time> (Codex)` or `(Claude)`. No attempt is spent, and
the retry that follows is free. Codex is read from its own error line on the
pane, and only while the latest state it reports to its terminal (OSC 7501), if it
reports one, is an error. Claude is read from its `StopFailure` hook. An error
state with some other message isn't called a limit.

Needs You lists one item for each provider in each workspace, naming every ticket
it stopped. Those tickets don't also ask for a decision of their own. A question
somebody asked on one still shows.

To move a queued, failed or running ticket to another model:

```sh
farcooler workflow model <workspace-uuid> <key> claude:opus
```

The model must be one the step lists in the workflow's `models`. The command
moves the step the run is at, or the build before it; `--step` names another. A
running agent of that step is stopped first, and the step starts again on the new
model in the same worktree, uncommitted work included. A failed run runs again,
as `workflow retry` would run it.

A ticket still waiting to start, such as one the start policy has queued, takes
the same command. Nothing starts: the pick is kept on the ticket, checked against
the published workflow's list for its first build step (or `--step`), and its run
uses it instead of the start policy's rotation. `workflow start --model` still
names a different one.

A workflow can also name a fallback for a step, one of the models it lists:

```json
"models": { "build": ["codex:gpt-5.5", "claude:sonnet"] },
"fallbacks": { "build": "claude:sonnet" }
```

When the step's agent is at its limit and the fallback runs on another provider,
the step starts again on the fallback and the ticket says so. Each ticket reaches
the limit once before it moves. If the fallback is at its limit too, the step
fails. Without `fallbacks`, nothing changes models by itself.

## A stalled build

A native agent that prints nothing and uses next to no CPU (under 2% of one core)
for ten minutes is listed in Needs You as stalled, with the last lines of its
terminal, and its ticket reads `Stalled`. Nothing is stopped: watch it, message
it, stop it, or move it. Time a landing check holds it paused doesn't count.
