# Notifications about tasks, not agents: design

Date: 2026-10-02. Status: design, not built. Card: "Notifications about tasks, not agents".

The owner asked: "can we also have better notifications for task related notifications? would love to see if there's a way to replace/augment the existing agent notifications with status updates instead".

## The agreed direction (restated, so this doc can be checked against it)

- One notification thread per task on the Mac and the phones. The thread is the task.
- The title is `<key> <title>`, for example `ov-90 Wake the agent on an answer`. The body is the status update: `Moved to In Review · 3 files changed`, `Needs your decision · Which PDF library?`, `Blocked on ov-88`, `Done`.
- Actions: Open, a deep link to the task. A decision also gets its answer options as notification actions, the same options the phones' Needs You rows already offer as buttons.
- An agent's own notifications (finished a turn, needs permission, asking a question) fold into its task's thread when the agent works on a task, worded in terms of the task. An agent with no task notifies as it does today.
- A newer update for a task replaces the older one, because both use the same request identifier. Bursts within a few seconds coalesce into one.
- Per-event settings: Needs decision, Needs review, Blocked, Done and New task. The first three are on by default; Done and New task are off.

---

## 1. How notifications work today

### 1.1 The daemon decides and pushes

All push content is composed in `crates/daemon/src/watch.rs` and sent through `crates/daemon/src/push.rs`.

**Agent notices.** These come from activity transitions in the sampling loop.
- `notification()` (`watch.rs:335-421`) maps `AgentActivity` to a `Notice`: Blocked becomes `"{label} needs you"` with the question or "Waiting for your answer"; Done becomes `"{label} finished"` or `"{label} failed"`; Working becomes a silent card update. The `Notice` struct is at `watch.rs:140-185`.
- `Who::name()` (`watch.rs:431-453`) prefixes the workspace: `"Billing · claude"`, `"Billing Orchestrator"`.
- The transition site is `watch.rs:4951-5000`. It asks `attention()` (`watch.rs:2631-2648`): an agent that a client claims to be watching (`terminal.watching`, `anyone_watching` at `watch.rs:2555`) gets no alert. Done retires the card, and Blocked holds it.
- `push_if_paired` (`watch.rs:2732-2745`) and `push_notice` (`watch.rs:2828-2898`) send a detached `push::notify` for each notice, stamped with the needs-you count and the card stats.

**Decision notices.** These are about tasks.
- `task_ops::set_status` (`crates/daemon/src/task_ops.rs:505-528`) calls `watcher.announce_decision(&task)` only when a task enters Needs Decision (`task_ops.rs:522-525`).
- `announce_decision` (`watch.rs:3679-3718`) sends `kind:"decision"` with the title `"Billing · bil-7 needs a decision"` (`decision_title`, `watch.rs:468-473`) and the latest QUESTION note's body as the subtitle.
- Nothing else about a task ever pushes. Moving to In Review, to Done or to blocked, and filing a new task, all reach clients only as the `TaskChanged` event (`announce_task_changed`, `watch.rs:3864-3878`) and the needs-you re-read (`announce_needs_you`, `watch.rs:3780-3812`, debounced 250 ms at `watch.rs:526`).

**Other kinds.** `kind:"count"` (`watch.rs:3639-3672`, at most one per 5 s window, `watch.rs:484`) and `kind:"ask"` (`watch.rs:2934-2985`) only move the lock-screen card.

**The wire contract** is `push::Notification` (`push.rs:130-350`):
- `kind` (`push.rs:141`), `task` (`push.rs:212`), `runner` (`push.rs:241`, decisions only) and `ask` (`push.rs:251`).
- `wire_body` (`push.rs:479-516`) enforces which kind carries what.
- An `ask` never carries option names, because "for Bash the allow option's name IS the command line" (`push.rs:246-249`).

### 1.2 The relay fans out

**`/v1/notify`** (`services/relay/src/index.ts:1100-1275`):
- Only an agent notice (no kind) or a `decision` alerts (`index.ts:1109`). An unknown kind gets no alert and only refreshes the card (`index.ts:1266-1268`). This is what makes it safe for a newer daemon to send new kinds to an older relay.
- It loops over every device on the account (`index.ts:1174-1236`). The only per-device filter is `notify_on_done = 0`, which skips agent `done` (`index.ts:1202`; column from migration `0007_notify_on_done.sql`; registered through the upsert at `index.ts:340-356, 385`).

