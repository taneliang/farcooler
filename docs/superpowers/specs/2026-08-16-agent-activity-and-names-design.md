# What an agent is doing, and what a pane is called

A pane running Claude Code sat finished and idle while Far Cooler reported
`Working` — for thirty-three hours. It never reached `Done`, so no notification
ever fired, and the timer counted from a transition that had nothing to do with
the work. Meanwhile every agent pane in the fleet was called `claude` and every
other pane was called `shell`.

These are one problem wearing three hats. Far Cooler asks a screenshot what an
agent is doing, and a screenshot cannot answer. This design replaces the guess
with the agent's own account of itself, and keeps the guess only as a floor.

It matters now rather than later because Live Activities and widgets will put
this state on a lock screen. A wrong glyph in a sidebar is an annoyance you can
correct by looking. A wrong glyph on a lock screen is a promise that was broken
while nobody was watching.

## What is actually wrong

Every claim below was read off a running agent, not reasoned about. The captures
are in `crates/core/captures/`.

### The classifier reads the conversation

`Registry::classify` matches its signature table against the whole visible pane.
The visible pane includes the transcript, and the transcript contains whatever
the user typed and whatever the agent said back.

The reproduction is exact. Claude Code was asked to write a haiku and to explain
what the phrase `esc to interrupt` means in a terminal UI. It did both and
stopped. Twenty samples later the pane was still idle — the title glyph had
flipped to `✳`, the footer read `? for shortcuts` — and `classify` still
returned `Working`, because Claude's own answer, sitting in the transcript, said
`esc to interrupt`.

The detector detected itself.

The same mechanism runs the other way. `blocked` is checked before `working`, and
Claude ends turns with "Do you want me to…" as a matter of habit. Any such
sentence pins a finished pane to `Blocked` — a "Needs you" that nothing can
clear, on a pane that needs nobody.

Once wrong, it stays wrong. `advance` only mints `Done` on a `Working → Idle`
edge, and that edge never comes.

### Depth separates truth from transcript

Measured across the captures, from the last non-blank line upward:

Measured against the **publicly shipped** binaries — `claude` 2.1.233, the
Homebrew `codex` 0.147.0, and `cursor-agent` 2026.08.11 — not against the
superset wrappers that shadow codex and cursor-agent on this machine. The
wrappers turned out to change nothing about these signals, but a corpus that
only holds for one developer's setup is not a corpus. They did change one thing:
under the wrapper `pane_current_command` reads `bash`, where the bare binary
reads `codex`.

| agent | signal | depth |
| --- | --- | --- |
| claude | working, `esc to interrupt` | 1 |
| claude | idle, `? for shortcuts` | 1 |
| claude | blocked, `Esc to cancel · Tab to amend` | 1 |
| claude | blocked, `❯ 1. Yes` | 5 |
| claude | blocked, `Do you want to create haiku.txt?` | 6 |
| codex | blocked, `Press enter to confirm or esc to cancel` | 1 |
| codex | blocked, `› 1.` | 5 |
| codex | working, `esc to interrupt` and `Working (` | 6 |
| codex | blocked, `Would you like to run the following command?` | 11 |
| cursor | blocked, `Skip & tell the agent what to do instead` | 1 |
| cursor | working, `ctrl+c to stop` | 5 |
| cursor | blocked, `Not in allowlist:` | 5 |
| cursor | blocked, `Run this command?` | 6 |
| **claude** | **false positive, the transcript** | **18, 24, 26, 38** |

Every signal needed to decide the state is at depth 6 or less, for all three
agents. Only codex's question text sits deeper, at 11. Every false positive is at
18 or more. The
status furniture an agent draws lives at the bottom of the pane, because that is
where a person looks; the conversation scrolls above it. Nothing in the design
required this to be true, but it is true of all three agents, and it is the
cheapest correct fix available.

### The timer measures the wrong event

`changed_at` records when the classifier last changed its mind. Two consequences
follow even after the classifier is fixed:

- Approving a permission prompt walks `Working → Blocked → Working`, and the
  clock restarts. "Working 12m" becomes "Working 2s" because you said yes.
- There is no field for the question a person actually asks at 3am, which is how
  long this task has been running, not how long the current label has held.

### Naming answers a question nobody asked

`Registry::describe` returns the preset name whenever a pane identifies as an
agent, so every agent pane is `claude`. Otherwise it returns a summary of the
foreground argv, and `Model.name(of:)` folds every shell to `shell`.

The argv summary is worse than it looks. `foreground::summarize` keeps the first
argument only when it is not a flag, on the reasoning that a subcommand is the
distinguishing part. Measured against real panes:

