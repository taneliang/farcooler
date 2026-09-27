# Phone Permission Answers (ov-14) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** When claude, running as a TUI in a Far Cooler pane, asks for permission, the owner can answer from the lock screen or the watch. The keyboard can still answer, and the first answer wins. Whichever answer lands, the other surfaces stop offering buttons.

**Architecture:** claude runs Far Cooler's `PermissionRequest` hook while it shows its own dialog. The hook connects to the daemon and must hear back within the existing 400 ms. The daemon either says "no decision" (`{}`) or sends a **hold** frame. A hold lets the hook wait up to 60 s more for a second line, which carries the verdict. While the ask is held, the daemon records `AgentEvent::Permission` in the pane's ring, which the lock screen and the watch already replay and answer through `terminal.agent_answer`. The ask ends in one of five ways: a device answers, the keyboard answers (seen as claude's dialog leaving the screen), the turn ends, the hold runs out, or the hook goes away. Each ending records exactly one `AgentEvent::Resolved`.

**Tech Stack:** Rust (tokio, serde), Unix sockets, tmux. No app changes.

**Inputs:** `.claude/agent/reports/ov14-plan-check.md` (the state of the tree) and `.claude/agent/reports/ov14-spike.md` (claude 2.1.283 measured). This plan replaces Tasks 11 and 12 of `docs/superpowers/plans/2026-09-07-live-agent-sessions.md`. Phase 3 (Tasks 13 and 14 there) stays parked.

Every `file:line` below was checked against `main` at `bebf1379` on 2026-09-28.

---

## Decisions already made (by the owner)

- Claude only. Codex and cursor register no gate.
- Any of the owner's devices with Control scope may answer, for every claude pane, orchestrators included. There is no opt-in. `terminal.agent_answer` is already Control-scoped (`crates/daemon/src/rpc.rs:360-365`).
- A held ask waits 60 s for a device.
- A deny names the device: "Denied from iPhone".

## Open decisions for the owner

1. **The watch answers as the phone.** The watch reaches the runner through the phone (`apps/ios/FarCooler/WatchLinkHost.swift:322-326`), so the daemon sees the phone's `client_id`. A deny from the wrist therefore reads "Denied from iPhone". Saying "Apple Watch" needs a new field on `AgentAnswer` and changes to four app call sites. *This plan ships without that.*
2. **What the label really says.** "Denied from <label>" uses the device's enrollment label from `authorized_keys`. The phone enrolls with `UIDevice.current.name` (`apps/ios/FarCooler/Ceremony/AddDeviceView.swift:23`). The app has no device-name entitlement (grep finds none), so on iOS 16 and later that name is the generic "iPhone". Whether the ceremony carries that name onto the fence line is unmeasured. Task 6 step 1 reads the owner's file to check.
3. **The phone app's own pane view shows no buttons for a TUI pane.** iOS draws the chat only for agent-mode panes (`apps/ios/FarCooler/TerminalView.swift:902`). So the answer surfaces are the lock screen card and the watch. An in-app banner is app work, and it is not in this plan.
4. **Parallel asks.** Keeping one held ask per pane is a choice made here: a newer ask withdraws the older one, and the older one's dialog stays up for the keyboard. Whether claude ever fires two `PermissionRequest` hooks at once (parallel tool calls, subagents) is unmeasured.

## Global constraints

- US English. Never run `cargo fmt`; match the file's wrapping. If `cargo` is not on `PATH`, source the Rust env.
- **Every test is seen red first,** for the reason it names. Where a test would pass before the change, step 2 says how to break it on purpose.
- **A hook is never in the way.** Every failure path in `crates/cli/src/hook.rs` exits 0 and prints nothing. A hold only widens the wait after the daemon has answered inside 400 ms.
- **Tests never start the real claude.** The daemon unit tests get `test_agent::PROGRAM` (`crates/daemon/src/service.rs:517-543`). The end-to-end test uses `FARCOOLER_STAND_IN_AGENT` and the trap (`crates/client/tests/against_a_real_daemon.rs:94-196`).
- Nothing a person reads carries a raw Rust error. Refusals cross the wire as existing `DomainError` words.

---

## The design, answering the six questions

### 1. How the held hook waits

**Wire** (`crates/agent-hooks/src/wire.rs`). `HookVerdict` gains `hold_ms: Option<u64>`, with `#[serde(default, skip_serializing_if = "Option::is_none")]`. For a gating event, the daemon's first line is always one of:
- `{}`: no decision. The hook prints nothing and exits at once.
- `{"decision":…}`: a decision. Unused in this cut, but kept.
- `{"hold_ms":60000}`: the hold. The hook then reads a second line, which is a plain `HookVerdict`.

An older `farcooler` binary decodes a hold frame as `decision: None`, because serde ignores unknown fields and `wire.rs:27-31` has no `deny_unknown_fields`. So version skew defers to the keyboard. It never approves anything.

New shared constants go in `wire.rs`, because both crates read them:
- `LONGEST_HOLD = 60 s`.
- `GATES: &[(Agent, &str)] = &[(Agent::Claude, "PermissionRequest")]`. This table is what the installer registers as gating and what `serve` treats as a gate. `HookLine` carries no gating flag (`wire.rs:14-20`), and it does not need one.

**Hook** (`crates/cli/src/hook.rs`).
- `converse` (`:179-204`) reads the first line under the caller's deadline, as today.
- On a hold frame, it reads one more line, bounded by `min(hold_ms, LONGEST_HOLD)`.
- `run`'s outer bound (`:75`) becomes `deadline + LONGEST_HOLD` for a gating hook. For every other hook it stays at `deadline`.

So the first contact is still bounded at `HOOK_DEADLINE` (400 ms, `:50`). The long wait happens only once a live daemon has asked for it. The doc at `:36-42` ("nothing here can be told that") is rewritten, because the hold is exactly that signal. `--deadline-ms` stays test-only, and `no_installed_hook_carries_a_deadline_of_its_own` (`crates/daemon/src/hook_install.rs:976`) stays true unchanged.

**Registration** (`hook_install.rs`). `PermissionRequest` joins `CLAUDE_ONLY_EVENTS` (`:79`) as gating. Its entry carries `"matcher": "*"` and `"timeout": 70`, the shape the spike measured. It is not added to `CLAUDE_CODEX_EVENTS` (`:53-57`) or `CURSOR_EVENTS` (`:85-89`).

Why 70 s: claude SIGTERMs a hook at its `timeout` (spike, run 9: 8.004 s for `timeout: 8`). 70 s is above 0.4 s + 60 s, so the daemon's own `{}` always lands first, and a wedged hook still dies.

Claude's settings file is written fresh each launch and never merged (`service.rs:849-870`, `:910-930`). So the extra keys do not touch `entry_is_ours` (`hook_install.rs:432-445`). Orchestrators get the same table through `write_orchestrator_settings` (`service.rs:920-926`).

**Daemon** (`crates/daemon/src/hook_ingress.rs`, `serve` at `:726-811`). For a line whose `(agent, event)` is in `GATES`:

1. `terminal_for` (`:783`) stays before the reply. Without it the ask cannot be routed.
2. If it returns `None`, write `{}` now. Today nothing is written and the hook burns its full 400 ms. That covers an unclaimed session, an ambiguous one, and a chat pane (`is_a_chat`, `:845`).
3. If it returns `Some(terminal)`:
   - hold the ask in the ledger (Task 4), which mints its id;
   - write the hold frame;
   - only if that write succeeded, record `[Permission]` through the sink. If it failed, withdraw quietly: the hook missed its 400 ms and has gone.
4. Only now, after the reply, run `start_transcript_tail`, `observe_cwd` and `accept` (today `:787-798`). They move off the executor, into `spawn_blocking`. This is the card's "claim check after the reply", and it is marked at `:788-792`.
5. Wait on `select!` over three things:
   - the ask's decision channel;
   - `LONGEST_HOLD` elapsing, which means withdraw;
   - the read half reaching EOF or an error, which means the hook is gone, so withdraw.
6. Write the verdict line, and acknowledge the write back to whoever answered.

Non-gating lines keep today's order. The 600 s `IDLE` (`:68`) does not apply while an ask is held, because `serve` is then in step 5 and not in `read_line`. Each connection already has its own task (`:716-722`), so one held ask blocks no other hook.

### 2. From an ask to the phones, and from a phone's answer to the hook

**Ask to phones.** The event is built by a pure function, `permission_ask(id, payload)`, in `crates/agent-hooks/src/ask.rs`:
- `Permission { id, tool_call: "", options }`.
- The options come from one shared builder, moved to `farcooler-agent-core` (Task 1). ACP's `normalize::permission_event` (`crates/claude/src/normalize.rs:603-621`) uses the same builder, so the vocabulary cannot drift. The ids are `allow` and `deny`, the kinds are `allow_once` and `reject_once`, and the allow button is named `Allow <title>`, from `tool_name` and `tool_input`. The hook payload says `tool_input` where ACP says `input` (fixture `crates/agent-hooks/tests/fixtures/claude-permission-request.json`).
- `tool_call` is empty because the hook has no `tool_use_id`. Empty is already the documented value when an agent leaves it out (`normalize.rs:607`, `apps/shared/AgentKit/Sources/AgentKit/WatchLink.swift:516-532`). Its only reader is an id comparison in the chat (`apps/ios/FarCooler/AgentView.swift:1188-1190`).
- No synthetic `ToolCall` is emitted. No client draws a chat for a TUI pane (open decision 3), and a synthetic row would have no honest end state.

The event is recorded through `AgentSupervisor::record` (`crates/daemon/src/agent_supervisor.rs:584`), using the sink `resume_agent_listeners` already installs (`service.rs:3861-3877`). The pane reads Blocked from its screen, because claude draws the dialog at once (spike §1; `crates/core/src/activity.rs:242-256`). That Blocked is what raises the lock screen card, and the card and the watch then replay the ring and find `pendingPermission` (`apps/shared/AgentKit/Sources/AgentKit/Transcript.swift:501-505`, `WatchLinkHost.swift:430-445`). The plan-check's worry that `watch.rs` needs a parked ask forced to Blocked is moot for 2.1.283 (spike, "What it means").

**Phone's answer to hook.** In `rpc.rs:2062-2078` (`terminal.agent_answer`), a `request_id` that starts with `HOOK_ASK_PREFIX` goes to `svc.hooks().answer(terminal, request_id, option_id, decider)`. Every other id still goes `to_the_shim` (`:282-288`).
- `allow` becomes `Decision::Allow`.
- `deny` becomes `Decision::Deny { message: "Denied from <decider>" }`.
- Any other option id is refused with `InvalidArgument { what: "option_id" }`, and the ask stays held.

`answer` returns only after `serve` has written the verdict to the hook's socket, with a bound of 2 s. So a phone is never told "sent" about a verdict that reached no hook. This matters because the glance surfaces are required not to report success they cannot confirm (`apps/shared/AgentKit/Sources/AgentKit/AnswerPermissionIntent.swift:62-66`).

How the decider is named:
- A remote peer (`Peer.client_id`, `crates/transport/src/lib.rs:46-58`) is named by its fence entry's `label` (`crates/fence/src/lib.rs:76`), read with the existing `enrollment::read` (`crates/daemon/src/enrollment.rs:662`). The label is trimmed, cut to 40 characters, and control characters are removed. The message goes to the model, not the TUI: the TUI draws only "Denied by PermissionRequest hook" (spike §2).
- A local caller (`client_id: None`, which is the Mac app or the CLI) is "Mac".
- A label that is missing or empty becomes "a paired device".

### 3. Detecting a keyboard answer

**Chosen signal: claude's dialog leaves the pane's screen.** The watcher already samples that screen.

Evidence:
- The watcher captures every live non-agent, non-changes pane once per `SAMPLE_INTERVAL = 1 s` (`crates/daemon/src/watch.rs:43`, `:3624`, `:4058`, `:4073-4074`), and classifies it at `:4113` (`screen_says`).
- claude's permission dialog is a positive, footer-anchored match (`activity.rs:242-256`, checked at `:801`). The 2.1.283 dialog the spike captured carries three of those needles: "Do you want to", "❯ 1. Yes" and "Esc to cancel · Tab to amend" (spike §1).
- A keyboard "Yes" took the dialog down within about 0.1 s. The key was pressed at 1790550967.340, and the frame at .443 shows the working footer instead (`/tmp/fc-t/ov14-spike/cap-r2.txt`). A hook allow took it down within about 1 s (spike run 1, +30.0 to +31.0 s).
- It covers every way out of the dialog: Yes, No, Esc and interrupt. It costs nothing new, because the capture already happens.

The alternatives, rejected:
- **PostToolUse.** It is not registered today (`hook_install.rs:36-52`). It fires only for a tool that ran, so a keyboard No or Esc produces none. For a Yes it fires after the tool finishes, so a 5-minute `cargo test` keeps the phone offering a stale ask until the hold ends. It would also add a hook process to every tool call, about 15-18 ms each (`hook_install.rs:46-47`).
- **The transcript.** Claude panes get no transcript tail on the hook path (`hook_ingress.rs:557-560`; the test `claude_never_gets_a_transcript_tail`, `:1425`). The watcher's own log reader says outright that it "cannot see a permission prompt" (`watch.rs:4121-4123`). And a Yes's `tool_result` lands after the tool runs, the same latency as PostToolUse.

**The rule is an edge, not a level.** A held ask is withdrawn once the dialog has been seen at least once since the ask was held, and has then been absent for 2 consecutive samples. Two samples copies `CONFIRMATIONS` (`watch.rs:937`) and stops a one-frame redraw from withdrawing a live ask. A dialog not yet drawn never counts as gone.

Two backstops cover a dialog answered before any sample saw it:
- a `Stop` or `UserPromptSubmit` hook from the same pane withdraws its held ask, because a turn cannot end or begin with a dialog up;
- the 60 s hold.

`MessageDisplay` deliberately does not withdraw an ask, because a parallel subagent could send one while the dialog is still up.

On withdrawal, the ledger sends `None` to `serve`, which writes `{}`. Claude ignores that, because the keyboard already decided (spike run 2). The ledger also records `Resolved { id, chosen: "" }`, so every surface drops its buttons.

### 4. Matching an ask to its answer without `tool_use_id`

- **Device to ask, by id.** The daemon mints `hook-ask-<uuid v7>` per ask. The phone echoes it back unchanged, which is the existing `AgentAnswer.request_id` contract (`proto/farcooler.proto:320-326`). Ids are unique for the life of the daemon, and the ring is in memory, so no id can outlive its daemon and meet another ask.
- **Keyboard to ask, by pane.** The screen edge and the turn backstops are per terminal. The ledger holds **at most one ask per terminal**. A newer `PermissionRequest` on the same pane withdraws the older one, with `{}` and `Resolved`. The older dialog stays up for the keyboard (open decision 4).
- The screen only counts samples taken after the ask was held, so a dialog from an earlier ask cannot satisfy a later one.

### 5. Timeout, cancel, restart, two answers

| What happens | The hook prints | The ring gets | A later device answer gets |
|---|---|---|---|
| A device answers first | allow, or deny "Denied from X" | `Resolved{chosen: allow/deny}` | `ResourceConflict` |
| The keyboard answers (the dialog leaves) | nothing (`{}`) | `Resolved{chosen: ""}` | `ResourceConflict` |
| The turn ends or begins (`Stop`, `UserPromptSubmit`) | nothing | `Resolved{""}` | `ResourceConflict` |
| A newer ask on the same pane | nothing | `Resolved{""}` for the older ask | `ResourceConflict` |
| 60 s with no answer | nothing, and the dialog stays for the keyboard | `Resolved{""}` | `ResourceConflict` |
| The hook goes away (claude exits, the pane dies, SIGTERM at 70 s) | nothing | `Resolved{""}` | `ResourceConflict` |
| The terminal row is deleted (`HookIngress::forget`, `hook_ingress.rs:443`; called at `service.rs:2890`) | nothing | nothing (the ring goes too) | `ResourceConflict` |
| The daemon restarts mid-hold | nothing: EOF on the second read | nothing: the ring is in memory (`agent_supervisor.rs:175-176`) | `ResourceConflict`, because the id has the hook prefix and nothing holds it |
| The daemon wedges mid-hold | nothing when the hook's own ceiling (60 s) runs out | nothing | an answer that cannot be written fails its 2 s ack: `ResourceConflict` |
| An old `farcooler` binary gets a hold | nothing at once (the hold decodes as no decision) | `Resolved{""}` on EOF | `ResourceConflict` |

- **Two phones answer.** The first `answer` settles the ask. The second finds nothing held under that id and gets `ResourceConflict`. The apps already show that word as "Something else changed this first. Take another look and try again." (`apps/shared/AgentKit/Sources/AgentKit/RunnerRefusal.swift:112-113`). No app change.
- **A phone and the keyboard in the same instant.** claude takes whichever reaches it first and ignores the other (spike run 2). The ring may then say `allow` where the keyboard said No. Nothing on this side can know which one won, and this is written down rather than hidden.
- **Exactly one `Resolved` per ask.** Every ending goes through one `settle` that removes the entry under the lock, so the answer, withdraw, forget, supersede and timeout paths cannot double-resolve. Nothing in the tree produces `Resolved` today (grep: only `event.rs:267` and `activity_source.rs:28`), so this is its first producer.

### 6. Tests

These are listed per task below, each with its red step. The end-to-end test is Task 9.

---

## Tasks

### Task 1: One options vocabulary for both claude paths

**Files:**
- Create: `crates/agent-core/src/permission.rs`
- Modify: `crates/agent-core/src/lib.rs`, `crates/claude/src/normalize.rs`

The work:
- Move `tool_title` and `task_update_title` (`normalize.rs:531-560`) into `permission.rs` as `pub fn claude_tool_title`.
- Add `pub fn permission_options(tool_name: &str, input: &Value) -> Vec<PermissionOption>` there, returning exactly what `normalize.rs:608-619` builds.
- `permission_event` calls it. The existing `tool_title` tests in `normalize.rs` call the moved function.

- [ ] **Step 1: Write the failing tests** in `permission.rs`:
  - `the_options_are_allow_and_deny_by_those_ids`: ids `allow`/`deny`, kinds `allow_once`/`reject_once`.
  - `a_bash_ask_is_named_for_its_command`: `Allow touch x`.
- [ ] **Step 2: Watch them fail.** Stub `permission_options` to return `vec![]`, and see both go red on the assertion, not on compilation.
- [ ] **Step 3: Implement.** Run `cargo test -p farcooler-agent-core -p farcooler-claude`. All existing `normalize` tests must stay green.
- [ ] **Step 4: Commit.** `agent-core: one permission vocabulary for claude's ACP and hook paths (ov-14)`

### Task 2: The hold frame, the gate table, and the ask event

**Files:**
- Modify: `crates/agent-hooks/src/wire.rs`, `crates/agent-hooks/src/lib.rs`
- Create: `crates/agent-hooks/src/ask.rs`

The work:
- Add `hold_ms`, `LONGEST_HOLD` and `GATES` (design §1).
- Add `pub fn is_gate(agent, event) -> bool`.
- Add `pub fn permission_ask(id: &str, payload: &Value) -> AgentEvent`, reading `tool_name` and `tool_input` through Task 1's builder, with `tool_call: ""`.

- [ ] **Step 1: Write the failing tests.**
  - `wire::tests::a_hold_survives_a_round_trip`.
  - `wire::tests::a_hold_is_not_a_decision_to_a_hook_that_predates_it`. Decode `{"hold_ms":60000}` into a local copy of the pre-change struct, `struct Before { #[serde(default)] decision: Option<Decision> }`, and assert `decision == None`.
  - `wire::tests::a_verdict_with_no_hold_writes_no_hold_key`. The encoded `{}` stays `{}`, which old hooks rely on.
  - `ask::tests::a_claude_permission_request_becomes_a_permission_with_no_tool_call`, against the fixture `claude-permission-request.json`: options `Allow /tmp/probe/probe-test.txt` and `Deny`, and `tool_call == ""`.
  - `ask::tests::only_claudes_permission_request_is_a_gate`: codex `PermissionRequest`, cursor `beforeShellExecution` and claude `Stop` are all not gates.
- [ ] **Step 2: Watch them fail.** `a_hold_is_not_a_decision…` passes before the change, so prove it can fail: add `#[serde(deny_unknown_fields)]` to `Before`, see it go red, then remove it.
- [ ] **Step 3: Implement,** then run `cargo test -p farcooler-agent-hooks`.
- [ ] **Step 4: Commit.** `agent-hooks: a hold frame, the gate table, and a permission ask event (ov-14)`

### Task 3: The hook waits through a hold, and nowhere else

**Files:**
- Modify: `crates/cli/src/hook.rs`, `crates/cli/tests/a_hook_is_never_in_the_way.rs`

The work is design §1, the hook half. Add `pub fn hold_from_ms(ms) -> Duration`, capped at `LONGEST_HOLD`.

- [ ] **Step 1: Write the failing tests.** Unit tests in `hook.rs`, using the fake-daemon pattern of `:276-314`:
  - `a_held_hook_prints_the_decision_that_follows_the_hold`: the daemon writes the hold, waits 500 ms (longer than `HOOK_DEADLINE`), then writes a deny. Assert the deny envelope. This proves the wait widened.
  - `a_held_hook_whose_daemon_goes_quiet_prints_nothing_when_the_hold_ends`: `hold_ms: 300`, then silence. Assert `""`, returned in under 2 s.
  - `a_held_hook_whose_daemon_hangs_up_prints_nothing`: hold, then drop the stream. Assert `""` at once.
  - `a_hold_that_arrives_after_the_first_contact_deadline_is_ignored`: the hold is written at 600 ms. Assert `""` in under 1 s.
  - `a_non_gating_hook_never_waits_for_a_hold`: `gating: false`, and the daemon writes a hold. Assert a return in under 400 ms.
  - `a_hold_is_capped_at_the_longest_hold`: `hold_from_ms(u64::MAX) == 60 s`.

  Binary-level, in `a_hook_is_never_in_the_way.rs`:
  - `a_held_hook_exits_zero_and_silent_when_its_daemon_dies_mid_hold`: exit 0, empty stdout and stderr, via `it_was_never_in_the_way` (`:194`).
- [ ] **Step 2: Watch them fail.** Today the first two print nothing, because the hold decodes as no decision. Check that each red is for the stated reason. `a_non_gating…`, `…hangs_up…` and `…after_the_first_contact…` pass today, so break them by making `converse` read a second line unconditionally, and see all three go red.
- [ ] **Step 3: Implement.** Rewrite the doc at `hook.rs:28-50` to say that the first contact is bounded and a hold widens only on the daemon's word. Run `cargo test -p farcooler-cli`.
- [ ] **Step 4: Commit.** `cli: a gating hook waits through the daemon's hold, and only then (ov-14)`

### Task 4: The held-ask ledger

**Files:**
- Create: `crates/daemon/src/hook_asks.rs`
- Modify: `crates/daemon/src/lib.rs`, `crates/daemon/src/hook_ingress.rs` (an `asks: Arc<HookAsks>` field; `forget` calls `asks.forget`)

`HookAsks` is pure state: no sockets. It knows nothing about screens either: it is told what they show. It holds a `Mutex<HashMap<Uuid, Held>>` and an `EventSink` for `Resolved`. A `Held` is `{ id, since, seen_dialog: bool, absent_samples: u8, reply: oneshot::Sender<Settled> }`, where `Settled` carries `Option<Decision>` and an ack sender.

API:
- `hold(terminal) -> (String, Receiver<Settled>)`. It supersedes any older ask on the same terminal.
- `answer(terminal, id, option, decider) -> Result<(), AnswerRefused>`. It awaits the ack, bounded at 2 s.
- `saw_screen(terminal, dialog_up: bool)`
- `turn_boundary(terminal)`
- `withdraw(terminal, id)`
- `forget(terminal)`
- `is_holding(terminal) -> bool`, for tests and logs.

Every ending goes through one private `settle`.

- [ ] **Step 1: Write the failing tests** in `hook_asks.rs`:
  - `an_allow_from_a_device_decides_the_held_ask`
  - `a_deny_names_the_device_that_sent_it`: `Deny { message: "Denied from iPhone" }`.
  - `an_option_the_ask_never_offered_is_refused_and_the_ask_stays_held`
  - `a_second_answer_to_the_same_ask_is_refused`
  - `an_answer_naming_an_id_nobody_holds_is_refused`
  - `a_newer_ask_on_the_same_pane_withdraws_the_older`
  - `every_ask_is_resolved_exactly_once`: drive each ending (answer, withdraw, supersede, turn boundary, dialog gone, forget) and count `Resolved` events per id; each must be exactly 1.
  - `the_dialog_leaving_withdraws_the_ask_only_after_it_was_seen`
  - `a_dialog_missing_for_one_sample_does_not_withdraw_the_ask`
  - `a_dialog_not_yet_drawn_does_not_withdraw_the_ask`
  - `a_turn_boundary_withdraws_a_held_ask`
  - `forgetting_a_terminal_withdraws_its_ask`
- [ ] **Step 2: Watch them fail.** Use `todo!()` bodies. Then, for `every_ask_is_resolved_exactly_once`, also emit `Resolved` in `answer` as well as in `settle`, and see it catch the double.
- [ ] **Step 3: Implement,** then run `cargo test -p farcooler-daemon hook_asks`.
- [ ] **Step 4: Commit.** `daemon: a ledger of held permission asks, each resolved once (ov-14)`

### Task 5: `serve` replies first, then holds

**Files:**
- Modify: `crates/daemon/src/hook_ingress.rs`, `crates/daemon/tests/a_hook_reaches_the_daemon.rs`
- Modify: `crates/daemon/src/claims.rs` (a `#[cfg(test)]` way to hold the `Ledger` lock, for one test)

The work is design §1, the daemon steps 1-6. `HookIngress::with_hold(Duration)` sets the hold length for tests; the default is `LONGEST_HOLD`. `Stop` and `UserPromptSubmit` from claude call `asks.turn_boundary(terminal)` before `accept`.

- [ ] **Step 1: Write the failing tests.** Integration tests over the real socket (`a_hook_reaches_the_daemon.rs`, using its `store_with_terminal` and `listening_on`):
  - `a_permission_request_from_a_bound_claude_is_held_and_offered`: the first line is `{"hold_ms":…}`, and the sink got one `Permission` whose options are `allow`/`deny`.
  - `a_permission_request_nobody_claims_is_told_no_decision_at_once`: `{}` within 400 ms, and no events.
  - `a_permission_request_from_a_chat_pane_is_told_no_decision_at_once`: `PaneMode::Agent`.
  - `a_codex_permission_request_is_never_held`: codex is not in `GATES`, so no reply line and no `Permission`.
  - `a_held_ask_answered_from_a_device_writes_the_verdict_to_the_hook`: through `ingress.answer(…)`. The second line is the deny with its message, and the sink got `Resolved{chosen:"deny"}`.
  - `a_held_ask_nobody_answers_is_released_with_no_decision_when_the_hold_ends`: `with_hold(300 ms)`. The second line is `{}`, and there is one `Resolved{""}`.
  - `a_hook_that_hangs_up_mid_hold_withdraws_its_ask`: drop the client. `is_holding` goes false, with one `Resolved`.
  - `a_stop_from_the_same_session_withdraws_its_held_ask`

  A unit test in `hook_ingress.rs`:
  - `the_reply_does_not_wait_for_the_claim_check`: hold the claims `Ledger` lock, send a `PermissionRequest` whose `cwd` is the pane's own worktree (so `observe_in` reaches `settle`), and assert the hold frame arrives within 400 ms.
- [ ] **Step 2: Watch them fail.** Today no reply line is ever written. For `the_reply_does_not_wait…`, first write the reply after `observe_cwd` (where `:788-792` says it must not be), and see it time out. Then move it.
- [ ] **Step 3: Implement.** Delete the stale comment at `:788-792` and replace it with one saying where the claim check now runs and why. Run `cargo test -p farcooler-daemon --test a_hook_reaches_the_daemon` and `cargo test -p farcooler-daemon hook_ingress`.
- [ ] **Step 4: Commit.** `daemon: a permission hook is answered first and held after, claim check last (ov-14)`

### Task 6: `terminal.agent_answer` reaches a held hook, and names the device

**Files:**
- Modify: `crates/daemon/src/rpc.rs` (`:2062-2078`)
- Modify: `crates/daemon/src/service.rs` (`pub fn hooks(&self) -> &HookIngress`, beside `agents()` at `:2400`)
- Modify: `crates/daemon/src/enrollment.rs` (`pub async fn label_for(svc, client_id) -> Option<String>`, over `read` at `:662`, on the blocking pool)

- [ ] **Step 1: Measure the label (read only).** Run `grep farcooler ~/.ssh/authorized_keys | awk '{print $NF}'` on the owner's runner, and record what a phone's label reads in the task report. If it is not a device noun, stop and ask the owner (open decision 2).
- [ ] **Step 2: Write the failing tests** in a new `#[cfg(test)] mod hook_answer_tests` in `rpc.rs`, built like `terminal_task_tests` (`:2703-2720`), with a peer of `client_id: Some("phone-1")` enrolled through `enrollment::enroll` with label `iPhone`:
  - `an_answer_to_a_held_hook_ask_goes_to_the_hook_not_the_shim`: a Terminal-mode pane. Today this is `AgentNotConnected`.
  - `a_deny_from_an_enrolled_device_is_named_by_its_label`: the verdict's message is `Denied from iPhone`.
  - `a_deny_from_the_local_socket_is_named_for_the_mac`: `Denied from Mac`.
  - `an_answer_to_an_ask_no_longer_held_is_a_conflict`: after one answer, a second gets `ResourceConflict`, not `AgentNotConnected`.
  - `a_device_label_is_cut_to_one_short_line`: a label with a newline, and one of 200 characters.
- [ ] **Step 3: Watch them fail,** each for its own reason. Today every one is `AgentNotConnected`.
- [ ] **Step 4: Implement,** then run `cargo test -p farcooler-daemon hook_answer_tests`.
- [ ] **Step 5: Commit.** `daemon: a phone's answer reaches claude's held hook, and a deny names the device (ov-14)`

### Task 7: The watcher tells the ledger what the screen shows

**Files:**
- Modify: `crates/daemon/src/watch.rs` (after `screen_says`, `:4113`)
- Create: `crates/core/captures/claude-permission-hook-waiting.txt` and `crates/core/captures/claude-after-a-keyboard-yes.txt`
- Modify: `crates/core/src/activity.rs` (tests)

In the Terminal-mode branch, call `self.service.hooks().asks().saw_screen(id, screen_says == AgentActivity::Blocked)`. The call uses the raw `screen_says`, not the folded or resolved activity. It costs no capture.

Record the captures from the spike, following `crates/core/captures/README.md`:
- the r1 frame at `t=1790550898.785` (dialog up), which is also verbatim in `ov14-spike.md` §1;
- the r2 frame at `t=1790550967.443` (dialog gone, working footer).

If `/tmp/fc-t/ov14-spike/` is gone, rebuild the first from the spike report, and the second from this r2 frame (claude 2.1.283, 140 columns):

```
❯ Run the shell command touch x in this directory. Nothing else.
  Creating empty file x
  ⎿  $ touch x
  <spinner>
                                                                                                                        ◐ medium · /effort
❯ 
  ⏸ manual mode on · esc to interrupt · ← 3 agents
```

- [ ] **Step 1: Write the failing tests.**
  - `activity::tests::claude_2_1_283s_permission_dialog_is_blocked`
  - `activity::tests::claude_2_1_283_after_a_keyboard_yes_is_not_blocked`
  - `watch::tests::a_dialog_that_leaves_the_screen_releases_the_held_ask`: `test_support::fixture`, a shell pane, `printf` the dialog capture into it, `svc.hooks().asks().hold(term)`, then sample until the dialog is seen. Then `clear` the pane, sample twice, and assert the receiver got `None` and `is_holding` is false.
- [ ] **Step 2: Watch them fail.** The two capture tests pass today, so break each on purpose by deleting the dialog lines from a copy, and see red. The watch test is red until the call is wired.
- [ ] **Step 3: Implement,** then run `cargo test -p farcooler-core activity` and `cargo test -p farcooler-daemon a_dialog_that_leaves`.
- [ ] **Step 4: Commit.** `watch: a claude dialog that leaves the screen releases the phone's ask (ov-14)`

### Task 8: Register the gate, for claude only

**Last before the end-to-end test.** Once this lands, every permission in every Far Cooler claude pane runs the hook. Before Tasks 5-7 land, that would cost up to 400 ms per prompt for no return (`hook_install.rs:43-47`).

**Files:**
- Modify: `crates/daemon/src/hook_install.rs`

The work:
- Add `("PermissionRequest", true)` to `CLAUDE_ONLY_EVENTS`.
- `claude_settings` writes `"matcher": "*"` and `"timeout": CLAUDE_GATE_TIMEOUT_S` (70) on gating entries only.
- Rewrite the docs at `:36-52` and `:80-84` to say what is registered now, and why codex and cursor still are not (their output shapes are unmeasured, and cursor's empty-output behavior is unmeasured).
- Update `no_installed_hook_carries_a_deadline_of_its_own`'s doc: "no table sets `gating` today" is false after this task.

- [ ] **Step 1: Write the failing tests.**
  - `claudes_permission_request_is_registered_as_a_gate`
  - `claudes_permission_hook_outlives_the_longest_hold`: `timeout` × 1000 > `LONGEST_HOLD` + `HOOK_DEADLINE` + 5 s. The CLI's 400 ms is copied as a named constant here, with a comment saying where it comes from.
  - `every_gate_is_registered_as_gating_for_its_agent_and_nothing_else_is`: `GATES` against all three tables, in both directions.
  - `codex_and_cursor_get_no_gate`
- [ ] **Step 2: Watch them fail.** Nothing registers `PermissionRequest` today.
- [ ] **Step 3: Implement.** Run `cargo test -p farcooler-daemon hook_install`, plus `an_orchestrators_settings_are_the_hooks_and_the_repositorys_memory` (`service.rs:6973`), which must stay green: orchestrators get the gate through the same table.
- [ ] **Step 4: Commit.** `hook_install: claude's PermissionRequest is a gate again, with room for a 60 s hold (ov-14)`

### Task 9: End to end, with a stand-in claude

**Files:**
- Modify: `crates/client/tests/against_a_real_daemon.rs`

The stand-in machinery here already runs a real `farcoolerd` with `FARCOOLER_STAND_IN_AGENT`, a trap `claude` on `PATH`, and a stripped environment (`:94-246`). Parametrize `StandIn::install` (`:183-193`) with a body, and add `spawn_with_stand_in(body)`. The existing `start_with_a_stand_in_agent` passes today's body unchanged.

The daemon writes the hook's CLI path through `shim_binary` (`crates/daemon/src/service.rs:44-51`). So the test asserts that `target/<profile>/farcooler` exists beside `farcoolerd`, as `daemon_binary` (`:24-35`) does for the daemon. Otherwise the hook would resolve a `farcooler` from `PATH`.

**The stand-in claude.** A `/bin/sh` script, launched as `claude --session-id S --settings F` (`service.rs:343`, `:351`, `:374-380`):
1. Parse `S` and `F` from `$@`.
2. Take the `PermissionRequest` command out of `F` with `sed` (the file is pretty-printed JSON that Far Cooler wrote itself).
3. Draw the spike's 2.1.283 dialog, banner line `Claude Code v2.1.283` included, so `identify` finds claude.
4. In a background subshell, pipe `{"session_id":"S","cwd":"$PWD","hook_event_name":"PermissionRequest","tool_name":"Bash","tool_input":{"command":"touch x"}}` into `sh -c "$cmd" > $OUT/hook-stdout`. Then touch `$OUT/hook-exited` and replace the dialog with `⎿  Allowed by PermissionRequest hook` when the output says allow.
5. In the foreground, `read answer`. On a line, clear the screen, draw the working footer (`esc to interrupt`), and write `$OUT/tui-answered`.
6. `exec sleep 600`.

- [ ] **Step 1: Write the failing tests.**
  - `a_permission_answered_from_a_phone_reaches_the_held_hook`: create a worktree and a `claude` terminal through the client, poll `agent_subscribe` until a `Permission` appears, then `agent_answer(terminal, id, "allow")`. Assert that `hook-stdout` holds the allow envelope, `hook-exited` exists, the ring's last event is `Resolved{chosen:"allow"}`, and the trap never ran.
  - `a_permission_denied_from_the_mac_says_so_to_the_model`: the same, with `deny`. The stdout message is `Denied from Mac` (the local socket has `client_id: None`).
  - `a_permission_answered_at_the_keyboard_releases_the_held_hook_and_the_phone`: wait for the `Permission`, then `write(terminal, b"1\n")`. Within 5 s: `hook-stdout` is empty, `hook-exited` exists, the ring gained `Resolved{chosen:""}`, and a later `agent_answer` for that id is `ResourceConflict`.
  - Tmux is required. On CI, a missing tmux fails the test, following `068fdc0b`.
- [ ] **Step 2: Watch them fail.** Run the three with Task 8 reverted, so no gate is registered: no `Permission` ever appears. Then run the keyboard test with Task 7's `saw_screen` call removed: the hook waits out its hold and the test times out.
- [ ] **Step 3: Make them pass.** Run `cargo build -p farcooler-daemon -p farcooler-cli && cargo test -p farcooler-client --test against_a_real_daemon permission`.
- [ ] **Step 4: Commit.** `test(client): a stand-in claude's permission, answered from a phone and from the keyboard (ov-14)`

---

## Not in this plan

- An `answered_from` field so the watch can say "Apple Watch" (open decision 1).
- Buttons in the phone app's own pane view for a TUI pane (open decision 3).
- Codex and cursor gates. Their output shapes are unmeasured (`crates/cli/src/hook.rs:206-210`), and for cursor an empty reply may mean allow (plan-check, constraint 2).
- Phase 3, typing into a TUI.