**APNs** (`sendApns`, `services/relay/src/push.ts:169-216`):
- `apns-priority: 10` and `interruption-level: time-sensitive` on every alert (`push.ts:185, 189`), including a plain "finished".
- `thread-id` is `payload.terminal` (`push.ts:192`), which is the empty string for a decision.
- `mutable-content: 1` so the service extension runs (`push.ts:198`).
- There is no `apns-collapse-id`, so every push is a new notification and nothing replaces anything.
- There is no `category`.

**Live Activity** pushes are separate (`sendLiveActivity`, `push.ts:545+`; `pushActivity` and `refreshCard` in `index.ts`). They are keyed by terminal rows, which a task is not (`push.rs:204-208`).

**FCM** (`sendFcm`, `push.ts:704-760`):
- It's a `notification` message, so Firebase draws the card itself when the app is backgrounded.
- `android.notification.channel_id` comes from `androidChannel(status, kind)` (`push.ts:700-702`): blocked or decision goes to `agents.blocked`, anything else to `agents.done`.
- There is no `tag`, so nothing replaces anything when the app isn't running.
- The data payload is `terminal`, `status`, `kind`, `task` and `runner`.

### 1.3 Mac

Local notifications only:
- `Notifier.report` (`apps/macos/Sources/FarCooler/Notifications.swift:132-165`) runs on every `TerminalChanged` event (`DaemonClient.swift:668-673`).
- It posts for Blocked and Done with identifier `"<terminal>-<activity>"` and `threadIdentifier = terminal.id` (`Notifications.swift:158-164`). A failed exit uses `"<terminal>-failedRun"` (`Notifications.swift:177-211`).
- The words come from `Notifier.words` (`WorkspaceActions.swift:156-184`).

Banner suppression:
- `presentation(terminalID:watching:presence:)` (`Notifications.swift:61-74`) reads the thread identifier back as a terminal id to hide the banner for a pane on screen.

Settings:
- `notifications.enabled` and `notifications.onDone`, both defaulting to on (`Preferences.swift:142-146`, UI at `Preferences.swift:462-476`).
- Only `onDone` reaches the relay (`ContentView.swift:266`).

No task notification exists on the Mac.

**Today's duplicate:**
- The Mac also registers for APNs (`Notifications.swift:124`), and the relay sends to every device on the account.
- So a blocked agent on a paired runner, not watched, reaches a Mac twice: once as the local `"<terminal>-blocked"` and once as a remote push with an APNs-assigned identifier.
- The two share a thread but don't replace each other. Nothing in the code dedups them.

### 1.4 iOS

**The local `Notifier`** (`apps/ios/FarCooler/Notifications.swift:89-166`) is a port of the Mac's: the same identifiers (`:162-165`), the same thread (`:159`), and a feed of polled terminals (`Connection.swift:1005`).

**Taps:**
- `ForegroundPresenter.didReceive` (`Notifications.swift:197-206`) handles only the default action and builds a `PushTap` (`apps/shared/AgentKit/Sources/AgentKit/ShellNavigation.swift:1982-1998`).
- `kind == "decision"` with `task` opens the task (via `PhoneDecisionLink`). Anything else falls back to `userInfo["terminal"]`, then to the thread id.

**No categories or actions.** There's no `UNNotificationCategory` or `UNNotificationAction` anywhere in the apps.

**The decision path is in-app only:**
- `NeedsYouScreen` draws the options as buttons, at most `TaskQuestion.buttonLimit = 3` (`NeedsYouScreen.swift:562-594`; `TaskQuestion.swift:26-32`).
- It sends `task.note kind:answer` (`Connection.swift:1498-1503`).
- The options are the QUESTION note's `extra.options` (`crates/daemon/src/needs_you.rs:195-199`).

**The service extension** (`apps/ios/FarCoolerNotify/NotificationService.swift:20-77`) only folds `terminal` and `status` into the widget snapshot. It returns early when either is missing (`:30-33`), so decisions don't touch it.

**Settings:**
- `notifyOnAttention` and `notifyOnDone`, both defaulting to on (`Settings.swift:51-64`, UI `:178-179`).
- Only `onDone` reaches the relay (`FarCoolerApp.swift:203`).

**Answering in the background exists already** for the Live Activity's permission buttons: `acceptAnswersFromGlances`, `answerFromGlance` (`WatchLinkHost.swift:770-810`). It's launched into the background, connects, verifies, and sends once.

### 1.5 Android

**The local `Notifier.report`** (`apps/android/app/src/main/java/com/farcooler/notify/Notifier.kt:130-180`):
- Posts under `notify(terminal.id.hashCode(), …)` with `setGroup(terminal.id)` (`:173-181`).
- Uses two channels, `agents.blocked` and `agents.done` (`:95-123`).
- Copy comes from `NotificationCopy.of` (`NotificationCopy.kt:72-113`).

