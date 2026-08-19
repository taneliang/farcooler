# Stage 4 — a transcript that moves, and a clock that ticks

## The three complaints, verbatim

> 1. the transcript should display the latest text, kinda like a last-3-lines
>    snapshot of the actual transcript. currently, you're dislaying the front
>    few chars of the last 3 lines of output? but I do want like a constantly
>    updated window.
> 2. when using codex, there's no transcript, and no steps? in general it seems
>    like the integration with codex wasn't built at all
> 3. timers displayed in the sidebar aren't getitng updated until you click a
>    different panel

## What is actually true, from driving it

**Codex steps DO work**, and they stream correctly. A real codex 0.147.0 turn,
polled once every three seconds:

```
23:48:16 working  line='Writing a.txt'
23:48:22 working  line='Running sleep 4'
23:48:25 working  line='Writing b.txt'
23:48:34 working  line='Writing c.txt'
23:48:44 working  line='Running ls -1 a.txt b.txt c.txt'
```

**What is empty for the whole turn is the transcript.** It gained its first
entry only at `done`. So complaints 1 and 2 are ONE root cause, not two: the
transcript accepts only an agent's FINAL answer, and a final answer by
definition does not exist while you are watching. On a short turn the row says
nothing at all until it is over, which reads exactly like "the integration was
never built".

**Both agents narrate mid-turn, and we throw it away.** Codex writes
`AgentMessage` with `phase: "commentary"`. From the run above:

> `I'll create fruit.txt in the workspace, then verify its exact contents from
> the shell.`

Claude writes the same thing as `text` blocks on assistant records before a
tool call. Stage 3 excluded both on purpose — correct for a list of discrete
messages, wrong for a window that is supposed to move.

**A fourth thing, found while reproducing.** For the first nine seconds of a
new turn the row still showed the PREVIOUS turn's action (`Running test "$(<
fruit.txt)" = banana`). `Signals::saw` clears tasks and spawns on
`Started`/`Ended` but not `action`, so a new turn wears the old one's action
until its first tool call.

## The shape

**The transcript becomes a stream with a tail window.** Not "the last three
messages, each cut to its first 40 characters" — that is the current model, and
it is why it reads as truncated fragments. Instead: recent prose appended to a
rolling buffer, wrapped to the rung width, and the LAST three display lines
shown. New text pushes old text up, the way watching a terminal does.

Mid-turn narration goes in. That is what makes it move at all.

Two things must survive the change:

- `Feed::push` is the only constructor of a `Step`, and `redact::redact` plus
  `narrow_paths` run there. Agent narration is agent-authored text that can
  carry a path or a secret. Whatever replaces it keeps that choke point.
- A finished agent must still read as what it did. The tail of the stream after
  a turn ends is the final answer's last lines, so this should hold — but check
  it, because "idle and done agents should still have their summary" was an
  explicit ask.

**The clock ticks.** `Terminal.displayDuration` (`Model.swift:460`) computes
`Date()` at render time, and nothing observable changes while a turn runs, so
SwiftUI never re-renders and the string freezes until some other event forces a
redraw — clicking another panel. `AgentRows.swift:632` already uses
`TimelineView` for exactly this reason; the sidebar rows
(`SidebarViews.swift:615` and `:991`) do not.

One second is the right cadence: the string only changes that often, and
`.animation` would redraw at display refresh rate for no gain. Scope it to the
leaf text, not the list — a `TimelineView` around the whole sidebar re-renders
every row every second.

## Tasks

Sequential, one worktree, one git index.

1. **Let the narration in.** Claude's mid-turn `text` blocks and codex's
   `AgentMessage`/`commentary` become `Said`. Cursor equivalently if it has
   one. Keep the `end_turn` / `final_answer` distinction available — a
   conclusion may still deserve to outrank narration — but stop dropping
   narration on the floor.
2. **The tail window.** Replace the last-N-messages feed with a rolling text
   buffer rendered as the last three wrapped lines. Redaction and path
   narrowing stay at the same choke point.
3. **Clear the action on a turn boundary**, alongside tasks and spawns.
4. **Tick the sidebar clock**, leaf-scoped, once a second.

## Verification

Tests have never once caught a real bug in this feature. Drive a real codex AND
a real claude in a scratch daemon on a symlink-free path, and confirm the
transcript is non-empty and CHANGING mid-turn for both. Then run the Mac app and
watch a row's duration advance without touching anything.
