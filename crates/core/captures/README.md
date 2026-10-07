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

## The permission dialog a phone can answer

Captured 2026-09-28 against claude **2.1.283**, on a 140-column pane, by the
ov-14 spike's sampler (`/tmp/fc-t/ov14-spike/cap.sh`). Unlike the files above,
these are NOT raw `capture-pane` output. The sampler dropped blank lines and
lines that were only `─`, then dropped the first three lines of what was left
(not of the pane), and it rewrote spinner lines as `<spinner>`. Its `─` filter
missed one rule, the dialog's own (line 7 of the dialog file), which survived.
Recapture raw before tightening any rule against them.

- `claude-permission-hook-waiting.txt` is claude's dialog while its
  `PermissionRequest` hook is held (frame r1, t=1790550898.785). Its footer,
  `Esc to cancel · Tab to amend`, and `Do you want to` are what classify it
  Blocked.
- `claude-after-a-keyboard-yes.txt` is the same pane about 0.1 s after a
  keyboard Yes (frame r2, t=1790550967.443), with the working footer back.

The watcher releases a held phone ask on the edge between these two screens.
The dialog must have been seen, and then be missing for two samples (see
`crates/daemon/src/hook_asks.rs`).

## The ones with a message going into a working agent (ov-360)

Captured 2026-10-06 with `tmux capture-pane -p -e` (escapes kept, so dim and
reverse video read) on a 160x45 pane, against claude 2.1.290 and codex-cli
0.153.4 in a sandbox HOME, each streaming a slow turn from a local stand-in
for its API. A bracketed paste went into the box mid-turn, then Enter.

| file | what it shows |
| --- | --- |
| `claude-2.1.290-working-paste-160x45-e.txt` | the paste in the box; the footer has dropped `esc to interrupt` |
| `claude-2.1.290-working-queued-160x45-e.txt` | after Enter: the message queued above the spinner, `ctrl+x ctrl+s to send now`, and the box's dim hint with the cursor on its first letter |
| `codex-0.153.4-working-paste-160x45-e.txt` | the paste in the box; the footer is `tab to queue message`, not the model line |

Both agents took the message as the next prompt once the turn ended: claude
from its queue (`promptSource: queued` in the transcript), codex as a message
"submitted after next tool call".

## The ones with a message composed into codex (ov-416)

Captured 2026-10-07 with `tmux capture-pane -p -e` against codex-cli 0.153.4
in a sandbox (`HOME` and `CODEX_HOME` under `/tmp/fc-t`), on a stand-in for
its API, each after one or two bracketed pastes and no Enter.

| file | what it shows |
| --- | --- |
| `codex-0.153.4-idle-160x45-e.txt` | the empty box, its dim placeholder, the model footer |
| `codex-0.153.4-three-lines-160x45-e.txt` | three short lines, shown as pasted |
| `codex-0.153.4-blank-lines-160x45-e.txt` | `first\n\nthird\n\n\nsixth`: blank rows inside the box, then the blank row and the footer |
| `codex-0.153.4-long-paste-160x45-e.txt` | 1,001 characters, collapsed to `[Pasted Content 1001 chars]` |
| `codex-0.153.4-image-and-long-paste-160x45-e.txt` | an image's path pasted alone, `[Image #1]`, then 1,201 characters |
| `codex-0.153.4-tall-paste-80x24-e.txt` | thirty lines in a 24-row pane: the box scrolls, its first ten lines out of sight |
| `codex-0.153.4-slash-init-160x45-e.txt` | `/init`: the command popup below the box, in place of the footer |
| `codex-0.153.4-mention-popup-160x45-e.txt` | `look at @READ`: the file picker, `enter insert` |
| `codex-0.153.4-mention-no-matches-160x45-e.txt` | `try @zzzq`: the picker with no matches |
| `codex-0.153.4-skill-no-matches-160x45-e.txt` | `try $zzzq`: the skill picker with no matches |