| argv | label today |
| --- | --- |
| `node` | `node` |
| `top -l 0` | `top` |
| `vim notes.md` | `vim notes.md` |
| `python3 -m http.server 8099` | **`Python`** |
| `fish` | `shell` |

The rule discards exactly the informative part of every modern runner
invocation: `python -m http.server`, `node --inspect server.js`,
`cargo run -p api`, `npm --silent run dev`.

### Three signals exist and none of them are read

Every one of the three first-class agents sets an OSC 2 pane title. The daemon's
`list-panes` format string never asks for `#{pane_title}`.

| agent | working | blocked | idle | name |
| --- | --- | --- | --- | --- |
| claude | `◐`, `◑` | (same as idle) | `✳` | task summary |
| codex | braille spinner | `[ . ]` / `[ ! ] Action Required` | no glyph | cwd basename |
| cursor | no glyph | no glyph | no glyph | task summary |

No agent gives everything. Codex's title is the only one that distinguishes
blocked — it blinks `[ . ]` against `[ ! ] Action Required` — while claude's
shows the same `✳` for blocked and idle alike, and cursor's carries no status at
all. For names it is the other way round: claude and cursor summarize the task,
codex only repeats the directory. Read together they cover both questions for
all three; read alone none of them does.

### The titles are the agents' own, not something already installed

Worth establishing, because this machine has `herdr` and `superset` hooks
registered in all three agents, and `codex` on the `PATH` is a superset wrapper
script rather than the real binary. A hook painting the title would make this
whole layer an accident of one developer's setup.

It is not. Five checks:

- the claude binary contains `useTerminalTitle` and `terminalTitleFromRename`
- the real codex binary contains `terminal_title` as a config key, listed beside
  `theme`, `keymap` and `status_line_use_colors`, and the literal
  `Action Required`
- the superset codex wrapper contains no OSC sequence and no title code at all
- no installed hook script — superset's or herdr's — contains a title escape,
  and neither `~/.claude/settings.json` nor `~/.codex/config.toml` configures one
- the codex spinner advanced through ten distinct braille frames in 5.2 seconds,
  and its blocked title blinked for thirty-five seconds while the agent sat
  still at a prompt

The last is decisive on its own. Hooks fire on discrete lifecycle events. Nothing
fires three times a second, and nothing fires repeatedly while an agent is doing
nothing at all.

One consequence follows from `terminal_title` being a *config key*: a user can
presumably turn it off. Layer 3 must therefore degrade to layer 4 rather than
assume a title is there — which is what "unknown glyph means no opinion" already
does, now for a second reason.

And all three support hooks — `~/.claude/settings.json`, `~/.codex/hooks.json`,
`~/.cursor/hooks.json` — which report turn boundaries and permission requests as
events, with no polling and no string table. Cursor's are the most precise:
`beforeShellExecution` and `beforeMCPExecution` say "this agent is blocked" as a
fact rather than an inference.

### Cursor's rules were never verified

The table's own comments admit it: the cursor entry was written against a
sign-in screen nobody could get past. It is wrong. `Do you want to`, `Allow?`,
`› 1.` and `Generating` appear nowhere in a running cursor-agent. The real
furniture is in the captures and in the table below.

## The shape of the fix

Four sources, ordered. Each is authoritative over the ones below it while its
information is fresh; when it goes stale the next one takes over.

| | source | answers | covers |
| --- | --- | --- | --- |
| 1 | protocol — ACP or native backend | everything | agent-mode panes; exists today |
| 2 | **the agent's own session log** | turn boundaries, tools, subagents, tokens | claude, codex, cursor |
| 3 | pane title (OSC 2) | working bit, name | claude, codex, cursor |
| 4 | screen, footer-scoped | working, blocked, idle | everything else |

Layer 4 never disappears. An agent Far Cooler has never heard of still gets a
reading, and that is the point of keeping it: the floor is what makes the
feature honest about agents it does not know.

The layering is a single function over a per-terminal record, not a chain of
special cases scattered through the watcher. One place decides, so a Mac badge,
a phone notification and a Live Activity cannot disagree — the property the
derivation model already exists to protect.

The two stages are two implementation plans, not one. Stage 1 is self-contained
and ships on its own: it fixes every failure reproduced above using only what a
pane already shows. Stage 2 reads the agents' own session logs, and is worth
planning separately once stage 1 is real.