**Pushes in the foreground** (`FarCoolerMessagingService.onMessageReceived`, `FarCoolerMessagingService.kt:61-124`):
- Posted under `NotificationCopy.postedAs` (`NotificationCopy.kt:122-126`): the terminal, else `"task:<key>"`, else the title.
- Grouped by terminal or `"task:<key>"` (`:116`).
- No actions anywhere.

**Taps** go through `MainActivity.handleIntent` (`MainActivity.kt:72-93`).

**Probable existing bug: a background-drawn decision tap doesn't open the task.**
- The relay always sends `terminal: body.terminal ?? ''` (`index.ts:1210`), and `sendFcm` copies it into `data.terminal` (`push.ts:753`).
- Firebase puts `terminal=""` into the launch intent.
- `handleIntent` reads `terminal` as `""`, which isn't null (`MainActivity.kt:83-84`), so the `terminal == null && kind == decision` branch (`:88`) never runs.
- The foreground path guards with `isNotEmpty()` (`FarCoolerMessagingService.kt:90`); the background path doesn't.
- This needs checking on a device. Phase 2 fixes it either way, because task notices will omit `terminal`.

**Settings:** `notifyOnAttention` and `notifyOnDone` (`data/Settings.kt:53-54`). Only `onDone` reaches the relay (`account/PushRegistration.kt:119`).

### 1.6 What this adds up to

- Every notification is about a pane.
- A task speaks once, when it enters Needs Decision, and on a thread of `""`.
- Nothing ever replaces anything remotely: no collapse id, no tag.
- The Mac double-delivers.
- The only setting any relay knows is "finishes or fails".

---

## 2. Decisions

### Q1. Where task status and agent events join: who knows "this agent works on task X"?

**The daemon, and only the daemon.**

**The link is `Terminal.task_id`**:
- Defined at `crates/store/src/models.rs:313-315`: "The task this terminal was opened for… Set at creation and never moved".
- Written by `task dispatch` through `terminal.create`, and exported to the pane as `FARCOOLER_TASK` (`crates/daemon/src/service.rs:879-886, 3970-3973`).

**`needs_you::assemble` already performs exactly this fold for the needs-you list:**
- A terminal's signals are about its task unless the terminal is an orchestrator (`needs_you.rs:325-331`).
- One item per subject, with the most urgent signal winning (`needs_you.rs:339-357`).
- The notification fold should be the same rule, so the needs-you row and the notification can't disagree about what an agent's question is "about".

**The secondary link is `Task.worktree_id`** (`models.rs:595-597`):
- A task's lane. ov-90's recipient search falls back to the lane's terminals (see Q7).
- For notifications, the fallback is used only when the terminal's worktree has exactly one open task and the terminal isn't an orchestrator.
- With two tasks on one lane, a guess files an agent's question under the wrong thread. That's worse than leaving it on the agent's own thread.

**Decision:**
- Add one function, `task_link::task_of(store, &Terminal) -> Option<Task>`, in a new `crates/daemon/src/task_link.rs`.
- `needs_you` (subject choice), the notice composer and, after it lands, ov-90's recipient search (in reverse) all call it.
- Clients never do the join. The Mac's `Terminal` does carry `task_id` on the wire (`wire.rs:334`), but a client-side join is a second copy of the rule.

**A note on this repo's own workflow.** The coordinator dispatches Claude Code subagents that aren't Far Cooler terminals, so they have no `task_id` and nothing folds. Their news reaches the board as the coordinator's status moves (`--actor manager`), and those are exactly the task events below. The fold matters for agents started with `task dispatch` or Start Task.

### Q2. Who decides and coalesces: the daemon, so every client agrees, or each client?

**The daemon decides, words, identifies and coalesces. Each device filters by its own settings.**

Why the daemon:
- It's the only place that sees status writes (`task_ops`), notes, blocks and agent activity together.
- It's the only producer of pushes (`watch.rs:2828`).
- Phones are usually not connected, so per-client logic would run only when the app is open, which is precisely when notifications matter least.
- Replacement depends on identical identifiers on every path (local post, APNs, FCM). One producer means one identifier.
- This continues the pattern that's already there: "The status the phone acts on is decided HERE, next to the sentence the person reads" (`watch.rs:302-306`).

**The composer is a new `crates/daemon/src/watch/task_notice.rs`** (ov-90 already introduces the `watch/` submodule directory). It has three parts.

