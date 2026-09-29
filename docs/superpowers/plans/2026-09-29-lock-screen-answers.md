# ov-57: answering a claude TUI ask from the lock screen while the app is suspended

Read-only pre-work, 2026-09-29. Nothing here is built yet. Paths are repo-relative; `AK/` is
`apps/shared/AgentKit/Sources/AgentKit/`.

## 0. Where we are, in one paragraph

Everything past the card already exists. The card's buttons are `Button(intent:)` over
`AnswerPermissionIntent`, a `LiveActivityIntent` with `openAppWhenRun = false`
(`AK/AnswerPermissionIntent.swift:74-89`). It runs in the app's process through a handler the app installs at
launch (`AK/AnswerPermissionIntent.swift:126-162`, `apps/ios/FarCooler/WatchLinkHost.swift:718`). That handler
is `WatchLinkHost.answerFromGlance` (`WatchLinkHost.swift:762-872`). It connects, re-reads the pane, and sends
`terminal.agent_answer`. The daemon settles a hook ask exactly once and acks only once the verdict is on the
hook's socket (`crates/daemon/src/hook_asks.rs:156-181, 253-270`).

**The one missing fact is the ask itself.** The card draws buttons only from `GlancePermissionStore`, which only
the running app writes (`apps/ios/FarCoolerActivity/AgentActivityWidget.swift:349-364`,
`AK/GlancePermissions.swift:34-39`). The relay's content state has no ask in it (`services/relay/src/push.ts:243-350`).
So the smallest fix is to put the ask on the Live Activity's content state, and teach the widget to draw buttons
from it.

---

## 1. What the payload must carry

### 1.1 The field

