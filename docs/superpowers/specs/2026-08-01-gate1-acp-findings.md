# Gate 1 findings: does the ACP adapter map to a resumable Claude session?

**Verdict: PASS on all four conditions.** The design is not killed. `npx -y @zed-industries/claude-code-acp` (v0.16.2, now renamed upstream to `@agentclientprotocol/claude-agent-acp`) produces an ACP `sessionId` that is a literal Claude Code session id: it names a real `.jsonl` transcript under `~/.claude/projects/<munged-cwd>/`, and `claude --resume <sessionId>` opens it.

Two corrections to the plan's assumptions are recorded below because they are load-bearing for Task 3 and Task 7: the **munged-cwd directory name** (condition 4) and the **location of `availableModes`/`currentModeId`** in the wire protocol (used by Task 7's `AgentSession::start`).

## Environment note (not a gate finding, but required to reproduce)

This investigation ran inside a nested Claude Code agent session (`CLAUDECODE=1` in the environment). The Claude Agent SDK that the adapter shells out to refuses to launch when it detects that variable:

```
Error: Claude Code cannot be launched inside another Claude Code session.
Nested sessions share runtime resources and will crash all active sessions.
To bypass this check, unset the CLAUDECODE environment variable.
```

Every adapter/`claude` invocation below was run with `CLAUDECODE`, `CLAUDE_CODE_ENTRYPOINT`, `CLAUDE_CODE_SESSION_ID`, `CLAUDE_CODE_CHILD_SESSION`, `CLAUDE_PID`, and `CLAUDE_CODE_EXECPATH` unset (`env -u ...`). This will not affect the real shim, which runs as a tmux pane process, not nested inside another Claude Code CLI invocation — but anyone re-running this spike from inside an agent session needs the same workaround.

The plan's `drive.mjs` also has a sequencing bug: it sets `SID` via a shell env var read at process-start, but `session/new`'s result (which contains the id) is only known *after* the process is already running, so a naive two-run choreography can't actually thread the id through. The driver was rewritten to chain requests off their real JSON-RPC responses (a `Map<id, {resolve,reject}>`) instead of fixed `setTimeout` delays, keeping the same message shapes and stdio-framing as the plan's version. This is what actually produced the captures below.

## Condition (1): writes arrive via `fs/write_text_file` — PASS

`grep -c '"fs/write_text_file"' capture.jsonl` → **1**

Evidence line (from `/tmp/gate1/capture.jsonl`, prompt: "Add a doc comment to main.rs"):

```json
{"dir":"in","msg":{"jsonrpc":"2.0","id":3,"method":"fs/write_text_file","params":{"sessionId":"8819cbd3-2eb4-4572-bc39-09ca935ac409","path":"/private/tmp/gate1/main.rs","content":"//! The main entry point for the application.\n\nfn main() {}\n"}}}
```

## Condition (2): permission requests arrive via `session/request_permission` — PASS

`grep -c '"session/request_permission"' capture.jsonl` → **1**