**Stage 2 does not need hooks.** All three agents write a structured JSONL
record of the session to disk as they go, and reading it beats being called by
it: nothing has to be installed, nothing is written to a file Far Cooler does not
own, and it works for an agent the user started before Far Cooler existed. Hooks
remain a fallback worth keeping in mind, not the plan.

## Stage 1 — correct the guess

Everything here works on an untouched machine. Nothing writes to a user's
config.

### Scope the match to the footer

`classify` matches `working` and `blocked` only within the last N non-blank lines
of the pane. `identity` keeps the whole screen: an identity string is furniture
an agent always draws, a banner sits at the top, and a stray identity match
promotes nothing on its own.

**Two windows, because they answer different questions.** Deciding the state is
safety-critical: a false `Blocked` is a notification nobody can clear, and a
false `Working` is the thirty-three hour bug. Reading the question text is not —
at the point it runs the state is already `Blocked`, and the worst case is an
unhelpful string in a sidebar.

| window | lines | holds |
| --- | --- | --- |
| decide | 8 | every state signature, all three agents, deepest at 6 |
| extract | 12 | codex's question, the only thing deeper, at 11 |

The decision window is one number rather than a per-agent field, because the
measurements came out that way: nothing needed to classify a state sits below
depth 6 in any of the three. It stays a field on `AgentRules` so an agent added
by config can widen it, but all three built-ins take the same 8.

The extraction window never feeds back into classification, so widening it
cannot reintroduce the bug. That is the whole reason for keeping the two
separate rather than taking 12 everywhere.

The margin is ten lines — a decision window of 8 against the nearest measured
false positive at 18. That false positive is claude's; for codex and cursor none
was observed at all, so their side of the margin rests on the shape of the
layout rather than on a counter-example.

Every margin is a function of pane height. These captures are from a 40-row
pane; a 15-row pane pushes the transcript nearer the footer and narrows the gap.
That is the honest reason stage 2 exists rather than being optional polish.

### Say what the agent is asking

A row that says "Needs you" makes you go and look. A row that says what is being
asked lets you decide whether to. On a lock screen, where looking costs the most,
that is most of the value.

All three draw the same shape inside the extraction window: a line ending in `?`,
optionally with a `$`-prefixed command above it.

| agent | question | subject |
| --- | --- | --- |
| claude | `Do you want to create haiku.txt?` | in the question |
| codex | `Would you like to run the following command?` | `$ echo banana > fruit.txt` |
| cursor | `Run this command?` | `$ echo banana > fruit.txt in .` |

So: within the extraction window, the last line ending in `?` is the question,
and the nearest `$`-prefixed line above it is the subject. Both are truncated for
display and carried in a new `blocked_question` field beside `activity`.

It is derived only while the state is already `Blocked`, and it is always
optional — a question that cannot be parsed leaves the field empty and the row
reads "Needs you", exactly as it does today. Nothing regresses when an agent
changes its wording; it just stops elaborating.

In stage 2 the same field is filled from structure instead of text: cursor's
`beforeShellExecution` carries the command outright, and claude's `Notification`
carries the message. Codex has no blocked hook, so the screen remains its source
— which is precisely why this is worth building at layer 4 rather than deferring
it wholesale to hooks.

### Read the pane title

Add `#{pane_title}` to the `list-panes` format in `crates/tmux/src/windows.rs`
and carry it on the pane record.

Parsing is two questions asked of one string. A leading glyph answers the status
question: a braille spinner (`⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏`) or `◐◑◒◓` means working; `✳` or no
glyph means not working; anything else means no opinion, which falls through to
the layer below rather than guessing. The remainder answers the name question,
and is rejected when it is not a name:

- the machine's hostname, which is what a plain shell leaves behind
- a filesystem path, which is what fish leaves behind (`/p/t/c/-/9/s/probe`)
- the program's own name, which says nothing the command did not
- the bare harness name (`Claude Code`, `Cursor Agent`), which is the
  placeholder an agent shows before it has a task to describe

A title that survives all four is a name a program chose for itself, which is
better than anything Far Cooler can derive.

### Anchor the timer to the turn

Replace the single `changed_at` with two clocks on the observation record:

```rust
struct Turn {
    /// When the user's request started. Set on Idle -> Working. Held across
    /// Blocked, because approving a tool call does not start a new turn.
    started_at: i64,
    /// When the CURRENT state began. Reset on every state change.
    state_since: i64,
}
```

`started_at` clears on `Done` and on `Idle`. Both go on the wire beside
`activity`, so a Live Activity renders "Working 12m" and "Needs you 2m" without
recomputing either, and without a phone and a Mac deriving them differently.

