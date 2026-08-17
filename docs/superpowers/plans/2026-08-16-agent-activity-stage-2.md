# Agent Activity — Stage 2: read what the agent already writes

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Take turn boundaries, timing, the running feed and the counts from each agent's own session log instead of from its screen — and leave the blocked state where it already works, because no agent writes it down.

**Architecture:** Four layers decide what a pane is doing, higher wins while fresh: the ACP protocol, then the session log, then the OSC title, then the footer-scoped screen. Stage 1 built layers 3 and 4. This stage builds layer 2 and the merge. Blocked never leaves layer 4.

**Tech Stack:** Rust 2021 workspace, `cargo` at `~/.cargo/bin/cargo` (not on `PATH`), `notify` for file watching, prost/protobuf wire, SwiftUI on the Mac.

## Read this first

`docs/agent-session-logs.md` is the observed schema for all three agents, written from every log on this machine. It is the reference for every parser task here. Do not re-derive it, and do not trust anything about these formats that is not in it — including anything in the design doc that predates it.

## Global Constraints

- **US English throughout**, in code and copy.
- **Never run `cargo fmt`.** Hand-formatted tree; CI skips `fmt --check` deliberately.
- **`cargo` is at `~/.cargo/bin/cargo`**, not on `PATH`.
- **Do not set `FARCOOLER_HOME`** when running daemon tests — it breaks `paths::tests`.
- **Apple copy conventions** for user-visible strings: title-case buttons, contractions, "machine" not "host", never a raw Rust error.
- **Comments explain why, not what.**
- **Never `pkill` by pattern.** A live Far Cooler app runs on this machine; kill scratch tmux servers by exact socket name.
- **Baseline:** 1091 passing across the workspace, 6 ignored (the live agent suite).

## The rule that governs every parser

A session log is a private format that will change without notice. Every parser here:

