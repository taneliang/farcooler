# Real agent screens

Every signature in `activity.rs` should be answerable from a file in here. The
first version of that table was guesswork and matched no real screen; the cursor
entry stayed guesswork long enough to ship wrong.

Captured 2026-08-16 with `tmux capture-pane -p` on a 120x40 pane, against the
**publicly shipped binaries** rather than any local wrapper:

| agent | version | binary |
| --- | --- | --- |
| claude | 2.1.233 | `~/.local/share/claude/versions/2.1.233` |
| claude (`claude-asking.txt`, 2026-08-19) | 2.1.237 | `~/.local/share/claude/versions/2.1.237` |
| codex | codex-cli 0.147.0 | `/opt/homebrew/Caskroom/codex/0.147.0/bin/codex` |
| cursor-agent | 2026.08.11-e8db854 | `~/.local/share/cursor-agent/versions/…` |

This matters. On the machine these were taken from, `codex` and `cursor-agent`
on the `PATH` are wrapper scripts that add flags and side processes. The wrappers
changed none of these signals, but they do change `pane_current_command` —
`bash` under the wrapper, `codex` bare — and a corpus that only holds for one
developer's setup is not a corpus. Recapture against the bare binary.

Pane geometry matters. These are 40 rows; a shorter pane pushes transcript text
nearer the footer and narrows the margin the footer window relies on. A capture
taken at a different size should say so in its name.

## The one with nothing on it

`claude-asking.txt` is claude holding an `AskUserQuestion`, captured at the
SECOND turn of a session so the welcome banner has scrolled away. That is the
whole point of it: none of claude's four identity markers appear anywhere on
the screen, and `pane_current_command` for the pane was `2.1.237`, because
claude renames itself to its own version.

So the pane could not be identified by process OR by screen. `describe` fell
through to `shell`, `classify` returned `None` for a pane it could not name, and
the notification that went out while claude sat waiting for an answer read
**"shell finished"**. One gap, both halves of the sentence wrong.

The question box is what carries identity here, and `↑/↓ to navigate` is the
part of its footer to match: `Enter to select` beside it is cursor's trust gate
verbatim.

## The one that started it

`claude-idle-transcript-says-esc-to-interrupt.txt` is a **finished, idle** Claude
pane. Its footer reads `? for shortcuts` and its title glyph is `✳`. Twenty-four
lines up, in the transcript, Claude explains what the phrase `esc to interrupt`
means — because that is what it was asked.

Matching the whole screen classifies this pane as `Working`, forever. It never
reaches `Done`, so no notification ever fires, and its timer counts from whenever
that text first appeared. That is the bug, frozen.

## The ones where the main loop is idle and the work is not

Captured 2026-09-06 against claude **2.1.263**, the version the report came
from. Five files, and the version matters more here than usual: the agent-tree
footer these turn on — a `⏺ main` row with a `◯ <agent-type>` row under it per
running subagent, and a `· N shells · ↓ to manage` tray hint on the mode line —
does not exist on 2.1.233, which is why `claude-idle-fresh.txt` shows none of
it.

| file | pane | what is running |
| --- | --- | --- |
| `claude-background-agents-main-idle.txt` | 112x61 | three subagents, five shells |
| `claude-background-shell-main-idle-80col.txt` | 80x30 | one shell, no subagent |
| `claude-background-agent-main-idle-40col.txt` | 40x24 | one subagent, one shell |
| `claude-idle-nothing-running.txt` | 120x40 | nothing |
| `claude-idle-after-background-agents.txt` | 120x40 | nothing, twice over |

The bug they freeze: on all three of the first ones the main loop is BETWEEN
turns, so neither `esc to interrupt` nor `Thinking…` is anywhere on the screen,
and the pane classified as idle while an agent it had spawned was two and a
half hours into its work. What the screen actually says is
`Waiting for 3 background agents to finish` — at depth 11, three lines outside
the footer window, which is where it stays.

Two signatures came out of them and each is the only evidence for itself:

* **`· ↓ to manage`** is the tray hint. It is the only one of the two that
  appears for background SHELLS, which is half the report —
  `claude-background-shell-main-idle-80col.txt` holds a running shell and no
  subagent at all, so it carries no `◯`.
* **`◯ `** is the subagent row. It is the only one of the two that survives a
  narrow pane: at 40 columns the mode line truncates mid-word to `· ← 3 age…`
  and takes the tray hint with it, while the row is still there. The same
  crowding that costs opencode `ctrl+p commands` below 120 columns.

The separator in front of `· ↓ to manage` is load-bearing, and
`claude-idle-nothing-running.txt` is why. It was taken off the same pane the
narrow captures came from, at 120 columns, once its first subagent and its
shell had both finished — the pane was then resized down for those — and its
transcript still holds `⎿ Backgrounded agent (↓ to manage · ctrl+o to expand)`
— the tool result that started the work. On the mode line the hint always has a
separator in front of it, because the permission mode is always first; in that
transcript line it has a bracket. A pane that has finished must read finished
even with its own history above it, so the signature takes the form only the
mode line draws.

`N shells` is on the mode line beside the hint and is NOT a signature: it is two
ordinary words a person can type into the prompt box, and the prompt box, unlike
the transcript, is inside the footer window. `⏺ main` tracks subagents exactly —
it is absent from every idle capture — but says nothing `◯` does not, and `⏺` is
the glyph claude prefixes every tool call in the transcript with. `/tasks to see
subagents` is the trap of the three: it was seen on the mode line of a pane
whose subagents had all finished, twice, minutes apart, and it comes and goes on
its own — no capture here holds it, so it is written down and not relied on.

A fifth file, `claude-idle-after-background-agents.txt`, is that pane once the
work was done: a screen whose whole transcript is the evidence that something
WAS running — `Backgrounded agent (↓ to manage · ctrl+o to expand)` at depth 16,
`Waiting for 1 background agent to finish` at depth 12 — and which must read
idle anyway. It is the tightest such screen that could be produced deliberately:
the agent was asked to dispatch one short subagent and then say nothing, and
still wrote three more entries. Depth 16 is where `DEFAULT_FOOTER_LINES` gets
its margin from now, down from 18, and this is the file to re-measure against.

The screen is the second answer to this question, not the first. The pane's
session log states the same fact as a number, on the record that ends the turn
(`pendingBackgroundAgentCount`), and the daemon believes that over any footer —
see `resolved_activity` in `crates/daemon/src/watch.rs`. These signatures are
what stage 1 needs so that it stops contradicting it.