This is the field a lock screen reads. It is specified now, before there is a
widget, so the widget does not have to invent it.

### Two samples to change, one to block

A state change publishes after two consecutive agreeing samples. A single
half-drawn capture mid-redraw then cannot fire a false `Done`.

`Blocked` is exempt and publishes on the first sighting. A question that arrives
a second late is fine; a question that never arrives is the one failure that
makes the whole feature pointless.

### Correct the cursor rules

From the captures, replacing guesses with observations:

- identity: `→ Add a follow-up`, `Cursor Agent`
- working: `ctrl+c to stop`
- blocked: `Run this command?`, `Not in allowlist:`,
  `Skip & tell the agent what to do instead`, `Run Everything (shift+tab)`,
  `[a] Trust this workspace`, `Do you trust the contents of this directory?`

`⠞ Working` is deliberately **not** in the working list. It appeared in the
wrapped run and is absent from the bare one, which drew only `ctrl+c to stop`
while working — so it is drawn for some turns and not others, and a signature
that is sometimes there is worse than none. `ctrl+c to stop` was present
whenever cursor was working and absent whenever it was not.

Codex's list needs one correction of the same kind. `Press enter to continue` is
in the table today and matches nothing: the approval prompt reads
`Press enter to confirm or esc to cancel`. The string it was presumably written
for is on the *trust gate*, which is a different screen. Both are worth having,
spelled correctly.

The trust gate is included deliberately. It is the first screen a new workspace
shows and it blocks everything behind it, so a pane sitting on it is precisely a
pane that needs you.

### Say what a non-agent pane runs, and what it does

Two changes, in that order of confidence.

**The command, done properly.** `summarize` skips leading flags and keeps the
first meaningful token, with the runner forms named rather than inferred:
`python -m http.server` reads as `python http.server`, `npm run dev` as
`npm run dev`, `cargo run -p api` as `cargo run api`, `node --inspect server.js`
as `node server.js`.

**The purpose, from the process rather than the pixels.** A pane's foreground
process group either holds a listening socket or it does not; that is a fact
about the machine, readable without interpreting a single character of output. A
pane serving on 8099 reads `web :8099`. This is deliberately not a regex over
scrollback — pattern-matching prose is the mistake this whole document exists to
correct, and it would be perverse to reintroduce it two sections after removing
it.

Ports are read on the same cadence as `ps`, once for the host per sample, not
once per pane.

Anything needing progress counts (`vitest 42/50`) is out of scope. It cannot be
had without reading output, and the confidence would not survive contact with a
second test runner.

### Precedence, in one place

1. a name the user typed — never overwritten, as `remember_agent_title` already
   guarantees
2. the agent's own summary — the ACP session title, or the OSC title
3. the command and its purpose — `python http.server`, `web :8099`
4. `shell`

Codex is the gap in rung 2, and stays one until stage 2. Its title is only the
cwd basename, which the workspace already says, so a codex pane names itself from
its command in stage 1. Stage 2 fills it from the session log's first
`user_message`, which is the nearest thing codex has to a summary of its own
work.

## Stage 2 — read what the agent already writes

Every one of the three keeps a JSONL record of the session on disk, appended as
the work happens. Reading it is strictly better than being called by a hook:
nothing is installed, no file Far Cooler does not own is written, and it works
for an agent the user started long before Far Cooler was running.

| agent | file |
| --- | --- |
| claude | `~/.claude/projects/<slug-of-cwd>/<session-uuid>.jsonl` |
| codex | `~/.codex/sessions/YYYY/MM/DD/rollout-<ts>-<session-uuid>.jsonl` |
| cursor | `~/.cursor/projects/<slug-of-cwd>/agent-transcripts/<uuid>/<uuid>.jsonl` |

### What each one says

Read off real sessions, not documentation.

Codex is the richest, and its shape is a plain event stream:

| payload | carries |
| --- | --- |
| `task_started` | `turn_id`, `started_at`, `model_context_window` |
| `task_complete` | `turn_id`, `completed_at`, `duration_ms`, `time_to_first_token_ms` |
| `token_count` | `total_token_usage`, and `rate_limits.used_percent` |
| `agent_message` | `phase: final_answer` |

`task_started` and `task_complete` are the turn clock, exactly — no inference at
all.

Claude's is a message log with metadata on every record: `aiTitle`,
`pendingBackgroundAgentCount`, `isSidechain`, `permissionMode`, `effort`,
`gitBranch`, `stop_reason`, per-message `usage`, and every `tool_use` including
the `Agent` calls that spawn subagents.