- **reads only the fields named in `docs/agent-session-logs.md`** and ignores everything else, so an added field breaks nothing;
- **returns `None` rather than erroring** when a record does not match — an unparseable log falls through to the layer below, which is the title, and then the screen;
- **trusts line order, never timestamps** (claude's go backwards in 85% of files);
- **caps the bytes it will hold for one line**, because claude writes lines over a megabyte.

## File structure

| file | responsibility |
| --- | --- |
| `crates/core/src/session_log/mod.rs` (create) | the `TurnEvent` vocabulary every parser emits, and the slug rules |
| `crates/core/src/session_log/claude.rs` (create) | claude's records → `TurnEvent` |
| `crates/core/src/session_log/codex.rs` (create) | codex's payloads → `TurnEvent` |
| `crates/core/src/session_log/cursor.rs` (create) | cursor's records → `TurnEvent` |
| `crates/core/src/session_log/tail.rs` (create) | incremental reads from a byte offset, safely |
| `crates/core/fixtures/session-logs/` (create) | recorded transcript fixtures, the corpus discipline extended |
| `crates/daemon/src/log_watch.rs` (create) | `notify` watches over the three roots |
| `crates/daemon/src/log_join.rs` (create) | which file belongs to which pane |
| `crates/daemon/src/watch.rs` (modify) | merge the log layer above the title layer |
| `crates/core/src/feed.rs` (create) | the three-step feed, and its ladder rungs |
| `proto/farcooler.proto` (modify) | feed steps, rung strings, rank |
| `apps/macos/Sources/FarCooler/*.swift` (modify) | render the feed |

---

### Task 1: The vocabulary, and the two slug rules

**Files:**
- Create: `crates/core/src/session_log/mod.rs`
- Modify: `crates/core/src/lib.rs` (add `pub mod session_log;`)

**Interfaces:**
- Produces:
  - `pub enum TurnEvent { Started { at_ms: Option<i64> }, Step { verb: String, object: String }, Ended { at_ms: Option<i64>, duration_ms: Option<i64>, outcome: TurnOutcome }, Title(String), BackgroundAgents(u32) }`
  - `pub enum TurnOutcome { Finished, Aborted, Failed }`
  - `pub fn claude_slug(cwd: &str) -> String`
  - `pub fn cursor_slug(cwd: &str) -> String`

- [ ] **Step 1: Write the failing tests**

The two slug rules differ, and the difference is the whole point — claude does not collapse runs, cursor does.

```rust
#[test]
fn claude_does_not_collapse_runs() {
    // A dot directory produces TWO hyphens, because claude replaces each
    // character separately. Observed across 35 real project directories.
    assert_eq!(claude_slug("/Users/e-liang/.unixconfig"), "-Users-e-liang--unixconfig");
    assert_eq!(
        claude_slug("/Users/e-liang/Dev/overnight/.claude/worktrees/agent-session-logs"),
        "-Users-e-liang-Dev-overnight--claude-worktrees-agent-session-logs"
    );
    assert_eq!(claude_slug("/private/tmp/claude-title-clean-r4x"), "-private-tmp-claude-title-clean-r4x");
}

#[test]
fn cursor_collapses_runs_and_trims() {
    // Cursor's own source: replace non-alphanumerics, collapse, trim.
    assert_eq!(cursor_slug("/Users/e-liang/.unixconfig"), "Users-e-liang-unixconfig");
    assert_eq!(cursor_slug("/Users/e-liang/Dev/Verdela"), "Users-e-liang-Dev-Verdela");
}

/// Cursor cuts a long path and signs it. Claude has no observed equivalent.
#[test]
fn a_long_path_is_cut_and_hashed_for_cursor_only() {
    let long = format!("/Users/e-liang/{}", "x".repeat(200));
    let slug = cursor_slug(&long);
    let joined = format!(".cursor/projects/{slug}");
    assert!(joined.len() <= 92, "cursor guards at 92 chars, got {}", joined.len());
    assert!(slug.len() > 8 && slug.contains('-'), "the hash suffix is kept: {slug}");
    // Claude is left alone: no truncation was ever observed, and inventing one
    // would send us looking in a directory that does not exist.
    assert!(claude_slug(&long).len() > 100);
}
```

- [ ] **Step 2: Run them and watch them fail**

Run: `~/.cargo/bin/cargo test -p farcooler-core session_log 2>&1 | tail -15`
Expected: FAIL — the module does not exist.

- [ ] **Step 3: Implement the module**

```rust
//! What an agent writes down as it works.
//!
//! Every parser here turns one agent's private JSONL into the same small
//! vocabulary, so the daemon does not learn three formats. See
//! `docs/agent-session-logs.md` for what each agent actually writes — observed,
//! not documented, and the only thing to trust about these formats.
//!
//! None of them records that the agent is WAITING for a human. That state stays
//! with the footer-scoped screen matching, which is the one place it can be
//! seen at all.

/// One thing that happened in a session.
///
/// Deliberately small. Three formats with nothing in common map onto this, and
/// anything richer would be one agent's vocabulary imposed on the others.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TurnEvent {
    /// The human asked for something.
    Started { at_ms: Option<i64> },
    /// The agent did something worth naming in a row.
    Step { verb: String, object: String },
    /// The turn is over, however it went.
    Ended { at_ms: Option<i64>, duration_ms: Option<i64>, outcome: TurnOutcome },
    /// The agent's own name for this session's work.
    Title(String),
    /// How many background agents are still running.
    BackgroundAgents(u32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnOutcome {
    Finished,
    Aborted,
    Failed,
}

/// Claude's project directory name for a working directory.
///
/// Each character is replaced on its own and runs are NOT collapsed, so
/// `/.claude` becomes `--claude`. That double hyphen is real: it is how the
/// directories on disk are named, and collapsing would send a reader to a path
/// that does not exist. Cursor's rule is different — see `cursor_slug`.
pub fn claude_slug(cwd: &str) -> String {
    cwd.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '-' }).collect()
}

/// Cursor's project directory name for a working directory.
///
/// From cursor's own shipped source: replace every non-alphanumeric, collapse
/// runs, trim the ends, and if the joined path would exceed 92 characters cut
/// the slug to 84 and append `-` plus seven hex of the full path's sha256.
pub fn cursor_slug(cwd: &str) -> String {
    let mut out = String::with_capacity(cwd.len());
    for c in cwd.chars() {
        let c = if c.is_ascii_alphanumeric() { c } else { '-' };
        if c == '-' && out.ends_with('-') {
            continue;
        }
        out.push(c);
    }
    let out = out.trim_matches('-').to_string();
    // 92 is cursor's own bound on `.cursor/projects/<slug>`, so the budget for
    // the slug is what is left after that prefix.
    const JOINED_MAX: usize = 92;
    const PREFIX: usize = ".cursor/projects/".len();
    if PREFIX + out.len() <= JOINED_MAX {
        return out;
    }
    let keep = 84usize.saturating_sub(PREFIX).min(out.len());
    format!("{}-{}", &out[..keep], short_hash(cwd))
}
```

`short_hash` needs a sha256. Check whether the workspace already has one (`grep -rn "sha2\|Sha256" crates/*/Cargo.toml`). If it does, use it. If it does not, do NOT add a crypto dependency for a directory name: implement a small FNV-1a hex digest instead and say in a comment that the exact digest does not need to match cursor's, because Far Cooler only ever reads directories it can also list — it can find the truncated directory by prefix instead of reproducing the hash. Prefer the listing approach if it is simpler; state which you chose and why.

- [ ] **Step 4: Run the tests**

Run: `~/.cargo/bin/cargo test -p farcooler-core session_log 2>&1 | tail -15`
Expected: PASS. Adjust the long-path assertion to whatever your chosen approach guarantees, but do not weaken the claude/cursor difference.

- [ ] **Step 5: Commit**

```bash
git add crates/core/src/session_log/mod.rs crates/core/src/lib.rs
git commit -m "feat: one vocabulary for three private log formats

Three agents write three unrelated JSONL shapes and the daemon should learn
none of them. TurnEvent is what they all become.

The two slug rules differ and the difference is load-bearing: claude replaces
each character separately so /.claude becomes --claude, while cursor collapses
runs and truncates past 92 characters with a hashed suffix. Collapsing
claude's would send a reader to a directory that is not there.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_011A27xRp5hrevabCQpUYbZB"
```

---

### Task 2: Recorded fixtures, the corpus discipline extended

Stage 1's screen captures are why its rules are trustworthy. The same argument applies here, and fixtures must exist before the parsers so the parsers are written against real bytes.

**Files:**
- Create: `crates/core/fixtures/session-logs/{claude,codex,cursor}-*.jsonl`
- Create: `crates/core/fixtures/session-logs/README.md`

- [ ] **Step 1: Take the fixtures from real logs on this machine**

For each agent, extract a SHORT excerpt — tens of lines, not megabytes — that contains a complete turn:

- **claude**: a `user` record with `promptId`/`promptSource`, the assistant records including at least one `tool_use`, the assistant record with `stop_reason == "end_turn"`, and the closing `system`/`turn_duration`. Take it from any file under `~/.claude/projects/`.
- **codex**: `session_meta`, a `task_started`, a `token_count`, an `agent_message`, and the matching `task_complete`. From `~/.codex/sessions/`.
- **cursor**: an entire transcript — they are tiny. From `~/.cursor/projects/*/agent-transcripts/*/`.

Also take **one adversarial fixture per agent**:
- `claude-out-of-order-timestamps.jsonl` — real adjacent lines whose timestamps go backwards (235 of 276 files have them; find one).
- `codex-unmatched-task-started.jsonl` — a `task_started` with no completion (5 of 183 files end this way).
- `cursor-turn-ended-error.jsonl` — a `turn_ended` with `status: "error"` if one exists; if none does, say so in the README rather than fabricating one.

**Redact before committing.** These are real transcripts. Replace absolute home paths with `/Users/example`, and remove any message text that is not needed to exercise the parser — the fixtures are for record SHAPE, not content. Do not commit anything you have not read.

- [ ] **Step 2: Write the README**

Record for each fixture: which agent, which CLI version it came from, what shape it demonstrates, and what was redacted. Say plainly that cursor's fixtures come from four files totaling under 2 KB, so its coverage is thinner than the others by necessity.

- [ ] **Step 3: Commit**

```bash
git add crates/core/fixtures/session-logs
git commit -m "test: real session-log excerpts, redacted

Stage 1's screen captures are why its rules can be trusted; the same argument
applies to a private JSONL format that changes without notice. These are real
records, cut to the shape a parser needs, with paths and message text removed.

Including the awkward ones on purpose: timestamps that go backwards, a codex
turn that never completes. Both are common in the wild and neither is
something anyone would think to invent.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_011A27xRp5hrevabCQpUYbZB"
```

---

### Task 3: The claude parser

**Files:**
- Create: `crates/core/src/session_log/claude.rs`
- Modify: `crates/core/src/session_log/mod.rs` (add `pub mod claude;`)

**Interfaces:**
- Consumes: `TurnEvent`, `TurnOutcome` (Task 1); fixtures (Task 2).
- Produces: `pub fn parse_line(line: &str) -> Vec<TurnEvent>` — a collection, NOT an option, because one line can report two facts. A claude `turn_duration` carrying `pendingBackgroundAgentCount` ends the turn AND says how many agents are still running; forcing a choice between them dropped the `Ended` and left the turn open forever. An unrecognized or non-JSON line returns an empty vec and never errors.

- [ ] **Step 1: Write the failing tests, against the fixtures**

Cover, with `include_str!` over the Task 2 fixtures:

- a `user` record with `promptId` and `promptSource: "typed"` yields `Started`;
- a `user` record that is a **tool result** (has `message.content[].type == "tool_result"`, no `promptSource`) yields `None` — this is the distinction that decides whether every tool call looks like a new turn;
- a `system`/`turn_duration` record yields `Ended` with `duration_ms` from `durationMs` and `TurnOutcome::Finished`;
- a `turn_duration` carrying `pendingBackgroundAgentCount: 3` also yields `BackgroundAgents(3)`, and one WITHOUT the key yields no `BackgroundAgents` event at all — the key is omitted rather than zero, and reading a missing key as zero would be wrong in the same way for every idle session;
- an `assistant` record with a `tool_use` block yields `Step`, with the verb from the tool name lowercased and the object from `file_path`'s basename, `command`, `pattern` or `description` — whichever is present first;
- an `ai-title` record yields `Title`;
- a line that is not JSON at all yields `None` and does not panic;
- a record of a type not listed in `docs/agent-session-logs.md` yields `None`.

- [ ] **Step 2: Run them and watch them fail**

Run: `~/.cargo/bin/cargo test -p farcooler-core session_log::claude 2>&1 | tail -15`

- [ ] **Step 3: Implement**

Read only the named fields. Use `serde_json::Value` rather than deriving structs: a derived struct with `deny_unknown_fields` would break on the next CLI release, and one without it is no simpler than reading the fields directly.

- [ ] **Step 4: Run the tests, then commit**

```bash
git add crates/core/src/session_log/claude.rs crates/core/src/session_log/mod.rs
git commit -m "feat: read claude's transcript for turn boundaries

The turn end is explicit -- system/turn_duration, always written, carrying
durationMs -- which is better than anything the screen could tell us. The turn
START needs care: a tool result is also a 'user' record, and only the presence
of promptSource tells them apart. Without that every tool call would look like
a new turn.

pendingBackgroundAgentCount is absent rather than zero when nothing is
running, so a missing key is not a count of none.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_011A27xRp5hrevabCQpUYbZB"
```

---

### Task 4: The codex parser

**Files:**
- Create: `crates/core/src/session_log/codex.rs`
- Modify: `crates/core/src/session_log/mod.rs`

**Interfaces:**
- Produces: `pub fn parse_line(line: &str) -> Vec<TurnEvent>` — a collection, NOT an option, because one line can report two facts. A claude `turn_duration` carrying `pendingBackgroundAgentCount` ends the turn AND says how many agents are still running; forcing a choice between them dropped the `Ended` and left the turn open forever. An unrecognized or non-JSON line returns an empty vec and never errors.

- [ ] **Step 1: Write the failing tests**

- `task_started` yields `Started` with `at_ms` — and the test must assert the **unit conversion**: `started_at` is unix SECONDS, so `at_ms` must be `started_at * 1000`. Use a real value from the fixture and assert the millisecond result explicitly. This is the trap the reference document names.
- `task_complete` yields `Ended` with `duration_ms` taken from `duration_ms` directly (already milliseconds — do NOT multiply) and `TurnOutcome::Finished`.
- `turn_aborted` yields `Ended` with `TurnOutcome::Aborted`.
- `agent_message` with `phase: "final_answer"` yields a `Step` with verb `says`.
- a `token_count` record yields `None` for now — it is read in Task 10, and inventing an event here would be a field nobody consumes.
- a malformed line yields `None`.

- [ ] **Step 2: Run them and watch them fail**

- [ ] **Step 3: Implement**

- [ ] **Step 4: Run the tests, then commit**

```bash
git add crates/core/src/session_log/codex.rs crates/core/src/session_log/mod.rs
git commit -m "feat: read codex's rollout, which states the turn outright

task_started and task_complete carry a matching turn_id and a duration_ms.
That is the turn clock exactly, with no inference left in it.

Two traps, both observed and both pinned by tests: started_at and completed_at
are unix SECONDS while duration_ms in the same payload is milliseconds, and a
turn can end with turn_aborted instead of completing at all.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_011A27xRp5hrevabCQpUYbZB"
```

---

### Task 5: The cursor parser

**Files:**
- Create: `crates/core/src/session_log/cursor.rs`
- Modify: `crates/core/src/session_log/mod.rs`

**Interfaces:**
- Produces: `pub fn parse_line(line: &str) -> Vec<TurnEvent>` — a collection, NOT an option, because one line can report two facts. A claude `turn_duration` carrying `pendingBackgroundAgentCount` ends the turn AND says how many agents are still running; forcing a choice between them dropped the `Ended` and left the turn open forever. An unrecognized or non-JSON line returns an empty vec and never errors.

**Say this in the module doc:** cursor's format is evidenced by four files totaling under 2 KB, where claude and codex have hundreds. Everything here is written to fail soft, because the sample is too small to be confident about anything it did not contain.

- [ ] **Step 1: Write the failing tests**

- a `role: "user"` record yields `Started`, and its text has the
  `<timestamp>…</timestamp><user_query>…</user_query>` envelope **unwrapped** — assert the extracted text is the bare query, since a row showing a date instead of the request would be the visible symptom.
- `{"type":"turn_ended","status":"success"}` yields `Ended` with `TurnOutcome::Finished`; `status: "error"` yields `TurnOutcome::Failed`.
- a `tool_use` block named `Shell` yields a `Step` whose object prefers `description` over `command` — cursor writes a human sentence there and it reads better in a row than a shell line.
- a `tool_use` with a name other than `Shell` still yields a `Step` rather than `None`, degrading to `command` or the name itself. Only `Shell` was ever observed, so anything else must not be treated as an error.
- a malformed line yields `None`.

- [ ] **Step 2: Run them and watch them fail**

- [ ] **Step 3: Implement**

- [ ] **Step 4: Run the tests, then commit**

```bash
git add crates/core/src/session_log/cursor.rs crates/core/src/session_log/mod.rs
git commit -m "feat: read cursor's transcript, on much thinner evidence

Cursor closes a turn outright with turn_ended and gives each tool call a
human-written description, which reads better in a row than the command does.

Written to fail soft throughout, and the module says why: four transcript
files totaling under two kilobytes exist on the machine this was built
against, where claude and codex have hundreds each. Only one tool name was
ever seen, so anything else degrades rather than erroring.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_011A27xRp5hrevabCQpUYbZB"
```

---

### Task 6: Tail a growing file safely

**Files:**
- Create: `crates/core/src/session_log/tail.rs`
- Modify: `crates/core/src/session_log/mod.rs`

**Interfaces:**
- Produces:
  - `pub struct Tail { path: PathBuf, offset: u64 }`
  - `pub fn new(path: PathBuf) -> Tail` — starts at the END of the file, so attaching to a 100 MB log does not replay a day of history
  - `pub fn read_new_lines(&mut self) -> Vec<String>`

- [ ] **Step 1: Write the failing tests**

- appending two lines yields both, and reading again yields none;
- a partial final line (no trailing newline) is NOT returned, and IS returned once its newline arrives — a half-written record must never be parsed;
- a line longer than the cap is skipped rather than held, and the tail stays correctly positioned afterwards. Claude writes lines over a megabyte, and a reader that buffers one per pane per tick is a memory problem the user never asked for;
- a file that shrinks (truncated or replaced) resets the offset to zero rather than reading garbage from the middle of a record;
- a file that does not exist yields nothing and does not error.

- [ ] **Step 2: Run them and watch them fail**

- [ ] **Step 3: Implement**

Cap a single line at 64 KiB. Anything longer is a tool result carrying a file dump; the fields this stage reads are never that big, so skipping is lossless in practice and bounded in the worst case.

- [ ] **Step 4: Run the tests, then commit**

---

### Task 7: Which file belongs to which pane

**Files:**
- Create: `crates/daemon/src/log_join.rs`
- Modify: `crates/daemon/src/lib.rs`

**Interfaces:**
- Consumes: `claude_slug`, `cursor_slug` (Task 1).
- Produces: `pub fn find_session_log(preset: &str, pid: Option<i32>, cwd: &str, title: &str) -> Option<PathBuf>`

- [ ] **Step 1: Write the failing tests**

Against a temp directory standing in for each root:

- **codex**: given a pid whose open files include `~/.codex/sessions/2026/08/16/rollout-x.jsonl` AND an unrelated `/tmp/superset-codex-session-1.jsonl`, the rollout is chosen. The decoy is the case that matters: a wrapped codex holds both open;
- **codex** with no rollout open yields `None` — the file does not exist until the first turn is submitted, and a freshly started pane legitimately has nothing;
- **claude**: given two session files in one slugged directory, the one whose `aiTitle` matches the pane title is chosen;
- **claude**: with only one file in the directory, it is chosen without needing the title;
- **claude**: the pane title's leading activity glyph is stripped before comparing (`✳ Write a haiku` must match `aiTitle: "Write a haiku"`);
- an unknown preset yields `None`.

- [ ] **Step 2: Run them and watch them fail**

- [ ] **Step 3: Implement**

Take the lsof call from `ports::listening_ports`'s shape — one process spawn, failure degrades to empty rather than erroring.

- [ ] **Step 4: Run the tests, then commit**

---

### Task 8: Watch the three roots

**Files:**
- Create: `crates/daemon/src/log_watch.rs`
- Modify: `crates/daemon/Cargo.toml` (add `notify`), `crates/daemon/src/lib.rs`

**Interfaces:**
- Produces: `pub struct LogWatcher` with `pub fn start(roots: Vec<PathBuf>) -> LogWatcher` and `pub fn drain(&self) -> Vec<PathBuf>` — the paths that changed since the last drain.

- [ ] **Step 1: Check the dependency before adding it**

Run `grep -rn "notify" Cargo.lock | head -3`. If `notify` is already in the tree, use that version via the workspace. Report which you did.

- [ ] **Step 2: Write the failing tests**

- writing to a file under a watched root makes its path appear in the next `drain`;
- `drain` is idempotent — a second call with no further writes returns nothing;
- a root that does not exist is not an error. `~/.cursor/projects` legitimately does not exist on a machine that has never run cursor, and a daemon that refused to start over that would be broken for most users;
- events coalesce: ten writes to one file yield that path once, not ten times.

- [ ] **Step 3: Run them and watch them fail**

- [ ] **Step 4: Implement**

Recursive watches on the three roots. Debounce with a short window so a burst of appends is one wake-up.

- [ ] **Step 5: Run the tests, then commit**

---

### Task 9: Merge the log layer above the title

**Files:**
- Modify: `crates/daemon/src/watch.rs`

**Interfaces:**
- Consumes: everything above.
- Produces: no new public API. Behavior only.

**This is the task where the stage's central constraint lives.** The log decides Working and Idle and owns the turn clock. It NEVER decides Blocked. The screen keeps that, because no agent writes it down.

- [ ] **Step 1: Write the failing tests**

Put the decision behind a named function the loop calls, the way `should_announce` and `promoted_by_title` already are, and test that function directly:

- a log saying the turn started, with a screen saying `Idle`, yields `Working` — the log outranks the screen;
- a log saying the turn ended, with a screen saying `Working` (a stale footer), yields the fold that produces `Done`;
- **a screen saying `Blocked` beats the log saying the turn is running.** The log cannot see a permission prompt; if this test fails the stage has broken the one state it must not touch;
- a log with no events for a pane falls through to the title, and then to the screen, unchanged from stage 1;
- a log whose last event is older than a staleness bound stops being trusted, so an agent whose log stopped being written does not sit on Working forever. This is the same trap the title's staleness bound closed.

- [ ] **Step 2: Run them and watch them fail**

- [ ] **Step 3: Implement**

- [ ] **Step 4: Run the whole daemon suite, then commit**

---

### Task 10: The feed

**Files:**
- Create: `crates/core/src/feed.rs`
- Modify: `proto/farcooler.proto`, `crates/daemon/src/watch.rs`, `crates/daemon/src/wire.rs`, `crates/daemon/src/rpc.rs`, `crates/cli/src/main.rs`

**Interfaces:**
- Produces: `pub struct Feed` holding three `Step`s, `pub fn push(&mut self, verb: &str, object: &str)`.

- [ ] **Step 1: Write the failing tests**

- three steps in, three out, oldest evicted on the fourth;
- each step is truncated in the daemon so two clients cannot disagree about where the ellipsis goes;
- **every step passes through `redact`** — a planted `Authorization: Bearer` in a tool argument must not survive. Stage 1 learned this the hard way on the blocked question;
- a finished agent KEEPS its feed rather than clearing it, because "what did it do while I was away" is when the summary is worth most.

- [ ] **Step 2: Add the proto fields**

Next free numbers on `Terminal`. Repeated feed steps, and the rung strings from Task 11 if you take them together.

- [ ] **Step 3: Implement, and plumb every read path**

**All four paths, and this is the bug that recurred three times in stage 1:** the watcher broadcast, the RPC read, `terminal_event_json`, and `workspace_list_terminal_json`. The parity test added in stage 1 (`the_two_terminal_projections_agree_on_every_field`) will catch the last two — run it.

- [ ] **Step 4: Run the workspace suite, then commit**

---

### Task 11: The compact-rendering ladder and ranking

**Files:**
- Create or modify: `crates/core/src/feed.rs`, `proto/farcooler.proto`, `crates/daemon/src/wire.rs`

**Interfaces:**
- Produces: `pub fn glyph(...) -> char`, `pub fn headline(...) -> String`, `pub fn line(...) -> String`, `pub fn rank(...) -> u32`

**Why this exists:** the destination is a Live Activity — a lock screen, a Dynamic Island, a watch face. A Dynamic Island cannot truncate a forty-character string well and a watch complication cannot re-derive which of six agents matters most. Both are decided here, once, so every surface agrees.

- [ ] **Step 1: Write the failing tests**

- each rung is a strict narrowing: whatever `headline` says is derivable from `line`, and `glyph` from both;
- `rank` puts blocked above done above working, and within blocked the oldest first — the agent stuck longest is costing the most;
- a non-agent pane fills the same rungs from its command and exit status.

- [ ] **Step 2–4: Fail, implement, commit**

---

### Task 12: Show it on the Mac

**Files:**
- Modify: `apps/macos/Sources/FarCooler/Model.swift`, `SidebarViews.swift`, `EventStream.swift`, `DaemonClient.swift`

- [ ] **Step 1: Decode the new fields on BOTH paths**

`TerminalEvent` and the full-list `Terminal`. A field decoded on one and not the other is the stage 1 bug in its Swift form.

- [ ] **Step 2: Render the feed**

Three lines under an active agent's row, in the register `displayDuration` already uses. Idle and finished agents keep theirs — the user asked for that explicitly, so they can see what an agent did while they were away. No auto-collapsing.

- [ ] **Step 3: Build the Swift for real**

Run `cd apps/macos && swift build`. Do not claim a build you did not run; `cargo test` does not cover this.

- [ ] **Step 4: Commit**

---

## Verification

- [ ] **Whole suite green**

`~/.cargo/bin/cargo test --workspace` — at or above 1091 passing.

- [ ] **The live suite still passes**

`cargo test -p farcooler-core --test live_agents -- --ignored --nocapture` — all three agents, which now exercises the log path as well as the screen.

- [ ] **End to end, by hand**

Drive a real claude in a scratch tmux server. Confirm the row's turn clock comes from the log rather than the screen — the simplest proof is that it survives a permission prompt, since the log's turn never ended.

- [ ] **The state that must not regress**

Force a permission prompt and confirm the row still says the agent needs you, with its question. That is screen-derived and this stage must not have touched it.
