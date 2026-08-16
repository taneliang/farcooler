# Real agent screens

Every signature in `activity.rs` should be answerable from a file in here. The
first version of that table was guesswork and matched no real screen; the cursor
entry stayed guesswork long enough to ship wrong.

Captured 2026-08-16 with `tmux capture-pane -p` on a 120x40 pane, against the
**publicly shipped binaries** rather than any local wrapper:

| agent | version | binary |
| --- | --- | --- |
| claude | 2.1.233 | `~/.local/share/claude/versions/2.1.233` |
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

## The one that started it

`claude-idle-transcript-says-esc-to-interrupt.txt` is a **finished, idle** Claude
pane. Its footer reads `? for shortcuts` and its title glyph is `✳`. Twenty-four
lines up, in the transcript, Claude explains what the phrase `esc to interrupt`
means — because that is what it was asked.

Matching the whole screen classifies this pane as `Working`, forever. It never
reaches `Done`, so no notification ever fires, and its timer counts from whenever
that text first appeared. That is the bug, frozen.