Cursor's is the simplest and still complete: a `user` record opening the turn, an
`assistant` record per step whose `tool_use` carries both a `command` and a
human-written `description`, and `{"type":"turn_ended","status":"success"}`
closing it.

### What reconnaissance found, and what it refuted

Everything above this heading was written from a handful of sessions captured by
hand. Before stage 2 was planned, four investigators went through every session
log on one developer's machine — 276 claude files, 183 codex, and every cursor
transcript that exists. Some of what they found contradicts what is written
above, and the contradictions matter more than the confirmations.

**The blocked state is not in any of the three logs.** This is the finding that
reshapes the stage. Claude records the OUTCOME of a permission decision —
`toolDenialKind` is `automode-blocked`, `user-rejected` and so on — but never
the pending state, so a log reader cannot tell that an agent is waiting right
now. Codex's rollout has nothing approval-shaped at all across 183 files.
Cursor's transcripts show none either, though with only four files totalling
under two kilobytes that is suggestive rather than settled.

So stage 2 does NOT replace the screen. It replaces it for turn boundaries,
timing, the feed and the counts — where the logs are exact and the screen was
guessing — and the footer scoping from stage 1 remains the only thing that can
say "this agent needs you". That is the one state the whole feature exists for,
and it stays where it already works.

**Three specific claims above are wrong.**

- Subagents are not marked by `isSidechain`. Claude writes each subagent to its
  own file under `<project>/<session-uuid>/subagents/agent-<id>.jsonl`. The
  field itself does exist — 40,724 occurrences in one project — but is `false`
  in every one, so it marks nothing. (An earlier draft said it appeared zero
  times, which was wrong: a reader could have concluded claude had dropped it.)
- Claude's `timestamp` is not monotonic. 235 of 276 files contain adjacent lines
  whose timestamps go backwards. Line order is the only order a parser may
  trust.
- Codex's `started_at` and `completed_at` are unix **seconds** while
  `duration_ms` is milliseconds, in the same payload.

**What the logs are genuinely good for**, confirmed rather than assumed: codex
opens a turn with `task_started` and closes it with `task_complete` or
`turn_aborted`, all carrying a matching `turn_id`; claude opens with a `user`
record carrying `promptId` and `promptSource`, and closes with an explicit
`system` / `turn_duration` record that always appears and carries `durationMs`
and, when it is above zero, `pendingBackgroundAgentCount`.

**Hazards a parser has to survive**, all observed: single claude lines over a
megabyte inside files over a hundred; five of 183 codex files ending on a
`task_started` with no matching completion, leaving a turn open forever; and
codex's `rate_limits` shape differing by subscription plan.

### Which file belongs to which pane

The one hard problem, and it has a different answer per agent.

Codex holds its session file **open**, so `lsof -p <pid>` on the pane's
foreground process names it outright. Verified against a running codex.