Add one optional object, `ask`, to `ActivityState` (the Live Activity's content state), describing the **headline's**
ask only:

```json
"ask": { "id": "hook-ask-0199…", "tool": "Bash", "until": 1790551063000 }
```

| field | why | bound |
|---|---|---|
| `id` | What `terminal.agent_answer` echoes back. Always `hook-ask-` + a UUID v7 (`hook_asks.rs:37, 108`). | 45 bytes. The relay refuses anything that doesn't start with `hook-ask-` or is over 64 bytes. |
| `tool` | claude's `tool_name` ("Bash", "Edit", "WebFetch"), so the button can read "Allow Bash". It's a word from a fixed vocabulary, not content. | ≤ 32 bytes, `[A-Za-z0-9_.:-]`, else dropped. |
| `until` | When the daemon's hold ends, in Unix ms. This is `held.at + hold`. The widget can then show a countdown, and the intent can refuse locally once the hold is over (§4.1). | A number, like `startedAt` (`push.rs:226-251`). |

**What it does NOT need:**

- **The terminal and the runner.** The state already names the headline's `terminal` and `machine`
  (`push.ts:244-257`). The intent finds the runner by searching the fleet (`WatchLinkHost.swift:134-157`).
- **Option ids and kinds.** A hook ask's options are always `allow`/`allow_once` and `deny`/`reject_once`
  (`crates/agent-core/src/permission.rs:80-93`). AgentKit can synthesize them from the id prefix.
- **Option names.** The allow option's name is `"Allow " + claude_tool_title(...)`, which for Bash is **the raw
  command line** (`permission.rs:84`, and the test at `:110-112` gives `"Allow touch x"`). The push contract says
  "never a command line" (`crates/daemon/src/push.rs:98-104`, `services/relay/src/push.ts:13-17`), so names must
  not cross the relay. See §3.
- **A summary.** `detail`, the blocked question, is already on the state. It was redacted and cut to 40 characters
  on the runner (`push.rs:98-110`, `watch.rs:342-354`, `services/relay/src/index.ts:2073`).

### 1.2 Only the headline, carried per row

The relay composes the card's headline from its stored rows. When another agent's notice moves the card, the
headline's fields are copied from that agent's **stored row** (`index.ts:2121-2135`). So the ask has to be stored
on the row, not only passed through. Migration `0014_row_ask.sql` (0013 is ov-61's) adds nullable `ask_id`, `ask_tool` and
`ask_until` to `live_activities`. This is additive, like 0002-0012 (`migrations/0008_fleet_rows.sql:4-7`).

The widget answers the leader only (`AgentActivityWidget.swift:352`), so `ActivityRow` gains nothing.

### 1.3 Size

- **ActivityKit:** Apple documents that a Live Activity's dynamic data (the content state) can't exceed 4 KB, on
  both a local update and an ActivityKit push. *(Confident. See "Displaying live data with Live Activities" in the
  ActivityKit docs.)*
- **APNs:** a regular notification payload is limited to 4096 bytes. *(Confident. See "Generating a remote
  notification".)*
- **The relay already enforces both.** `STATE_BUDGET` is 3 KB (`index.ts:1448`), and rows are added until the
  encoded `{...state, rows}` would pass it (`index.ts:2021`). So `ask` needs to be set on `state` **before**
  `withFleet` runs. The worst case, about 110 bytes, then costs the card at most its last row, never the card.
  One test pins this down (R3 below).
- The alert push (`sendPush`) doesn't need the ask. The card is what carries it.

### 1.4 Why the card doesn't wake the app, and why that's fine

An ActivityKit push is applied by the system to the Live Activity's content state. The app isn't launched and
receives nothing. *(High confidence: Apple's "Starting and updating Live Activities with ActivityKit push
notifications" describes only the system updating the activity. The repo's own observation agrees: ov-54 L3 in
`.claude/agent/reports/ov54-review.md`.)* That's why the ask has to be **in** the state. Nothing on the phone can
fetch it later.

### 1.5 When the daemon sends it

- **On the blocked notice.** `watch.rs` sends `blocked` after `CONFIRMATIONS` (2) samples, 1 s apart
  (`watch.rs:43, 1027`). The hook's ask is held and offered within milliseconds of claude starting the hook
  (`hook_ingress.rs:900-909`), while claude draws its dialog at about +0.5 s (ov14 spike §1). So the ask
  nearly always exists by the time the blocked notice is composed. The notice reads it from
  `svc.hooks().asks().open()`, which is the same source `needs_you` uses (`needs_you.rs:97`).
- **On a change while still blocked.** A new `kind: "ask"` notice, `{kind, terminal, ask?, needsYou}`, is
  sent in two cases:
  - an ask is offered after the blocked notice went out;
  - an ask ends while the pane stays blocked. The usual case is the 60 s hold running out
    (`hook_ingress.rs:924-929`), because the TUI dialog stays up after that (ov14 spike §1).

  An absent `ask` means "none now". A relay older than this treats an unknown kind as "refresh the card silently"
  (`index.ts:1115-1119`). That's harmless, which is what makes the change additive. The new kind never alerts:
  `alerts` is true only for no kind or `decision` (`index.ts:1005`).

Why a new kind rather than re-sending `blocked`: every `blocked` agent notice is "news" and carries an alert
(`index.ts:2148-2154, 2296`). Re-sending one would buzz the phone again for a question the person has already
been told about.

---

## 2. How a lock-screen tap answers

### 2.1 What exists

1. `Button(intent: AnswerPermissionIntent(...))` is on the card (`AgentActivityWidget.swift:517-526`).
2. The intent is a `LiveActivityIntent`, so iOS performs it **in the app's process**. If the app isn't running, iOS
   launches it into the background. *(Confident: WWDC23 "Bring widgets to life" and the `LiveActivityIntent` docs.
   The codebase relies on it at `AnswerPermissionIntent.swift:52-61`.)*
3. `answerFromGlance` claims the tap in the App Group file, so a second tap on the same phone is refused
   (`GlancePermissions.swift:164-181`). It then runs these steps:

   | step | budget | code |
   |---|---|---|
   | find the runner that owns the terminal | 3 s | `glanceFleetBudget`, `WatchLinkHost.swift:787-793, 1074` |
   | make sure the connection is up; revive a dead SSH session with `reconnectNow` | 5 s | `glanceConnectBudget`, `WatchLinkHost.swift:794-805, 907, 1008-1029` |
   | **replay the pane's stream** to check the id is still pending | 7 s | `WatchLinkHost.swift:807-850, 908` |
   | send `terminal.agent_answer`; the daemon acks within `ACK_BOUND`, 2 s | 6 s | `WatchLinkHost.swift:852-857, 909`; `hook_asks.rs:44` |

   The worst case is 21 s.
4. The SSH key is readable while the phone is locked: it's `kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly`
   (`apps/ios/FarCooler/Store.swift:12-16, 132-133`). The exception is a phone that has rebooted and not yet been
   unlocked once.

### 2.2 The suspended case works; the terminated case doesn't

- **Suspended.** The process and its scene still exist. `WatchLinkHost.fleet` was adopted at scene creation
  (`FarCoolerApp.swift:167-173`), and `entries` still lists the terminal from the last poll. So the runner is found
  at once, and `ready()` revives the link. This is the case ov-57 names, and it needs **no change on the connection
  path**.
- **Terminated** (jettisoned for memory, or swiped away). `FleetStore` is a `@StateObject` of the scene's root
  view (`FarCoolerApp.swift:119-123`), and it's adopted from a view `.task`. The codebase says a background launch
  may never build a scene (`AnswerPermissionIntent.swift:128-131`, `FarCoolerApp.swift:167-172`). Then
  `fleet == nil`, and the card says "Open Far Cooler on your iPhone, then try again."
  (`WatchLinkHost.swift:779-781`, `AK/TerminalReach.swift:84-85`). *Uncertain:* whether iOS connects a scene for a
  `LiveActivityIntent` background launch. Measure it on a device. Fixing this means building the fleet at app
  level from `PushDelegate`. That's T-iOS-4 below, a follow-up outside the smallest slice, and it would fix the
  watch's identical cold-launch miss too.

### 2.3 Fitting inside the 60 s hold

- **Skip the replay for hook asks.** It re-proves on the phone something the daemon proves atomically: a stale
  `hook-ask-` id is refused as `not_held` under the ledger lock (`crates/daemon/src/rpc.rs:2109-2127`,
  `hook_asks.rs:163-176`). Skipping it removes up to 7 s and one round trip. The worst case drops to 3 + 5 + 6 =
  14 s.
- **The timeline.** The hook starts at t=0. The blocked notice leaves at about t=1-2 s. APNs delivery is typically
  about 1 s *(not guaranteed)*. The person then has to notice, look and tap, and the answer must land before
  t=60. So in practice the person has about 45-50 s from the moment the card lights up.
- **The hold is the binding limit, not iOS.** `LONGEST_HOLD` is "the owner's number: a minute for a phone to
  answer" (`crates/agent-hooks/src/wire.rs:81-86`). A longer hold costs nothing at the keyboard: claude's dialog
  is live the whole time, and the first answer wins (ov14 spike §1). But the installed hook's `timeout` of 70 s
  (`crates/daemon/src/hook_install.rs:86-94`) and the hook binary's own cap both derive from it. Raising it
  therefore means shipping a new CLI and re-installing the hook settings. The guard test
  `claudes_permission_hook_outlives_the_longest_hold` (`hook_install.rs:1540-1552`) keeps the two consistent.
  **This is an owner decision (D1).**
- **iOS background time for an intent.** Apple doesn't publish a figure for how long a background-launched App
  Intent may run. *Uncertain.* The 14 s worst case should be measured on a device (T-iOS-5).

### 2.4 Unlocking before Allow

- A Live Activity button works from a locked phone. With `openAppWhenRun = false`
  (`AnswerPermissionIntent.swift:79-84`), anyone holding the phone can tap **Allow** on a shell command.
- `AppIntent.authenticationPolicy` (`IntentAuthenticationPolicy`: `.alwaysAllowed`, `.requiresAuthentication`,
  `.requiresLocalDeviceAuthentication`) exists. *Uncertain* whether iOS honors it for a Live Activity button on
  the Lock Screen; verify on a device.
- The policy is static per intent type. So "Deny without unlocking, Allow only after Face ID" means two intent
  types.
- **Owner decision D2.**
- A documented alternative: `UNNotificationAction` with `.authenticationRequired` on the blocked **alert**. Such
  an action runs in the background without opening the app. *(Confident.)* It would also cover a phone where Live
  Activities are off. It isn't in the slice; see D5.

---

## 3. Privacy

**What is on a locked screen today.** A locked screen already shows:

- the card's headline: "Billing · claude";
- the blocked question (`detail`, ≤ 40 characters, redacted on the runner by `farcooler_core::feed`);
- the alert body.

For an ask the running app filed (ov-54), the card also draws the option names, and for Bash the allow name
**is the command line** (`permission.rs:84`). That is already on the lock screen today whenever the app saw the
ask first.

**What this slice adds to a locked screen:**

- "Allow Bash" and "Deny", or just "Allow" and "Deny" (D3);
- a countdown.

**Redaction until unlock.**

- WidgetKit offers `.privacySensitive()` and `redactionReasons.contains(.privacy)`. *Uncertain* whether these
  redact a Live Activity's lock-screen presentation, and under which Settings toggle. Measure on a device before
  relying on them.
- If they work, the recommendation is to mark both `detail` and any store-sourced option name privacy-sensitive,
  and to keep the payload-sourced "Allow Bash" plain.
- If they don't, the only control is what we choose to draw (D3).

**What the relay sees in plaintext.** There is no end-to-end encryption, so the relay sees these fields in
plaintext:

- **Already today:** the title, `detail`, the workspace name, the agent label, the runner label, diff counts and
  the trace buckets.
- **Already stored for 24 h today:** `live_activities.detail` and `workspace` (`index.ts:1710-1747`,
  `migrations/0008_fleet_rows.sql`). This contradicts the daemon's own doc, which says `/v1/notify` "persists a
  `version` and nothing else" (`crates/daemon/src/push.rs:119-122`). That comment is stale and should be fixed in
  D-1.
- **New:** the ask id (an opaque UUID), the tool name and the hold's end time. None of them is content. The id is
  a capability only to someone who also holds an enrolled SSH key for the runner.
- **Never:** option names, `tool_input` and command lines.

---

## 4. Failure modes

### 4.1 The answer comes after the hold expired (`not_held`)

- **Today.** `not_held` arrives as `Conflict{what:"not_held"}`, whose refusal word is `resource-conflict`
  (`rpc.rs:2125`). The glance path maps every daemon refusal to "That runner turned it down, so nothing was
  sent." with the outcome `.nothingSent` (`WatchLinkHost.swift:941-951`). `.nothingSent` hands the buttons back
  (`GlancePermissions.swift:329`), so the person can tap a dead id again and again.
- **Fix, part 1: refuse locally.** If `now > until`, settle without connecting: "That question timed out. Answer it
  at the keyboard."
- **Fix, part 2: map `resource-conflict` on a `hook-ask-` id** to a new outcome, `.over`, that keeps the buttons
  off: "That question was already answered, or timed out."
- **Fix, part 3: the daemon's `kind:"ask"` clear** takes the buttons off the card when the hold expires, even if
  nobody taps.
- **What's left.** `not_delivered` means the verdict never reached the hook: the hook was killed, or the pane was
  closed. Nothing landed, and the dialog is still at the keyboard. It also gets `.over`: "Your answer didn't reach
  claude. Answer it at the keyboard."

### 4.2 The phone is offline

- **At push time.** The card never updates, so no ask and no buttons. Or the stale content from an earlier ask
  shows, and the `until` check refuses it locally.
- **At tap time.** `ready()` fails, the outcome is `.nothingSent` with "can't reach that runner", and the buttons
  come back. A retry within the hold is safe. After the hold, it's refused locally.
- **The runner can't reach the relay.** No push goes out, so there is no card. This is unchanged.

### 4.3 Two devices answer

- The ledger settles under one lock, so exactly one answer wins (`hook_asks.rs:253-270`). The loser gets
  `not_held`, which maps to `.over`: "Already answered."
- After a winning answer, the ledger records `Resolved`. The pane leaves `blocked` a couple of samples later
  (`hook_asks.rs:46-49`), the watcher sends `working`, and the relay updates **every** device's card, which gates
  its buttons on `blocked` (`AgentActivityWidget.swift:352`).
- **The keyboard against the phone.** If the keyboard answers first, claude takes it at once. The daemon only
  withdraws the ask after the dialog has been missing for 2 samples (`hook_asks.rs:185-205`). A phone answer
  inside that gap of about 2 s is written to the hook, acked as "Sent", and **silently ignored by claude** (ov14
  spike, run 2). This is safe, but the card can say "Sent 'Allow'" after the keyboard said No. Accept this, and
  note it in the copy (D4).
- **Two taps on one phone** are stopped by `claiming` (`GlancePermissions.swift:164-181`).
- **A daemon restart** drops every held ask (the hook's socket closes). The card's id then gets `not_held` and
  shows `.over`.

---

## 5. The smallest slice

The goal: **a suspended app, a locked phone and a claude TUI ask. The card shows Allow and Deny, and a tap answers
within the hold.** Terminated-app launches, per-row asks, and notification actions stay out.

### T0. Freeze the contract (first; everything else depends on it)

**Done: see "T0 contract" at the end of this file.** It supersedes the bounds in §1.1 and the migration number
in R-1. The spec §7 below is written from it.

- Add §7 "Asks on the card" to `docs/superpowers/specs/2026-09-28-workspace-ui-design.md`. It covers:
  - the `ask` object on agent notices and on `ActivityState`;
  - the `kind:"ask"` notice;
  - "absent means none";
  - the bounds in §1.1.

### Daemon (runs in parallel with Relay and AgentKit once T0 is in)

**D-1. Carry the ask on notices.**

- Files:
  - `crates/daemon/src/push.rs`: `Notification.ask: Option<WireAsk>`, set only on an agent notice whose status
    is `blocked`, and on `kind:"ask"`. `wire_body` gains an `Some("ask")` arm. The stale "persists nothing" doc is
    fixed.
  - `crates/daemon/src/hook_asks.rs`: `Held` gains `tool: String` and `until: SystemTime`. `open()` returns them,
    or a new `open_on(terminal)` does.
  - `crates/daemon/src/hook_ingress.rs:900-909`: passes `tool_name` into `hold`/`offer`.
- Tests:
  - `push::tests::a_blocked_notice_carries_its_ask`
  - `push::tests::a_working_notice_never_carries_an_ask`
  - `push::tests::an_ask_notice_with_none_omits_the_key`
  - `hook_asks::tests::open_reports_tool_and_hold_end`

**D-2. Send `kind:"ask"` on changes.**

- Files:
  - `crates/daemon/src/hook_asks.rs`: an observer, a `tokio::sync::watch` or an mpsc, fired from `offer` and
    from `end`.
  - `crates/daemon/src/watch.rs`: subscribes, and dedupes per terminal on the last ask told, the same way
    `told`/`already_told` handle the count (`watch.rs:3427-3437`). It sends only while that terminal's last-told
    status is `blocked`.
  - The blocked notice (`watch.rs:2818-2838`) reads the open ask.
- Tests:
  - `watch::tests::an_ask_offered_after_blocked_is_sent_once`
  - `watch::tests::a_hold_that_runs_out_sends_an_ask_clear_without_an_alert`
  - `watch::tests::an_ask_is_not_resent_unchanged`

### Relay (parallel; deploy first, because it's additive and auto-deploys to canary)

**R-1. Migration and row.**

- Files:
  - `services/relay/migrations/0014_row_ask.sql`: `ask_id TEXT`, `ask_tool TEXT`, `ask_until INTEGER`, all
    nullable.
  - `services/relay/src/index.ts` `rememberAgent` (`:1689-1760`): a blocked notice sets them from `body.ask`,
    validated. Absent sets them to NULL. Any other status sets NULL.
  - `test/migrations.ts`.
- Tests:
  - `it('stores a blocked notice\'s ask on its row')`
  - `it('clears the ask when the row leaves blocked')`
  - `it('drops an ask id without the hook-ask- prefix')`

**R-2. The `kind:"ask"` route.**

- File: `index.ts` `notify` (`:1115-1119`). It updates only an existing blocked row for `(account, terminal)`,
  then calls `refreshCard` with no alert.
- Tests:
  - `it('an ask notice updates the card silently')`
  - `it('an ask notice for a row that is not blocked changes nothing')`

**R-3. Put it on the state.**

- Files:
  - `services/relay/src/push.ts` `ActivityState.ask?`.
  - `index.ts` `pushActivity`, the headline copy (`:2121-2135`), and `refreshCard`. Set it before `withFleet`.
- Tests:
  - `it('the headline carries its own row\'s ask')`
  - `it('an ask moves with the headline')`
  - `it('a state with an ask stays inside STATE_BUDGET and the payload')`. This extends the test at
    `test/relay.test.ts:3689`.

### AgentKit (parallel)

**A-1. Decode the ask.**

- File: `AK/AgentActivityAttributes.swift`. Add `CardAsk { id, tool, until: Date? }` and
  `AgentCardState.ask: CardAsk?`. Decode it leniently, with `try?`, as every field there is (`:305-340`), and read
  `until` through `AgentCardClock`.
- Tests in `AgentCardStateTests.swift`:
  - `aStateWithoutAnAskDecodesAsBefore`
  - `aMalformedAskCostsTheAskNotTheCard`
  - `anAskRoundTripsThroughPersistence`

**A-2. Choose where the buttons come from.**

- File: `AK/GlancePermissions.swift`. Add `GlancePermission.fromCard(terminal:ask:)`, which synthesizes
  `allow`/`allow_once` "Allow \(tool)" and `deny`/`reject_once` "Deny". Add a pure
  `CardAskSource.permission(store:state:now:)`. It:
  - prefers the store's record when its request equals `ask.id`;
  - falls back to the card's ask;
  - returns nil when the status isn't blocked, or `now > until`.

  This moves `LeaderAsk.current`'s logic out of the widget, where CI compiles it but never runs it, into AgentKit,
  where it's tested.
- Tests in `GlancePermissionsTests.swift`:
  - `aCardAskGivesButtonsWithNoStoreRecord`
  - `aStoreRecordForTheSameAskWins`
  - `aStoreRecordForAnotherAskLosesToTheCard`
  - `anExpiredCardAskGivesNoButtons`
  - `aCardAskOnAWorkingLeaderGivesNoButtons`

**A-3. Map a closed ask.**

- Files: `AK/GlancePermissions.swift` (`GlanceAnswer.Outcome.over`, with `refusesAnotherTap` true) and
  `AK/PermissionAnswering.swift`. Map `resource-conflict` on a `hook-ask-` id to `.over`.
- Tests:
  - `aNotHeldHookAskIsOverAndKeepsTheButtonsOff`
  - `anOverAnswerIsNotRefiledAsPending`. This extends `filing` (`GlancePermissions.swift:123-133`).

### iOS widget / Live Activity (after A-1 and A-2)

**T-iOS-1. Draw from the card.**

- File: `apps/ios/FarCoolerActivity/AgentActivityWidget.swift`. `LeaderAsk.current` (`:349-364`) calls
  `CardAskSource`. Add a countdown, "Answer within 0:42", as `Text(timerInterval:)`, and a note for `.over`.
  Apply the privacy modifiers per D3.
- Tests: the unit tests are A-2's. Device check: a card with buttons and the app suspended; step 1 of T-iOS-5.

### App Intent (after A-3; can run beside T-iOS-1)

**T-iOS-2. A faster, truthful hook-ask path.**

- Files:
  - `AK/AnswerPermissionIntent.swift`: an optional `until` parameter.
  - `apps/ios/FarCooler/WatchLinkHost.swift` `answerFromGlance` (`:762-872`). For a `hook-ask-` request it:
    1. refuses locally past `until`;
    2. skips `pendingPermission`;
    3. maps `not_held` and `not_delivered` to `.over`, in `glanceFailure` (`:925-960`).
- Tests: AgentKit covers the mapping. The UI test is
  `TerminalPermissionTests.testATUIAskIsAnsweredFromTheLockScreenIntent`. It invokes the intent's handler against
  the demo host's `/asking` stand-in (`ov54-review.md`, "demo-host.sh"). It asserts `resolved=<id>:deny` for this
  ask's id, which closes T1 in `ov54-rereview.md`. Run it with `scripts/ios-ui-tests.sh`.

**T-iOS-3. Authentication (only if D2 says so).**

- File: `AK/AnswerPermissionIntent.swift`. Split the intent into `AllowPermissionIntent` with
  `authenticationPolicy = .requiresAuthentication` and a deny intent with `.alwaysAllowed`.
- Test: device only. Verify that Face ID is asked for on a locked phone.

### Verification (last)

**T-iOS-5. Device run with the app suspended.**

1. Lock the phone. The claude TUI asks. The card shows "Allow Bash" and "Deny" within about 3 s.
2. Tap Deny. claude says "Denied by PermissionRequest hook". Time the tap-to-`Resolved` in the daemon log.
3. Let 60 s pass without tapping. The buttons go away, and the dialog is still at the keyboard.
4. Answer at the keyboard, then tap on the phone within 2 s. The card says what §4.3 says it will.
5. Two phones: tap both. One shows "Sent", the other "Already answered".
6. Record the background-intent wall time, and whether `.privacySensitive` redacts while locked.

**Follow-up, not in the slice. T-iOS-4:** build `RunnerStore` and `FleetStore` at app level in `PushDelegate`, so
a terminated app's background launch can answer. The watch path benefits too.

### What can run at once

```
T0 ──┬── D-1 → D-2
     ├── R-1 → R-2 → R-3        (deploy to canary as soon as it's green)
     └── A-1 → A-2 → A-3 ──┬── T-iOS-1
                           └── T-iOS-2 (→ T-iOS-3 if D2)
                                         all → T-iOS-5
```

- Three lanes (daemon, relay and AgentKit) run in parallel, each in its own worktree.
- The two iOS tasks share `AK/AnswerPermissionIntent.swift` only if D2 splits the intent. Otherwise they are
  disjoint.

---

## 6. Decisions for the owner

- **D1. The hold length.** Keep 60 s, "your number" (`wire.rs:81-86`), or raise it to about 5 min? Once the card
  lights up there are about 45-50 s left. A longer hold costs nothing at the keyboard (ov14 spike), but it needs a
  new CLI and a re-install of the hook settings (`hook_install.rs:86-94`).
- **D2. Unlocking to Allow.** Should Allow need Face ID on a locked phone? It's possible only if iOS honors
  `authenticationPolicy` for Live Activity buttons, which is unverified. The recommendation is yes for Allow and no
  for Deny.
- **D3. What the locked card says.** Options:
  - "Allow Bash" from the payload;
  - plain "Allow";
  - also hide the store-sourced "Allow touch x" (the command line, shown today) until the phone is unlocked.
- **D4. The copy** for `.over`, for a timeout, and for the case where the keyboard answered before the phone
  (the phone says "Sent", but claude ignored it).
- **D5. Scope.** Are these follow-ups or part of ov-57?
  - the terminated-app launch (T-iOS-4);
  - Allow/Deny actions on the blocked **alert**, with `.authenticationRequired`, for phones with Live
    Activities turned off.
- **D6. The relay stores the ask id and tool for up to 24 h** beside the `detail` it already stores. Is that
  acceptable, or should the ask columns be cleared as soon as the hold ends?

---

## T0 contract

Frozen 2026-09-29. It supersedes §1 and §5 wherever they differ. Every file:line below was checked against
`main` at `8b6ecdeb`, except the ov-61 citations, which were checked against the `ov-61` worktree at `6b962e6d`.
ov-61 moves `services/relay/src/index.ts` down by about 45-55 lines, so each relay citation also names its function.

Items marked **provisional (ov-57)** are the coordinator's recommendations. They stand until the owner answers
D1-D6 on the card. Changing one changes only the clause it's marked on.

**Sequencing with ov-61.** ov-61 adds migration `0013_daemon_install.sql` (`daemons.install_id`) and an optional
`install` field on every daemon notice (`ov-61:crates/daemon/src/push.rs:227`, stamped on every kind, `:357`). This
work lands after it. Its migration is therefore **0014**, and it keys runners the way ov-61 does.

### C1. The ask object (one shape, three places)

The same object is used on a daemon notice, on the relay's stored row (as three columns) and on the Live
Activity's content state:

```json
"ask": { "id": "hook-ask-0199a1b2-…", "tool": "Bash", "until": 1790551063000 }
```

| field | type | rule | source |
|---|---|---|---|
| `id` | string, **required** | Matches `^hook-ask-[0-9A-Za-z-]{1,55}$`, so it's ≤ 64 bytes. The daemon always makes `hook-ask-` + a hyphenated UUID v7, which is 45 bytes. | `crates/daemon/src/hook_asks.rs:37, 108` |
| `tool` | string, optional | claude's `tool_name` from the PermissionRequest payload. Matches `^[A-Za-z0-9_.:-]{1,64}$`. 64 rather than §1.1's 32, because MCP names (`mcp__server__tool`) run long. The daemon omits a value that doesn't match. The relay drops a non-matching `tool`, **not** the ask. | `hook.payload["tool_name"]`, read at `crates/daemon/src/hook_ingress.rs:848` |
| `until` | integer, **required** | Unix **milliseconds**, the runner's wall clock: `Held.at + hold`, where `hold` is the ingress's hold, `LONGEST_HOLD` in production. It's a finite integer > 0. The relay treats anything else as no ask. | `hook_asks.rs:76-89` (`Held.at` at `:82`); `crates/daemon/src/hook_ingress.rs:252, 924`; `crates/agent-hooks/src/wire.rs:86` |

- **Bounds.** Absent `ask` always means "no ask open now". There is never `"ask": null`. An ask whose `id` or
  `until` fails its rule is treated as absent, everywhere.
- **`until` is conservative.** `Held.at` is stamped in `hold()`. The hold's timer starts later, after the reply
  write and the offer (`hook_ingress.rs:900-924`). So the real end is ≥ `until`, and a phone that refuses at
  `until` refuses early, never late.
- **Clock skew** between the runner and the phone isn't corrected. A tap past a skewed `until` is refused
  locally. A tap inside a skewed one reaches the daemon, which refuses `not_held`, and then C5 applies.
- **Never on the wire:** option names, `tool_input` and command lines. `permission_options` names the Bash allow
  button with the raw command (`crates/agent-core/src/permission.rs:81-94`; the test at `:110-112` asserts
  `"Allow touch x"`).
- **The hold stays 60 s** (`LONGEST_HOLD`, `wire.rs:86`). **Provisional (ov-57), D1.**

### C2. The daemon notice

**C2.1 An agent notice** (`kind` absent) gains an optional `ask`:

- **When present.** It's present iff `status == "blocked"` **and** `HookAsks` holds an **offered** ask on that
  terminal. That's the same filter `open()` applies (`hook_asks.rs:232-237`), and the source `needs_you` reads
  (`crates/daemon/src/needs_you.rs:97`).
- **Never** on a `working` or `done` notice.
- **Where.** It's set on the blocked `Outgoing` the watcher builds (`crates/daemon/src/watch.rs:2820-2837`), and
  serialized by `wire_body`'s `None` arm (`crates/daemon/src/push.rs:393-408`).

**C2.2 `kind: "ask"`** is a new notice kind:

```json
{ "kind": "ask", "terminal": "<uuid>", "ask": { … }?, "needsYou": 1?, "install": "<uuid>"?, "version": "…" }
```

- **No `title` or `subtitle`.** The relay's title check applies only to alerting kinds (`index.ts:1004-1006`,
  `notify`).
- **Serialization.** A new `Some("ask")` arm in `wire_body` carries `terminal` and `ask` on top of `shared`. Today
  that arm would fall through to `Some(_) => shared` and lose the terminal (`push.rs:416`).
- **When it's sent.** Only while that terminal's **last notice that landed** was `blocked`, and only when the open
  offered ask's `id` differs from the last `id` told for that terminal (`None` counts as an id). Two triggers:
  - an ask is offered after the blocked notice left;
  - the held ask ends while the pane is still blocked: the hold ran out (`hook_ingress.rs:924-929`), or it was
    superseded or withdrawn.
- **Deduplication.** It's per terminal, recorded only once the notice has landed, in the same way `told` and
  `already_told` handle the count (`watch.rs:3428-3437`). An unchanged ask is never re-sent.
- **Removal.** A non-blocked notice for the terminal clears its last-told ask.
- **It never alerts.** The relay alerts only for no kind or `decision` (`index.ts:1005`), and only then sends
  `sendPush` (`index.ts:1059`).
- **Older relays.** A relay that predates this routes any other kind to `refreshCard` (`index.ts:1115-1119`), so
  sending `kind:"ask"` to one is harmless.

**C2.3** The stale sentence "`/v1/notify` persists a `version` and nothing else" (`push.rs:120`) is rewritten to
list what the relay stores for 24 h: the row columns of `0008_fleet_rows.sql`, 0011 and 0014.

### C3. Relay storage: migration `0014_row_ask.sql`

**Additive only.** It adds three nullable columns and nothing else, the same as 0002-0013:

```sql
ALTER TABLE live_activities ADD COLUMN ask_id TEXT;
ALTER TABLE live_activities ADD COLUMN ask_tool TEXT;
ALTER TABLE live_activities ADD COLUMN ask_until INTEGER;
```

No index, no backfill and no NOT NULL. The previous worker never writes or reads them. `live_activities` is one row
per `(account_id, terminal)` (`migrations/0003_live_activities.sql:37-50`), and `AgentRow` (`index.ts:1518-1536`)
gains `ask_id`, `ask_tool` and `ask_until`.

**Writes:**

- **W1. An agent notice** (`rememberAgent`, `index.ts:1689`). The upsert writes all three from the validated
  `body.ask` when `status == "blocked"`. It writes NULL for all three when `status != "blocked"` or the ask is
  absent or invalid.
  - The writes are plain `= excluded.ask_*`, **never** `COALESCE`. An absent ask must clear.
- **W2. A `kind:"ask"` notice** runs one `UPDATE … SET ask_id, ask_tool, ask_until`: the validated values, or NULL
  for all three when `ask` is absent. It applies only where all of these hold:
  - `account_id = daemon.account_id`;
  - `terminal = body.terminal`;
  - `status = 'blocked'`;
  - **the row's runner is the sender's runner**, per ov-61's rule (`ov-61:services/relay/src/index.ts:1691`,
    `runnerOf`). The sender's runner is `install:<id>` when the notice's `install` passes `installId`
    (`ov-61:…/index.ts:835`), and `daemon:<daemons.id>` otherwise. The row's runner is found through
    `live_activities.daemon_id` → `daemons.install_id`. In SQL:
    `daemon_id IN (SELECT id FROM daemons WHERE account_id = ? AND (id = ? OR (? IS NOT NULL AND install_id = ?)))`.
    A row with `daemon_id IS NULL` (pre-0012) never matches.

  With no matching row, it writes nothing and still answers 200. After a write that matched a row, it calls
  `refreshCard`.
- **W3. Clear on resolve.** The ask columns are cleared by W1 when the row leaves `blocked`, and by W2 when the
  ask ends. **Provisional (ov-57), D6.**
- **W4. 24 h max.** The ask columns never outlive their row. `readFleet` already deletes rows older than
  `ROW_RETENTION_MS` = 24 h (`index.ts:1380, 1604-1606`). The same purge also runs
  `UPDATE live_activities SET ask_id = NULL, ask_tool = NULL, ask_until = NULL WHERE account_id = ? AND ask_until < ?`
  with `now`, so an expired ask is gone at the next read even if no W2 arrived. **Provisional (ov-57), D6.**
- **W5. Nothing logs** `ask_*`, as nothing logs a body today.

### C4. The Live Activity content state

**C4.1 Relay** (`ActivityState`, `services/relay/src/push.ts:243-345`): it gains `ask?: { id: string; tool?: string;
until: number }`, the **headline's** ask only. `ActivityRow` doesn't change.

- **Composing.** `state.ask` is set from the headline row. It's present iff that row's `status == 'blocked'`,
  `ask_id` and `ask_until` are non-NULL, and `ask_until > now`.
- **Where it's set:**
  - in `pushActivity`, on both sides of the headline copy (`index.ts:2124-2133`). That's the notice's own row
    when it heads the card, and the copied row when another row does;
  - in `refreshCard`'s state literal (`index.ts:2400-2408`).
- **Before `withFleet`.** Both are set **before** `withFleet` (`index.ts:1992`), so the row-by-row budget check
  (`index.ts:2021`, `STATE_BUDGET` = 3 KB at `:1448`) counts the ask. The worst ask is about 150 bytes; it can
  cost the card rows, never the card.

**C4.2 AgentKit** (`AgentCardState`, `apps/shared/AgentKit/Sources/AgentKit/AgentActivityAttributes.swift`):

- **New fields.** It gains `ask: CardAsk?`, where `CardAsk { id: String, tool: String?, until: Date }`, and
  `workspace: String?`. The relay already sends `workspace` (`push.ts:259`), but `CodingKeys` doesn't decode it
  today (`AgentActivityAttributes.swift:269-273`). The locked card needs it (C5).
- **Decoding.** It's lenient, like every field in `init(from:)` (`:304-339`):
  - a missing or malformed `ask`, or one without `id` or `until`, decodes as `nil` and never throws;
  - `until` goes through `AgentCardClock.date` (`AgentCardRows.swift:33-45`);
  - a bad `tool` becomes `nil`.
- **Encoding** writes `until` back as milliseconds (`AgentCardClock.number`), so a persisted card round-trips.

### C5. What a tap does, and what the card says

- **Where buttons come from.** The widget shows Allow and Deny only when the headline is `blocked` and `now <
  ask.until`:
  - from `GlancePermissionStore` when its record's request equals `ask.id`;
  - otherwise from `ask` itself, via synthesized options `allow`/`allow_once` and `deny`/`reject_once`.
- **The locked card shows the tool name and workspace only, never the command.**
  - With an `ask`, the ask block draws `"<tool> · <workspace>"`, or whichever of the two is present.
  - The buttons read exactly **"Allow"** and **"Deny"**, and the intent's `optionName` is the same word. They never
    carry a store option name, which for Bash is the command line.
  - The block replaces `detail` while it's drawn.
  - **Provisional (ov-57), D3.**
- **Allow requires an unlock; Deny doesn't.**
  - Allow is a separate `LiveActivityIntent` with `authenticationPolicy = .requiresAuthentication`.
  - Deny keeps `.alwaysAllowed`.
  - Both keep `openAppWhenRun = false` (`AnswerPermissionIntent.swift:84`).
  - Whether iOS honors the policy on a Lock Screen Live Activity button is verified in T-iOS-5.
  - **Provisional (ov-57), D2.**
- **The wire answer is unchanged:** `terminal.agent_answer` with `request = ask.id` and `option = "allow" | "deny"`.
  The daemon refuses a stale id as `not_held` and a verdict that never landed as `not_delivered`. Both are
  `resource-conflict` (`crates/daemon/src/rpc.rs:2125-2126`).
- **Failure copy.** **Provisional (ov-57), D4.** Each case below sets the new outcome `.over`, which keeps the
  buttons off (`refusesAnotherTap` is true; today only `.nothingSent` hands them back, `GlancePermissions.swift:329`).

  | case | copy |
  |---|---|
  | `now ≥ until` at tap time (refused locally, nothing is sent) | "Too late here. Answer it in the terminal." |
  | `not_held` with `now < until` | "Answered on another device." |
  | `not_held` with `now ≥ until` | "Too late here. Answer it in the terminal." |
  | `not_delivered` | "Too late here. Answer it in the terminal." |

  Unreachable runners keep today's `.nothingSent` copy and give the buttons back.
- **Out of scope** (**provisional (ov-57), D5**):
  - the terminated-app launch (T-iOS-4);
  - `UNNotificationAction` buttons on the blocked alert.

### C6. Acceptance: one test per clause

**Daemon (`crates/daemon`):**

- `push::tests::a_blocked_notice_carries_its_ask`
- `push::tests::a_working_notice_never_carries_an_ask`
- `push::tests::an_ask_notice_carries_terminal_and_omits_an_absent_ask` (no `"ask": null`)
- `push::tests::an_ask_notice_carries_the_install_id` (with ov-61)
- `hook_asks::tests::open_reports_tool_and_hold_end` (`until == at + hold`)
- `watch::tests::an_ask_offered_after_blocked_is_sent_once`
- `watch::tests::a_hold_that_runs_out_sends_an_ask_clear_without_an_alert`
- `watch::tests::an_ask_is_not_resent_unchanged`
- `watch::tests::no_ask_notice_follows_a_working_notice`

**Relay (`services/relay/test`):**

- `migrations`: 0014 applies after ov-61's 0013 and only adds columns.
- `it('stores a blocked notice\'s ask on its row')`
- `it('clears the ask when the row leaves blocked')`
- `it('clears the ask when a blocked notice carries none')`
- `it('drops an ask id without the hook-ask- prefix')`
- `it('keeps the ask but drops a tool outside the vocabulary')`
- `it('an ask notice updates the card silently')` (no `sendPush`)
- `it('an ask notice for a row that is not blocked changes nothing')`
- `it('an ask notice from another runner changes nothing')`
- `it('an ask notice from a re-paired token of the same install applies')`
- `it('an expired ask is not put on the state and is nulled on read')`
- `it('the headline carries its own row\'s ask')`
- `it('an ask moves with the headline')`
- `it('a state with an ask stays inside STATE_BUDGET and the payload')`, which extends
  `test/relay.test.ts:3689`

**AgentKit:**

- `aStateWithoutAnAskDecodesAsBefore`
- `aMalformedAskCostsTheAskNotTheCard`
- `anAskRoundTripsThroughPersistence`
- `theWorkspaceDecodes`
- `aCardAskGivesAllowAndDenyWithNoCommand`
- `anExpiredCardAskGivesNoButtons`
- `aNotHeldBeforeUntilSaysAnsweredElsewhere`
- `aNotHeldAfterUntilSaysTooLate`
- `aNotDeliveredSaysTooLate`
