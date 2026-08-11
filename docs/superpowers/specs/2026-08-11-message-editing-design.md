# Editing what you wrote

Two complaints, one root cause, and one genuinely missing affordance.

> "I'm not able to edit previous messages. Queuing/steering messages also seems
> broken, it seems to just send it but it doesn't appear but clearly the llm
> reacts to it."

## The bug: two deciders on two channels

A message typed while a turn is running is supposed to be held. `ChatSession`
holds it (`crates/agent/src/chat.rs`), answers with `AgentEvent::PromptQueue`,
and sends it when the turn ends — at which point it emits the user's words as a
`Message` and they enter the transcript.

The client does not ask which of those happened. It **predicts** it:

```swift
// AgentComposer.swift
let working = terminal.agent == .working
Task { await stream.send(body, images: images, whileWorking: working) }
```

and `AgentStream.send` draws the local echo only `if !whileWorking`. The comment
above it says the quiet part out loud: *"Whether this is going out now or joining
the queue is the daemon's decision, but the composer already knows the answer."*

It does not always know the answer, because the two sides read different
channels. `stream` polls the agent channel every 200ms. `terminal.agent` is
fleet state on a separate path. Between `TurnEnded` reaching the agent channel
and the fleet record dropping `.working`, the composer believes a turn is
running while `ChatSession` believes it is idle. In that window:

1. the composer suppresses the local echo, expecting a queue row;
2. `ChatSession` is idle, so it sends immediately — no `PromptQueue` is emitted;
3. the CLI echoes the prompt back, and `user_to_events` drops it live, because
   dropping it is what stops every prompt being drawn twice.

Three correct-in-isolation decisions, and the message reaches the model without
ever being drawn. That is exactly the reported symptom: *sent, invisible, and
the model reacts to it.*

That window is when people type — right as the agent finishes.

### What was ruled out

The first hypothesis was that a subagent's sidechain emits its own `result`
frame, which `normalize.rs` turns into `TurnEnded` with no `parent_tool_use_id`
check, ending the turn early. A live probe against claude 2.1.226 dispatching a
real subagent produced **exactly one** `result`, `parent = None`. The missing
parent check is still a latent fragility worth a comment, but it is not this
bug.

## Editing a queued message already works

`QueuedRow` (`apps/macos/Sources/FarCooler/AgentRows.swift`) already has edit,
cancel and steer, wired to `agent-edit-queued` / `agent-cancel-queued` /
`agent-steer-queued`, rendered from `stream.transcript.queue`
(`AgentSurface.swift`). `DaemonMessage::EditQueued` and `ChatSession::edit_queued`
are implemented and tested.

None of it appears, because the bug above means nothing ever reaches the queue.
Fixing the echo makes this feature visible rather than building it.

## Design

### Part 1 — echo locally, then reconcile

The composer stops predicting.

- `AgentStream.send` **always** appends the message locally, and `whileWorking`
  goes away.
- When a `PromptQueue` event arrives carrying an entry whose text matches a
  locally-echoed message that has not yet been confirmed, the transcript
  **removes that row**. It belongs in the queue, not the chat.

Your words never vanish and never lag; the daemon stays authoritative about
which bucket they landed in. A queued message is removed from the chat and
becomes editable, which is the behavior asked for.

Reconciliation is by text against locally-echoed-but-unconfirmed rows only, so a
queue entry cannot delete a message the agent genuinely received earlier. If no
match is found, nothing is removed — the failure mode is a duplicate row, never
a lost message.

Rejected alternatives:

- *Point the prediction at `stream` instead of `terminal`.* Cheapest, removes
  the cross-channel lag, keeps the class of bug alive.
- *Never echo; wait for the daemon.* Authoritative and simplest, but your own
  text is invisible for up to 200ms in the one interaction that must feel
  instant.

### Part 2 — Edit on a sent message

A user message row gains an **Edit** action that prefills the composer with its
text and focuses it. The original stays. Nothing rewinds, nothing re-runs, no
protocol or daemon change.

This is deliberately not a rewind. Rewinding would mean truncating the
conversation and re-running, which neither backend supports symmetrically and
which cannot undo what the agent already wrote to disk. Prefilling is what
Claude Code does and what was asked for.

### Part 3 — the user's own words are not the agent working

`activity_source::observe` maps every `Message` to `Working`, including
`Role::User`. A pane is marked busy because you typed. Harmless on its own, but
it feeds the state the composer mispredicts on, so it is corrected here:
`Role::User` implies nothing about what the agent is doing.

## Non-goals

- Rewinding, truncating or forking a conversation.
- Rolling back files an agent already changed.
- Editing a message that has reached the model.

## Testing

- Swift: a locally-echoed message is removed once it appears in a `PromptQueue`;
  a message sent immediately keeps its row; an unmatched `PromptQueue` entry
  removes nothing.
- Rust: `observe` returns `None` for `Message { role: User }`.
- The existing `ChatSession` queue tests already cover holding, editing,
  withdrawing and draining, and need no changes.

None of these needs a live agent.