Two nuances the first pass missed, both from watching a bare codex rather than a
wrapped one. The rollout file is not opened when the process starts — only once
the first turn is submitted, so a freshly launched pane has no file to find yet.
And a codex started through a wrapper holds a SECOND, unrelated jsonl open (the
wrapper's own shadow log), so the match has to be against
`~/.codex/sessions/**/rollout-*.jsonl` specifically rather than "the open jsonl".

Claude and cursor append and close, so the file cannot be found that way.
Instead the pane's cwd gives the project directory by a deterministic slug, and
within it the file being appended to is the live session. When two panes share a
cwd — normal in a worktree — the tie is broken by the pane's OSC title: claude's
transcript carries `aiTitle`, and that string is **exactly** what the title
shows once the leading activity glyph is stripped. Verified live on a bare
claude: pane title `✳ Write haiku about lighthouse`, transcript
`"aiTitle":"Write haiku about lighthouse"`.

So the title stops being merely a nice name and becomes the join key.

**The two slug algorithms are not the same, and neither is guessable.** Cursor's
was recovered from its own source (`workspace-paths.js` in the shipped bundle):
every non-alphanumeric character becomes `-`, runs collapse to one, leading and
trailing hyphens are trimmed, and if the joined path exceeds 92 characters it is
cut to 84 and given a `-` plus the first seven hex of the full path's sha256.
Claude's was established behaviourally across 35 real directories and one clean
live test: it does NOT collapse runs, so `/Users/e-liang/.unixconfig` becomes
`-Users-e-liang--unixconfig` — a double hyphen where cursor would write one. No
truncation behaviour was observed for claude, but no path long enough to trigger
one was tried, so that is unknown rather than absent.

One trap for anyone testing this by hand: a `claude` launched from inside a
claude session inherits `CLAUDE_CODE_SESSION_ID` and adopts the PARENT session's
`aiTitle`, which makes the join look broken when it is not. Strip
`CLAUDE_CODE_*` and `CLAUDECODE` before spawning a test agent.

And the column to put the answer in already exists: `terminals.agent_session_id`
is in the store, unpopulated, reserved for exactly this.

### Watching cheaply

Directory watches — fsevents on a Mac, inotify on Linux — over the three roots,
and on each event read only the bytes appended since the last offset. No
`capture-pane`, no per-pane polling, no work at all while an agent is quiet.

This is cheaper than what runs today, not more expensive: the current design
spends one tmux round trip per pane per second forever, and this spends nothing
until an agent writes a line.

### Honesty about the cost

These are private formats with no compatibility promise. They will change
without notice, and a schema that changes silently is the same class of hazard as
a screen signature that changes silently — which is the bug this document exists
to fix.

Three things keep that honest. The layering already handles it: a log that
cannot be parsed falls through to the title, and then to the footer. The
parsing reads only the fields named above and ignores everything else, so an
added field breaks nothing. And the corpus discipline extends — a recorded
transcript fixture per agent, asserted the way the screen captures are.

### A running feed, because "Working" says nothing

A row that says `Working` is opaque: it reports that something is happening and
refuses to say what. The logs already know, so an active agent shows its **last
three steps**.

A step is a verb and an object, derived once in the daemon so a Mac, a phone and
a watch cannot render the same event three ways:

```
  bash   cargo test -p farcooler-core
  edit   watch.rs
  says   Baseline clean — 145 passed…
```

The verb is the tool name, normalized. The object is the first of the tool's
`file_path` basename, its `command`, its `description`, or its `pattern` —
cursor supplies a written `description` per call, which is the best of them
(`Write banana to fruit.txt`). Assistant prose becomes a `says` step carrying its
first sentence. Everything is truncated to about forty characters in the daemon,
so no client invents its own limit.

**Every agent keeps its feed, including idle and done ones.** A finished agent's
last three steps are the answer to "what did it do while I was away", which is
most of why you look at a fleet at all. Discarding the feed the moment the work
finishes would throw away the summary exactly when it becomes useful.

A finished turn also gets its **outcome**, which the logs state rather than
imply: codex's `task_complete` carries `last_agent_message` and `duration_ms`,
cursor's `turn_ended` carries `status`, and claude's `stop_reason` distinguishes
`end_turn` from a turn that stopped for a tool. So a done row can read
`done · 4m · 145 passed, 0 failed` instead of `Done`.

No auto-collapsing. Rows do not shrink when they go idle and do not grow when
they wake; the sidebar's height is the user's business, and a layout that
rearranges itself while being read is worse than a long one. If height becomes a
real problem it is a separate decision, made against a real fleet.

The feed is a three-entry ring buffer per terminal held in the daemon, and it is
not written to the store. It survives a daemon restart anyway, because the
session log is still on disk: on startup the tail of each known log is read and
the feed rebuilt from it. That is better than persisting a copy — there is one
source of truth, and it is the file the agent itself wrote.

One parsing detail that bites: cursor wraps user text in a
`<timestamp>…</timestamp><user_query>…</user_query>` envelope, which has to be
unwrapped or every cursor pane reads as a date.

### What leaves the machine

The feed and the blocked question are conversation content, and both go into
push payloads so a lock screen can say `Run: cargo test?` rather than `codex
needs you`. That is the point of the feature, and it means fragments of the
conversation transit the relay.

Two rules keep that from being reckless.

**Only the derived line, never the source.** What travels is the forty-character
verb-and-object step and the question — never file contents, never a diff, never
a tool result, never the prompt. The parser reads the whole record and keeps the
named fields; everything else is dropped at the parse, not later.

**Redact before it leaves.** A command is exactly the kind of string that carries
a token — `curl -H "Authorization: Bearer …"`, an `AWS_SECRET…=`, a URL with a
key in the query. A redaction pass runs on any step or question before it is put
on the wire, replacing anything shaped like a credential with `…`. It runs on the
push path and the event path both, because the sidebar is only safer than the
lock screen by accident.

## What else a row could say

The session logs carry far more than state. What is actually available, measured:

| signal | claude | codex | cursor |
| --- | --- | --- | --- |
| turn start and end | yes | yes, with `duration_ms` | yes, with `status` |
| task summary | `aiTitle` | — | OSC title |
| current tool | `tool_use` | `tool_use` | `tool_use` + a written `description` |
| subagents running | `pendingBackgroundAgentCount`, `Agent` calls, `isSidechain` | — | — |
| tokens and context | per-message `usage` | `total_token_usage`, `model_context_window` | — |
| rate limit headroom | — | `rate_limits.used_percent` | — |
| model | yes | yes | — |
| mode | `permissionMode`, `effort` | `collaboration_mode_kind` | — |
| git branch | `gitBranch` | — | — |

Availability is not a reason to show something. A sidebar row is about forty
characters and a Live Activity is smaller; every field earns its place by
changing what the reader does. Ranked by that test:

1. **State and turn elapsed.** The core, and the reason for the two clocks.
2. **The last three steps, for an active agent.** The answer to "Working at
   what?", and the reason the row stops being opaque.
3. **The question, when blocked.** Turns "go and look" into "decide from here".
4. **Subagent count, only when above zero.** "3 agents" says the pane is a tree,
   not a line, which changes how long you expect to wait. Claude alone reports
   it, and a field only one agent can fill is worth having precisely because that
   agent is the one people run fleets of.
5. **Pressure, only near a threshold.** Codex's `rate_limits.used_percent` and
   context fill matter at 90% and are noise at 30%. Shown as a warning, never as
   a constant readout.

Deliberately not shown: token counts as a running number, cost, model name, and
git branch. The first two are noise a person cannot act on mid-turn, and the last
two are already known from the workspace the row sits in.

This section is a menu for the widget work, not a commitment. Stage 2 makes the
signals available; which ones reach a lock screen is a design question that
should be answered against a real Live Activity, not in advance.

## Built for a lock screen, rendered in a sidebar

The sidebar is the first consumer, not the only one. The work this design is
feeding is a Live Activity — a lock screen, a Dynamic Island, an Apple Watch —
where the job is overseeing a fleet from wherever you happen to be, in a space
that may be one line tall.

That constraint lands on this design in one specific way: **compact renderings
must be derived in the daemon, not by each client.** The Dynamic Island cannot
afford to truncate a forty-character string well, a watch complication cannot
re-derive "which of these six agents matters most", and a Mac that disagreed with
a phone about either would be the disagreement the whole derivation model exists
to prevent.

So the wire carries a ladder, each rung a strict narrowing of the one below, all
computed once:

| rung | budget | content | consumer |
| --- | --- | --- | --- |
| `glyph` | 1 char | the state | watch complication, Island minimal |
| `headline` | ~16 | `codex needs you`, `claude 4m` | Island compact, notification title |
| `line` | ~40 | the current step, or the question | lock screen, sidebar row |
| `feed` | 3 × ~40 | the last three steps | sidebar, Live Activity expanded |
| — | unbounded | the pane itself | the terminal |

A client picks the widest rung that fits and never composes its own. Adding a
surface later means picking a rung, not writing another truncator.

Two things follow that are worth stating before the widget exists, because they
are cheap now and expensive later.

**Rank, not just state.** A fleet view has to answer "which one first". Blocked
outranks done, done outranks working, and within blocked the oldest wins, because
the agent that has been stuck longest is the one costing you most. Computed in
the daemon beside the activity, so an Island showing one agent and a sidebar
showing twelve agree about which one that is.

**The question is the payload.** For every compact surface, the single most
valuable string is the blocked question — `Run: cargo test?` is actionable from a
lock screen in a way `codex needs you` is not. It is specified in stage 1 and
carried at the `line` rung for exactly this reason.

Building the Live Activity is a separate piece of work for a separate agent. What
this design owes it is that every signal it needs already exists, already agreed
across clients, at a size that fits.

## Panes that are not agents

A fleet is not only agents. A `pnpm dev`, a `cargo build`, a `node` REPL and a
plain shell all sit in the same sidebar and all have to fill the same rungs, or
the ladder has holes in it exactly where a build fails.

They have no session log, so the feed cannot come from one. But almost everything
that matters about them is a fact rather than an inference:

| signal | source | certainty |
| --- | --- | --- |
| what is running | argv, via the fixed `summarize` | fact |
| what it serves | a listening socket on the process group | fact, from the kernel |
| alive or finished | `pane_dead` | fact |
| succeeded or failed | the exit code | fact |
| how long it ran | when the foreground command changed | fact, from our own sampling |
| what it printed | the last output line | a guess — excluded |

**A non-agent feed already exists; it just is not being recorded.** The watcher
samples each pane's foreground process every tick, so it observes `cargo build`
becoming `cargo test` becoming `git status`. Keeping the last three of those,
with how long each ran and how it ended, gives a non-agent pane the same shape of
row an agent gets, from a source we already pay for:

```
  cargo build   ok      1m 40s
  cargo test    failed  exit 101   2m 12s
  git status    ok      0s
```

The honest limit: a command shorter than the sample interval may never be
observed, so the feed is "what this pane spent its time on" rather than a shell
history. That is the more useful of the two anyway, and it is what the sampling
can actually support.

### A failed command wants attention

`wants_attention` is `Blocked | Done` today, so a `cargo build` that fails at 3am
says nothing to anyone. For a design whose whole purpose is overseeing work while
away from the machine, that is a hole, and it is in the one direction that costs
real time: you find out in the morning.

So a terminal that exits non-zero becomes `Failed`, distinct from `Exited`, and
`wants_attention` includes it. `Exited` stays quiet — a shell you closed, or a
dev server you stopped, is not news. The distinction is the exit code, which is
already on the wire and already stored.

The rungs then fill for a non-agent as they do for an agent:

| rung | agent | non-agent |
| --- | --- | --- |
| `glyph` | working, blocked, done | running, ok, failed |
| `headline` | `codex needs you` | `cargo test failed` |
| `line` | the current step or question | `cargo test · exit 101 · 2m` |
| `feed` | last three steps | last three commands and outcomes |

A pane running an interactive program with no arguments — `node`, `psql`, a bare
shell — has genuinely little to say, and says little: its name and its state. Not
every row can be interesting, and inventing detail for one that is not is how a
sidebar becomes noise.

## Testing

**The corpus.** The 150 captures taken while diagnosing this go into
`crates/core/captures/`, each with its asserted classification. They are the
reason every claim in this document is a measurement.

The regression that started it gets a name: a finished, idle Claude pane whose
transcript explains what `esc to interrupt` means. It must classify as `Idle`.
Before this change it classifies as `Working`, and would forever.

**A property, not just examples.** No classification may depend on text above the
footer window. Testable directly: take any capture, prepend a thousand lines of
transcript containing every signature in the table, and assert the
classification is unchanged. That catches a future signature added carelessly to
the whole-screen path, which is how this bug was born.

**The clocks.** `Working → Blocked → Working` preserves `started_at` and resets
`state_since`. A process that exits does not reset either.

**Hysteresis.** One anomalous sample between two agreeing ones publishes
nothing. A first-sighting `Blocked` publishes immediately.

**The session logs.** A recorded transcript per agent, replayed record by record,
asserting the state machine it produces. A truncated final line — the normal case
while an agent is mid-write — parses as far as it can and never panics. An
unrecognized record type is ignored rather than fatal. Cursor's
`<timestamp>…<user_query>` envelope is unwrapped rather than shown.

**The feed.** Three steps and no more, oldest evicted. A step is truncated in the
daemon, so two clients cannot disagree about where the ellipsis goes. A finished
agent keeps its feed rather than clearing it, and a restarted daemon rebuilds it
from the tail of the session log rather than showing an empty row.

**Non-agent panes.** A command that exits non-zero is `Failed` and wants
attention; one that exits zero is `Exited` and does not. The command feed keeps
the last three foreground commands with their outcomes. A pane whose foreground
never changes has a feed of one, not an empty one.

**The rungs.** Each is a strict narrowing: whatever `headline` says is derivable
from `line`, and `glyph` from both. Asserted so a future surface cannot be handed
a `headline` that contradicts the `line` under it.

**Redaction, adversarially.** A table of real credential shapes — bearer tokens,
`AWS_SECRET_ACCESS_KEY=`, a URL with `?api_key=`, an `ssh` command with a
password — asserted to be redacted in both the event payload and the push
payload. The push path is tested separately rather than trusted to share the
event path, because "the sidebar is safer than the lock screen" is exactly the
assumption that goes stale first.

**The join.** Two panes in one cwd resolve to different sessions via the title;
a pane whose title matches nothing falls through to the layer below rather than
binding to the wrong session, because binding to the wrong session would report
one pane's work as another's.

**Naming.** Each precedence rung, and each of the four title rejections —
hostname, path, program name, bare harness name.

## What this does not do

It does not make an unknown agent first-class. Layer 4 gives it a reading and
nothing better, which is honest.

It does not read output to infer progress. That is the mistake being corrected.

It does not put the conversation on screen. The feed is three derived lines, and
the terminal remains the place you read what an agent actually said.

It does not touch the ACP path. Agent-mode panes already have layer 1, which is
better than anything below it, and this design changes what happens under it
rather than to it.