Evidence line (same run — the edit itself required an approval before the write above; "Default" mode's description is literally "Standard behavior, prompts for dangerous operations"):

```json
{"dir":"in","msg":{"jsonrpc":"2.0","id":1,"method":"session/request_permission","params":{"options":[{"kind":"allow_always","name":"Always Allow","optionId":"allow_always"},{"kind":"allow_once","name":"Allow","optionId":"allow"},{"kind":"reject_once","name":"Reject","optionId":"reject"}],"sessionId":"8819cbd3-2eb4-4572-bc39-09ca935ac409","toolCall":{"toolCallId":"toolu_01BLm3pzQXLeRFoND7FNhPHy","rawInput":{"file_path":"/private/tmp/gate1/main.rs","old_string":"fn main() {}","new_string":"//! The main entry point for the application.\n\nfn main() {}"},"title":"Edit `/private/tmp/gate1/main.rs`"}}}}
```

A second scenario (shell command) was captured to exercise a different tool `kind` (`execute` instead of `edit`) for `session_permission.jsonl`. `ls -la` alone did **not** trigger a permission prompt (Claude Code appears to auto-allow read-only shell commands even in Default mode); a mutating command did:

```json
{"dir":"in","msg":{"jsonrpc":"2.0","id":0,"method":"session/request_permission","params":{"options":[{"kind":"allow_always","name":"Always Allow","optionId":"allow_always"},{"kind":"allow_once","name":"Allow","optionId":"allow"},{"kind":"reject_once","name":"Reject","optionId":"reject"}],"sessionId":"79f10ad4-36c6-4d36-80ba-40f6e5528e17","toolCall":{"toolCallId":"toolu_01S4XxgGKc1qsPHwRkG4ycXo","rawInput":{"command":"touch /tmp/gate1/scratch_delete_me.txt","description":"Create the scratch file"},"title":"`touch /tmp/gate1/scratch_delete_me.txt`"}}}}
```

## Condition (3): `session/load` replays — PASS

A second adapter process called `session/load` with the `sessionId` recorded from the first run instead of `session/new`. It returned a normal RPC result (not an error) and replayed the entire prior turn as `session/update` notifications before returning:

Request (`/tmp/gate1/capture_load.jsonl` line 3):
```json
{"dir":"out","msg":{"jsonrpc":"2.0","id":2,"method":"session/load","params":{"sessionId":"8819cbd3-2eb4-4572-bc39-09ca935ac409","cwd":"/tmp/gate1","mcpServers":[]}}}
```

First replayed frame (line 4) — the original user prompt, replayed as history rather than re-executed:
```json
{"dir":"in","msg":{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"8819cbd3-2eb4-4572-bc39-09ca935ac409","update":{"sessionUpdate":"user_message_chunk","content":{"type":"text","text":"Add a doc comment to main.rs"}}}}}
```

12 `session/update` notifications were replayed in total (user/agent message chunks, the `Read` tool call and its update, the `Edit` tool call and its update), followed by the `session/load` RPC result:
```json
{"dir":"in","msg":{"jsonrpc":"2.0","id":2,"result":{"modes":{"currentModeId":"default","availableModes":[...]},"models":{"availableModels":[...],"currentModelId":"haiku"}}}}
```

Bonus confirmation: between the two capture runs, `claude --resume <sessionId> -p "say only: RESUMED"` was executed directly against the same id (see condition 4). The `session/load` replay above *also* replayed that later "say only: RESUMED" turn (`user_message_chunk` → `agent_thought_chunk` → `agent_message_chunk: "RESUMED"`), proving ACP's `session/load` and the native `claude --resume` CLI read the exact same transcript file, not two independent stores.

## Condition (4): the ACP `sessionId` names a file `claude --resume` accepts — PASS, with a correction

`session/new`'s result contained `sessionId: "8819cbd3-2eb4-4572-bc39-09ca935ac409"`.

**Correction to the plan:** the plan's check (`ls ~/.claude/projects/-tmp-gate1/`) assumes the munged directory is a literal string-substitution of the `cwd` passed to `session/new` (`/tmp/gate1` → `-tmp-gate1`). That directory **does not exist**. On macOS, `/tmp` is a symlink to `/private/tmp`, and Claude Code munges the *resolved* (realpath) cwd, not the string it was given — confirmed independently by the `tool_call` `locations` field showing `/private/tmp/gate1/main.rs` even though `session/new` was called with `cwd: "/tmp/gate1"` verbatim. The real directory is:

```
$ ls -la ~/.claude/projects/-private-tmp-gate1/
-rw------- ... 8819cbd3-2eb4-4572-bc39-09ca935ac409.jsonl
```

The session file exists exactly where expected once the realpath is used. The decisive check:

```
$ claude --resume 8819cbd3-2eb4-4572-bc39-09ca935ac409 -p "say only: RESUMED" --output-format text
RESUMED
```

`claude --resume` accepted the ACP-issued id directly and continued the exact conversation (it had full context — it did not re-explain what main.rs was). No id-recovery fallback (watching the directory for a newly created file) is needed; the id is usable as-is. The one thing a real implementation must get right is resolving the worktree path (`fs::canonicalize` / realpath) before deriving the munged directory name, for any code that needs to *locate* the transcript file directly rather than just handing the id to `claude --resume`.

## Observed wire field names (for Task 3)

These are the actual field names seen in `capture.jsonl`, `capture_load.jsonl`, and `capture_permission.jsonl`, checked against the plan's `wire.rs`/`session.rs` guesses.

### `initialize` result — structure differs from the plan's assumption

```json
{"protocolVersion":1,"agentCapabilities":{"promptCapabilities":{"image":true,"embeddedContext":true},"mcpCapabilities":{"http":true,"sse":true},"loadSession":true,"sessionCapabilities":{"fork":{},"list":{},"resume":{}}},"agentInfo":{"name":"@zed-industries/claude-code-acp","title":"Claude Code","version":"0.16.2"},"authMethods":[{"description":"Run `claude /login` in the terminal","name":"Log in with Claude Code","id":"claude-login"}]}
```

- `agentCapabilities.loadSession` — matches the plan's `init["agentCapabilities"]["loadSession"]` lookup in Task 7. Good.
- **`agentCapabilities.availableModes` and `agentCapabilities.currentModeId` do not exist at `initialize` time.** Task 7's `AgentSession::start` reads these from `init[...]` — that field does not exist in the real adapter and will always be empty/`None`. Mode and model info instead arrives in the **`session/new` (and `session/load`) result**, under top-level `modes` and `models` keys:

```json
{"sessionId":"...","models":{"availableModels":[{"modelId":"default","name":"Default (recommended)","description":"Opus 4.6 · Most capable for complex work"},{"modelId":"sonnet","...":"..."},{"modelId":"haiku","...":"..."}],"currentModelId":"haiku"},"modes":{"currentModeId":"default","availableModes":[{"id":"default","name":"Default","description":"Standard behavior, prompts for dangerous operations"},{"id":"acceptEdits","name":"Accept Edits","description":"Auto-accept file edit operations"},{"id":"plan","name":"Plan Mode","description":"Planning mode, no actual tool execution"},{"id":"dontAsk","name":"Don't Ask","description":"Don't prompt for permissions, deny if not pre-approved"},{"id":"bypassPermissions","name":"Bypass Permissions","description":"Bypass all permission checks"}]}}
```

  Task 7's `AgentSession::start` should read `available_modes`/`agent_mode` from the `session/new`/`session/load` result (`result["modes"]["availableModes"]` / `result["modes"]["currentModeId"]`), not from `initialize`'s result.

### `session/update` — `sessionUpdate` tag values actually observed

All snake_case, matching the plan's `#[serde(tag = "sessionUpdate", rename_all = "snake_case")]` convention:

- `user_message_chunk` — matches plan's `UserMessageChunk`.
- `agent_message_chunk` — matches plan's `AgentMessageChunk`.
- `agent_thought_chunk` — matches plan's `AgentThoughtChunk` (seen only in the replayed "say only: RESUMED" turn).
- `tool_call` — matches plan's `ToolCall`, but carries more fields than the plan's `wire.rs` captures (see below).
- `tool_call_update` — matches plan's `ToolCallUpdate`, same caveat.
- **`available_commands_update` — not modeled in the plan's `SessionUpdate` enum at all.** It fires once per turn and, in this environment, carries the full list of every slash-command/skill available to the host Claude Code install (60+ entries, tens of KB). It will currently fall into `#[serde(other)] Unknown` and become `AgentEvent::Gap { reason: Unparsed }` under Task 3's normalizer as written — which is *correct* per the "never silently drop" rule, but worth a deliberate decision: gap-and-ignore is probably right (it's session metadata, not conversation content), but the size of this payload makes it worth an explicit variant so it doesn't get misread as a meaningful gap by a future implementer.
- `plan` and `current_mode_update` (from the plan's guess) were **not observed** in any of the three captures — no run in this spike exercised plan mode or a mode switch. They remain unverified; keep them in `wire.rs` as best-effort guesses but do not treat their absence from the fixtures as validation.

`ContentBlock`: observed as `{"type":"text","text":"..."}`. The plan's `ContentBlock { text: String }` (ignoring `type`) works fine — serde ignores unknown fields by default.

### `tool_call` — extra fields beyond the plan's `wire.rs`

Observed (`kind` values seen: `"read"`, `"edit"`, `"execute"`; `status` values seen: `"pending"`, `"completed"`):

```json
{"_meta":{"claudeCode":{"toolName":"mcp__acp__Edit"}},"toolCallId":"toolu_...","rawInput":{"file_path":"...","old_string":"...","new_string":"..."},"status":"pending","title":"Edit `/private/tmp/gate1/main.rs`","kind":"edit","content":[{"type":"diff","path":"...","oldText":"...","newText":"..."}],"locations":[{"path":"...","line":1}]}
```

Fields the plan's `wire.rs::SessionUpdate::ToolCall` doesn't capture: `_meta.claudeCode.toolName` (the underlying MCP tool name, e.g. `mcp__acp__Edit`, `mcp__acp__Read`, `Bash`), `rawInput` (the tool's raw arguments), and — most notably — **`content`, which for an edit carries a `{"type":"diff","path":...,"oldText":...,"newText":...}` block directly on the wire.** `locations` entries also carry a `line` field the plan's `Location { path }` ignores (harmless, serde default-ignores it).

### `tool_call_update` — the native diff surface

```json
{"_meta":{"claudeCode":{"toolResponse":[...],"toolName":"mcp__acp__Edit"}},"toolCallId":"toolu_...","sessionUpdate":"tool_call_update","status":"completed","rawOutput":[{"type":"text","text":"Index: ...\n@@ ...\n+//! The main entry point...\n"}],"content":[{"type":"diff","path":"/private/tmp/gate1/main.rs","oldText":"fn main() {}","newText":"//! The main entry point for the application.\n\nfn main() {}"}],"locations":[{"path":"...","line":1}]}
```

This means the plan's Task 7 approach — reconstructing `Diff` purely from intercepting `fs/write_text_file` bytes — is not the *only* source of truth. The adapter also sends a complete `{path, oldText, newText}` diff directly on `tool_call_update.content`. Task 3's normalizer currently drops it (`ToolCallUpdate` only extracts `tool_call_id` and `status`). Worth a decision for Task 3/7, not acted on here: use the wire diff directly, or keep the `fs/write_text_file` reconstruction as the sole source for consistency with non-file tools. Also note: **`status` is sometimes entirely absent** from a `tool_call_update` frame (an intermediate update can carry only `rawOutput`/`content`, no `status` key) — the plan's `#[serde(default)]` on `status` already handles this by defaulting to `""` → `ToolStatus::Pending`, which is technically wrong (the tool isn't "pending," it's mid-flight) but not unsafe.

### `session/request_permission` — matches the plan's `session.rs` exactly

```json
{"options":[{"kind":"allow_always","name":"Always Allow","optionId":"allow_always"},{"kind":"allow_once","name":"Allow","optionId":"allow"},{"kind":"reject_once","name":"Reject","optionId":"reject"}],"sessionId":"...","toolCall":{"toolCallId":"...","rawInput":{...},"title":"..."}}
```

`params.options[].optionId/name/kind` and `params.toolCall.toolCallId` line up exactly with `permission_event()` in the plan's Task 7 `session.rs`. `kind` values seen: `allow_always`, `allow_once`, `reject_once`. Note `toolCall` in the *permission* params does **not** carry a `kind` field (unlike the `tool_call` update itself), so a UI cannot show the permission prompt's tool type without correlating back to the `tool_call` notification by `toolCallId`.

### `session/prompt` RPC result — turn-end reason lives here, not in a `session/update`

```json
{"jsonrpc":"2.0","id":3,"result":{"stopReason":"end_turn"}}
```

`EndReason` (Task 2's enum) should be derived from the `session/prompt` request's own RPC response (`result.stopReason`), not from a `session/update` notification — none of the captures show a `session/update` carrying an end-of-turn signal.

## Fixtures produced

- `crates/agent/tests/fixtures/session_basic.jsonl` — 26 inbound frames from the "Add a doc comment to main.rs" turn (initialize result, session/new result, available_commands_update, message chunks, a `Read` tool call/update, an `Edit` tool call/update, a `session/request_permission`, two `fs/*` requests, the `session/prompt` result).
- `crates/agent/tests/fixtures/session_permission.jsonl` — 22 inbound frames from a "create then delete a scratch file via shell" turn: a `Bash` tool call/update and a `session/request_permission` for the mutating `touch` command (kind `execute`), plus the same initialize/session-new/available-commands/message-chunk scaffolding.

Both were extracted with `jq -c 'select(.dir=="in") | .msg' capture*.jsonl`, one JSON object per line, inbound frames only, exactly as the plan specifies.