**1. Intake.**
- `task_ops::{create,set_status,note,block}` and `workspace_ops` move (`workspace_ops.rs:142`) call `watcher.task_event(task, TaskEvent, actor)`.
- The agent transition site (`watch.rs:4963`) calls `watcher.agent_event(terminal, next, quoted)` in place of `push_if_paired` when `task_of(terminal)` is `Some`.
- Each intake marks the task dirty with the event's class and a timestamp.

**2. Window.**
- A per-task trailing window of **3 s**, capped at **10 s** from the first event, so a steady trickle still sends.
- It works like `COUNT_NOTICE_EVERY` (`watch.rs:476-484`): open on the first event, read the state at the close.
- A Needs Decision or agent-Blocked event for a task with nothing pending skips the window and sends at once. Waiting 3 s on "it's stopped and waiting for you" buys nothing.

**3. Compose at close.**
- Read the task's current state, not the event log, so a burst like In Progress, then In Review, then Done sends `Done`.
- A Needs Decision that was answered inside the window sends nothing.
- Pick the most urgent class still true, then write one body:

| Class (setting) | Trigger at close | Body |
|---|---|---|
| decision (Needs decision) | status is Needs Decision with an unanswered QUESTION (same test as `needs_you.rs:265-280`) | `Needs your decision · <question, cut to feed::SAID_WIDTH>` |
| decision (Needs decision) | the task's agent is Blocked (agent question or permission) | `<agent> needs you · <blocked question or "Waiting for your answer">` |
| blocked (Blocked) | a new `task.block` edge (not a clear) whose blocker isn't Done | `Blocked on <blocker key>`, plus ` · <reason>` when given |
| blocked (Blocked) | the task's agent's turn failed | `<agent>'s last turn didn't finish` |
| review (Needs review) | entered In Review | `Moved to In Review · N files changed` (`review::Counts::Known(files, …)`, `review.rs:133-134`; dropped when unknown, never `0 files`) |
| done (Done) | entered Done | `Done`, plus ` · <last DECISION note, cut>` when one was written since In Review |
| done (Done) | the task's agent finished a turn and the task didn't move | `<agent> finished · <said>` |
| new (New task) | `task.create` by the manager or an agent | `New in <workspace> · filed by <manager or agent>` |

- Title: `<key> <title>`, with the title cut to 60 characters.
- The workspace isn't in the title: the key's prefix already places it. The runner name is appended to the body only when the account has more than one runner (as `NotificationCopy.body` does, `NotificationCopy.kt:181-192`).

