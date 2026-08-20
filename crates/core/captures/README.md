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