**The fold for agents on a task (owner's direction):**
- An agent turn finishing is filed under "Done" because it's the same kind of news ("something ended, nothing is waiting"). So it's off by default.
- The owner's Mac stops buzzing for every turn of a dispatched agent while still hearing about the task's Needs Review.
- This is a behavior change for task-bound agents only. Section 5 covers it.

**Per device.** Each notice carries its `event` class. The relay drops it for devices that turned that class off (Q4), and local notifiers drop it the same way.

### Q3. Identifiers and threads

A task key is unique only on its own runner (`push.rs:236-238`), so every id includes the runner id (`Host.runner_id`, the `stable_host_id(install)` that decisions already carry, `push.rs:511`).

**Notice id:**
- `t:<runner_id>:<task key>`. A UUID is 36 characters, so this is 39 plus the key.
- APNs limits `apns-collapse-id` to 64 bytes. When the string would exceed 64, use `t:<first 16 hex chars of sha256(runner_id + ":" + task uuid)>`.
- The daemon builds it, as `notice_id` on the wire. The relay validates `^[A-Za-z0-9:._-]{1,64}$` and drops (does not 400) anything else, falling back to no collapse id.

| Surface | Replace key | Thread / group | Today |
|---|---|---|---|
| Mac local `UNNotificationRequest` | `identifier = notice_id` | `threadIdentifier = notice_id` | `"<terminal>-<activity>"` / terminal (`Notifications.swift:158-164`) |
| APNs (iOS, watchOS mirror, Mac) | header `apns-collapse-id: notice_id`, which becomes the delivered request's identifier | `aps.thread-id = notice_id` | no collapse id; `thread-id = terminal` (`push.ts:192`) |
| FCM (Android, app not running) | `android.notification.tag = notice_id` | none: FCM v1 has no group field on a notification message; one tag per task already gives one card per task | no tag (`push.ts:715-728`) |
| Android local (foreground and local) | `notify(tag = notice_id, id = 0, …)`, the same (tag, id) pair Firebase uses for a tagged card, so the two replace each other | `setGroup(notice_id)` | `notify(terminal.hashCode())`, group terminal (`Notifier.kt:173-181`) |

How the replacement works:
- Because the local and remote identifiers are equal, a local post and a push about the same task replace each other instead of stacking. This retires the Mac's double delivery (1.3) for task notices.
- Replacement alerts again on Apple, so the daemon's coalescing window is what keeps a burst to one sound.
- The relay also stops sending `interruption-level: time-sensitive` for everything. It takes `level` from the notice: `time-sensitive` for decision, `active` for review and blocked, `passive` for done and new.

**Agent notices with no task** keep their current ids and threads. Phase 4 also gives them a collapse id, `a:<terminal>`, and the matching local identifier, to end the Mac's duplicate for them too.

**Mac banner suppression** stops reading `threadIdentifier` as a terminal id (`Notifications.swift:70`). It reads `userInfo["terminal"]` and `userInfo["task"]` instead, and suppresses the banner when that task's card or terminal is on screen in any window.

### Q4. Where the settings live, and their defaults

**Per device, filtered at the relay.**
- The phone in a pocket and the Mac on a desk legitimately differ. `notify_on_done` already set this precedent, filtered per device inside the loop (`index.ts:1194-1202`).
- Per runner would make the owner set the same five switches once per runner for no gain.
- Per workspace is a real future want ("mute the Docs workspace"), but it belongs on the daemon, beside ov-90's per-workspace `wake_on_answer`. It's left out of v1.

Relay storage:
- Migration `services/relay/migrations/0017_notify_events.sql` adds `devices.notify_events TEXT`.
- The value is a comma set of `decision,review,blocked,done,new`.
- NULL means the defaults. It's COALESCEd in the upsert like every optional column (`index.ts:312-356`), and sent as `notifyEvents: [String]` in the registration (`Registration`, `index.ts:431+`).

Defaults (NULL): `decision`, `review` and `blocked` on; `done` and `new` off.

Client storage and UI:
- One key per class, each with its own default, read by the local notifier and sent on registration through `PushRegistration` (`PushRegistration.swift:37-45`; the Android `PushRegistration.kt:119`).
- The toggles re-register on change, as `notifyOnDone` already does (`Preferences.swift:470-476`).
- The UI gets a "Tasks" section with "Needs a Decision", "Ready for Review", "Blocked", "Done" and "New Task", in that order.
- The existing two toggles move under an "Agents Without a Task" heading, unchanged.
- The master `notifyOnAttention` keeps its meaning of "all notifications from this app". On the relay it becomes a real setting at last: Phase 3 sends `notifyEvents: []` when it's off. Today it's local-only (1.3–1.5), so a phone with it off still receives pushes.

On Android, each class also gets a notification channel, so the OS's own per-channel switch agrees with the in-app one:
- `tasks.decision` (HIGH)
- `tasks.review` (DEFAULT)
- `tasks.blocked` (DEFAULT)
- `tasks.done` (LOW)
- `tasks.new` (LOW)

`androidChannel` (`push.ts:700`) picks the channel from `event` when present.

### Q5. Migrating from today's agent notifications

Ordered so no build combination double-notifies or goes silent:

**1. Apps ship first.**
- `PushTap` and `handleIntent` learn `kind:"task"`.
- The phones learn the actions and the new settings.
- Old relays and daemons never send `kind:"task"`, so nothing changes yet.

**2. The relay ships second.** It accepts `kind:"task"` as an alerting kind (`index.ts:1109`), the `notice_id`, `event`, `level` and `options` fields, and `notify_events`. It also accepts a new `alert: false` flag on agent notices: "card only, no banner".

**3. The daemon ships last.**
- For a terminal where `task_of` is Some, the agent transition still sends its agent notice, because the Live Activity row needs it. It now sends it with `alert: false`, and the alert goes out as the task notice instead.
- A relay older than step 2 ignores `alert: false` and would alert twice. The runner and relay are both ours, and relays deploy first. The daemon can't detect relay versions, so this ordering is the safeguard.

What existing behavior stays:
- An agent with no task: identical, words and ids.
- The orchestrator: never a task's agent (`needs_you.rs:327-331`), so unchanged.
- Decision notices: `kind:"decision"` is retired by the daemon in favor of `kind:"task", event:"decision"`. The relay and the apps keep reading `"decision"` forever, for older runners.

What the user loses:
- Turn-end banners for agents on a task. These move to "Done", which is off by default.
- This is the intended behavior change, and it should be named in the release notes: "Agents working on a task now notify about the task".

---

### Q6. What not to notify

**Never notify, whatever the settings:**
- **Your own writes.** Any event whose actor is `Actor::User` (`models.rs:507-521`) is something you just did. The coordinator writes as `manager` (`--actor manager`, as `orchestrating-async-work` requires), so its moves still notify.
- **Backlog churn.**
  - Moves between Backlog and Todo, and into In Progress.
  - Title, intent, acceptance, label and constraint edits (`task.update`, `task_ops.rs:463-503`).
  - Moves between workspaces (`workspace_ops.rs:142`).
  - Cancelled.
  - Clearing a block.
- **Bookkeeping notes.** PROGRESS, FINDING, DECISION and COMMENT notes never notify on their own. Progress is "high volume; the one kind a reader usually wants to skip" (`models.rs:455-457`). A DECISION note only enriches a later Done body.
- **The runner's own writes.** ov-90's "Told Agent 2 about the decision" is a PROGRESS note by the new `Actor::Runner` (ov-90 `store/src/wakes.rs`, `finish_answer_wake`), so it's excluded twice over.
- **A blocker that's already Done.** No `Blocked on` for it.
- **A watched task or terminal.** The existing `attention()` rule (`watch.rs:2631-2648`) still applies to the folded agent events, and Phase 4 extends `terminal.watching` to tasks so an open task card holds its own banners.
- **The daemon's own restart replay.** The composer keys "already told" per task by `(class, status_since)` in memory, and only intake events (never a startup scan) open a window.
- **Bulk moves.** When one actor moves more than 5 tasks inside one window (a manager re-sorting the board), each task still coalesces on its own thread, but only Needs Decision and Needs Review alert. The rest go with `level: passive` and no sound.

### Q7. How ov-90 interacts with this

ov-90 (in `.claude/worktrees/ov-90`, uncommitted on top of commits `66468cb1` and `6ba4665f`) types an ANSWER into the agent that asked:
- `Watcher::answered` is called from `task_ops::note` for every ANSWER (`watch/answer_wake.rs:101-117`).
- It's queued in `answer_wakes` and told when the agent goes idle, then recorded as a PROGRESS note by `Actor::Runner`.

How the two connect:
1. **Notification answers need no new plumbing.** A decision action sends `task.note kind:answer` from the phone or Mac, the same RPC the Needs You buttons use (`Connection.swift:1498-1503`), so ov-90 wakes the agent with nothing added here.
2. **The answer closes the thread.**
   - On an ANSWER, the composer sends a quiet replacement under the same `notice_id`: body `Answered · <answer, cut>`, `level: passive`, no sound.
   - That clears the stale buttons on every device.
   - When the agent is later told (`finish_answer_wake`), nothing further is sent.
   - A push can't remove a delivered notification, only replace it, so a quiet replacement is the honest option.
3. **The same join, from both ends.**
   - ov-90's `recipient` (`watch/answer_wake.rs:205-237`) walks task to terminals: `terminals_for_task`, newest first, then the lane's terminals, then the orchestrator.
   - The notice composer walks terminal to task.
   - Both should sit in `task_link.rs` so "the agent working on X" means one thing. `terminals_for_task` is ov-90's new store function (not on main), so this lands after ov-90 merges.
4. **Ordering.** Phase 1 below touches `task_ops::note` and the `watch/` module ov-90 creates. Start Phase 1 only after ov-90 merges, to keep to one lane per worktree.

### Q8. Answer options as notification actions

**Payload:**
- The relay forwards `options: [String]` (at most 3, each at most 40 characters, from the QUESTION's `extra.options`) on `event:"decision"` for a task decision only.
- Options are a person's or the manager's words, the same class of content as the question already in the body.
- Never for an agent's permission ask. Those option names can be a command line (`push.rs:246-249`), and their answer path is the Live Activity's verified one.

**iOS:**
- Actions can't be named from the payload directly.
- The service extension (`FarCoolerNotify/NotificationService.swift`) reads `options`, registers a category `decision.<sha of options>` whose actions are the options plus "Answer…" (`UNTextInputNotificationAction`), using `setNotificationCategories` merged with the existing categories, and sets `categoryIdentifier`. Content can always carry "Open".
- Each action uses `.authenticationRequired`, not `.foreground`.
- `ForegroundPresenter.didReceive` (`Notifications.swift:197-206`) handles action identifiers. It sends through a background answer path modeled on `answerFromGlance` (`WatchLinkHost.swift:776-810`): claim once, connect, verify the question is still the latest unanswered one, send once, and post a local "Couldn't send your answer. Open Far Cooler to try again." on failure.

**Mac:**
- Same categories, registered when it posts locally.
- For APNs on the Mac, there's no service extension in the Mac app. Mac remote decisions get Open only, unless Phase 3 adds a Mac notification service extension. The Mac usually posts locally anyway (Q5).

**Android:**
- `NotificationCompat.Action`s for options, plus a `RemoteInput` "Answer…".
- This needs the app to draw the card, so `event:"decision"` goes as a **data-only** FCM message at high priority.
- `FarCoolerMessagingService` notes the cost: Firebase doesn't wake a force-stopped app for data-only messages (`FarCoolerMessagingService.kt:42-45`). That's accepted for decisions, which are also on the Needs You screen.
- Every other event class stays a `notification` message.

---

## 3. Phased build plan

Each phase is shippable on its own and ordered per Q5. "Break it and watch it go red" applies to every test listed (the repo's defining failure mode).

### Phase 0: groundwork (after ov-90 merges)

Ships: no behavior change; the shared join, and Android's decision-tap fix.

- `crates/daemon/src/task_link.rs` (new): `task_of(store, &Terminal)` with the lane fallback rule from Q1, and `terminals_for(store, &Task)` absorbing ov-90's `recipient` ordering.
- `crates/daemon/src/needs_you.rs:325-331`: use `task_of`.
- `crates/daemon/src/watch/answer_wake.rs` `recipient`: use `terminals_for`.
- `apps/android/.../ui/MainActivity.kt:83-93`: treat `terminal == ""` as absent.

Tests:
- `task_of` returns None for an orchestrator, None for a lane with two open tasks, and the task for `task_id`.
- needs_you's existing fold tests still pass.
- Android: an intent with `terminal=""`, `kind=decision` and `task=ov-1` calls `openByTaskKey` (in `BackstackTest` or a new `PushIntentTest`).

### Phase 1: the daemon composes task notices

Ships: task notices to relays that understand them; nothing alerts yet on older relays (unknown kind, `index.ts:1109`).

- `crates/daemon/src/watch/task_notice.rs` (new):
  - `TaskEvent`, the classes, the per-task window (3 s trailing, 10 s cap, immediate for decision or blocked).
  - Compose-at-close against current state, the noise rules from Q6, and `notice_id`.
  - `tap` support for tests, as `Tapped` already has (`watch.rs:489-499`).
- `crates/daemon/src/task_ops.rs` `create`, `set_status`, `note`, `block`, and `workspace_ops.rs` move: call `watcher.task_event`. Remove the `announce_decision` call at `task_ops.rs:522-525`, and keep `announce_decision`'s body as the decision composer.
- `crates/daemon/src/watch.rs:4963-5000`: when `task_of` is Some, send the agent notice with `alert:false` and feed `agent_event`.
- `crates/daemon/src/push.rs`: `Notification` and `Outgoing` gain `notice_id`, `event`, `level`, `options`, `alert`. `wire_body` adds a `Some("task")` arm carrying title, subtitle, task, runner, workspace, count, notice_id, event, level, options, and no terminal.
- `proto/farcooler.proto`: `Event.payload` gains `Notice notice = 25` (the next free tag after `needs_you_changed = 24`), with the same fields, and `Host` gains `bool push_paired = 13`.
- `crates/cli/src/main.rs:4761` (events printer): a `"notice"` kind line.

Tests (in `watch.rs` tests and `task_notice.rs`):
- In Progress, then In Review, then Done inside 3 s taps one notice, `Done`.
- Needs Decision then an answer inside the window taps nothing.
- Needs Decision alone taps at once.
- A user-actor move taps nothing; a manager move taps.
- PROGRESS and Runner notes tap nothing.
- Blocked-on a Done blocker taps nothing.
- An agent Blocked on a task taps `kind:"task"` with `event:"decision"` and an agent notice with `alert:false`; with no task it taps the old agent notice unchanged.
- `notice_id` stays at most 64 and is stable across calls.
- `wire_body` for `"task"` never carries a terminal, and never carries options for an agent ask.
- Restart replay taps nothing.

### Phase 2: the relay delivers task notices

Ships: task notifications on phones and Mac via push, with replacement.

- `services/relay/src/index.ts`:
  - `alerts` includes `"task"`.
  - Validate `notice_id`, `event`, `level` and `options` (`^[A-Za-z0-9:._-]{1,64}$`; event in the 5; level in 3; at most 3 options of at most 40 characters).
  - `alert === false` skips the alert loop but not `pushActivity`.
  - Filter devices by `notify_events` (NULL means defaults) for `kind:"task"`; keep `notify_on_done` for agent `done`.
- `services/relay/src/push.ts`:
  - `Payload` gains the fields.
  - `sendApns` sets `apns-collapse-id`, `thread-id = notice_id` (else terminal), and `interruption-level` from `level`. It carries `task`, `runner`, `event`, `options` and `notice_id` in the body.
  - `sendFcm` sets `android.notification.tag`, picks the channel by `event`, and sends data-only for `event:"decision"`.
  - Omit `data.terminal` when empty.
- `services/relay/migrations/0017_notify_events.sql`, and the upsert column (`index.ts:340-356`).

Tests (`services/relay/test/relay.test.ts`):
- A task notice to two devices, one with `review` off, sends one.
- The APNs request has `apns-collapse-id` and `thread-id` equal to `notice_id`.
- `level: passive` is reflected.
- An over-long or odd `notice_id` sends with no collapse id rather than 400.
- `alert:false` sends no alert and still moves the card.
- An old registration (NULL) gets decision, review and blocked, but not done or new.
- The upsert regression test covers `notify_events`.

### Phase 3: clients render, act and set

Ships: Open and the answer actions, the settings, local posting on the Mac, and Android channels.

- **AgentKit:**
  - `ShellNavigation.swift:1986-1997`: `PushTap` accepts `kind:"task"`, and never falls back to a `t:` thread as a terminal.
  - `PushRegistration.swift`: `notifyEvents: () -> [String]`.
  - New `TaskNotifications.swift` with the five keys and their defaults, shared by Mac and iOS.
- **Mac:**
  - `Notifications.swift`: `post(notice:)` from the `"notice"` event (`EventStream.swift:296-322` gains the case). It posts locally only when the runner isn't `push_paired` or `PushRegistration.registered` is false; otherwise APNs delivers, and the shared id would replace it anyway.
  - Categories with options and "Answer…".
  - `didReceive` answers through the runner's `DaemonClient` with `task.note`.
  - `presentation` reads the task and terminal from `userInfo`.
  - `Preferences.swift:455-476`: the "Tasks" section.
- **iOS:**
  - `FarCoolerNotify/NotificationService.swift`: dynamic decision category; skip the widget fold when there's no `terminal` (unchanged).
  - `Notifications.swift`: action handling and the background answer path.
  - `Settings.swift`: the "Tasks" section; registration sends `notifyEvents`.
- **Android:**
  - `Notifier.kt`: the five `tasks.*` channels; local and data-only render with `notify(tag, 0)`, actions and `RemoteInput`.
  - A new `AnswerReceiver` sends `task.note` through the runner's connection.
  - `MainActivity.kt`: `kind:"task"`.
  - `Settings.kt` and `Screens.kt:336+`: the section; `PushRegistration.kt`: `notifyEvents`.

Tests:
- AgentKit `PushTap` for `task`, `decision` and a `t:` thread with no terminal.
- AgentKit `TaskNotifications` defaults.
- iOS UI test (through `scripts/ios-ui-tests.sh`): a delivered decision shows option actions. This is a UI-only check: Simulator push can be used with a `.apns` file carrying `options`.
- Mac `CeremonyTests`: `presentation` suppresses for an on-screen task; local post is skipped when paired and registered.
- Android: `NotificationCopyTest` reads the daemon's task wording (as it reads `notification()` today); channel-by-event; `AnswerReceiver` sends exactly one `task.note`.
- Settings round-trip: off sends `notifyEvents` without it.

### Phase 4: noise and polish

Ships: the quieter edges.

- **The answered replacement** (Q7.2) in `task_notice.rs`. Test: an ANSWER taps a passive `Answered · …` with the same id.
- **Watching a task.** `terminal.watching` becomes `watching` with task ids (proto plus `watch.rs:3052-3070`); the composer holds banners for a watched task as `attention()` does. Test: a watched task's In Review taps nothing at alert level.
- **The bulk-move rule** (Q6). Test: 6 manager moves in one window give passive notices for all but review and decision.
- **Agent notices with no task** get `notice_id = a:<terminal>` and the matching local identifier (Mac `Notifications.swift:160-164`, iOS `:161-165`, Android `Notifier.kt:181`). This ends today's Mac duplicate (1.3). Test: the relay sets the collapse id; the Mac local id equals it.

## 4. Open questions for the owner

1. **Agent turn ends on a task** are filed under Done, which is off by default. If you'd rather hear "claude finished a turn on ov-90" separately from "ov-90 Done", that's a sixth toggle. The recommendation is not to add it.
2. **The orchestrator's own turn-end notifications** are unchanged ("agents with no task notify as today"), and it ends a turn constantly. Turning orchestrator turn-ends off by default is a one-line follow-up if you want it.
3. **Per-workspace mute** is deferred. If wanted, it lives on the workspace beside ov-90's `wake_on_answer`, decided by the daemon.
