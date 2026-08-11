# Message Editing Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make a message written mid-turn appear as an editable queue row instead of vanishing, and let an already-sent message be re-opened in the composer.

**Architecture:** The client stops predicting whether the daemon will queue a message. It always draws what you typed, and removes that row when a `PromptQueue` event turns up carrying the same text — the daemon stays authoritative about which bucket the message landed in. Reconciliation lives in the shared `Transcript` reducer so a Mac and a phone cannot disagree; each client only loses its `whileWorking` argument. Editing a sent message is composer prefill and touches no protocol.

**Tech Stack:** Rust (`crates/agent`), Swift 6 / SwiftUI (`apps/shared/AgentKit`, `apps/macos`, `apps/ios`), Kotlin / Compose (`apps/android`).

## Global Constraints

- US English throughout, in code and in copy. Never "authorise", "colour", "centre".
- **Never run `cargo fmt`.** The Rust tree is hand-formatted and CI skips `fmt --check` deliberately. Match surrounding formatting by hand.
- `cargo` is not on `PATH`. Use `/Users/e-liang/.cargo/bin/cargo`.
- Comments explain WHY and name the concrete failure that motivated them. Never restate the code.
- Tests are named as full sentences stating the invariant: `a_queued_message_leaves_the_transcript_it_was_drawn_in`.
- Apple copy conventions: title-case buttons, contractions, no raw errors in UI.
- Work in the existing worktree `/Users/e-liang/Dev/overnight/.claude/worktrees/native-sdk-parity` on branch `feat/native-sdk-parity`.
- Spec: `docs/superpowers/specs/2026-08-11-message-editing-design.md`.

---

## File Structure

| File | Responsibility | Change |
|---|---|---|
| `crates/agent/src/activity_source.rs` | Maps an `AgentEvent` to the activity it implies | Modify — `Role::User` implies nothing |
| `apps/shared/AgentKit/Sources/AgentKit/Transcript.swift` | The shared reducer: events in, rows out | Modify — track unconfirmed echoes, reconcile on `promptQueue` |
| `apps/shared/AgentKit/Tests/AgentKitTests/TranscriptTests.swift` | Reducer tests | Modify — three new cases |
| `apps/macos/Sources/FarCooler/AgentStream.swift` | Mac transport + local echo | Modify — always echo |
| `apps/macos/Sources/FarCooler/AgentComposer.swift` | Mac composer | Modify — drop the prediction, accept a prefill |
| `apps/macos/Sources/FarCooler/AgentRows.swift` | Row views | Modify — Edit action on a user message |
| `apps/macos/Sources/FarCooler/AgentSurface.swift` | Wires rows to the composer | Modify — carry the prefill |
| `apps/ios/FarCooler/AgentStream.swift` | iOS transport + local echo | Modify — always echo |
| `apps/ios/FarCooler/AgentView.swift` | iOS composer call site | Modify — drop the prediction |
| `apps/android/app/src/main/java/com/farcooler/model/Transcript.kt` | Android reducer | Modify — mirror reconciliation |
| `apps/android/app/src/main/java/com/farcooler/net/AgentStream.kt` | Android transport | Modify — always echo |
| `apps/android/app/src/main/java/com/farcooler/ui/AgentScreen.kt` | Android composer call site | Modify — drop the prediction |
| `apps/android/app/src/test/java/com/farcooler/model/TranscriptTest.kt` | Android reducer tests | Modify — mirror one case |

**Out of scope, deliberately:** the Edit affordance is Mac-only in this plan. iOS and Android compose through different plumbing and neither was the surface the request came from. The reconciliation fix lands on all three, because leaving a phone dropping messages while the Mac keeps them is the disagreement the shared reducer exists to prevent.

---

### Task 1: The user's own words are not the agent working

`observe` maps every `Message` to `Working`, including `Role::User`. A pane is marked busy because a person typed. It is corrected first because it is self-contained and nothing else depends on it.

Consequence, considered and accepted: `send_next_queued` and `steer_queued` emit `Message { role: User }`, and those were marking the pane `Working` the instant a queued prompt went out. After this, the pane stays `Done` until the agent's own first token, a fraction of a second later. An ordinary send never emitted that event at all — the echo is client-side — so it was already the behavior on the common path, and reporting the agent as working before it has done anything is the thing being fixed.

**Files:**
- Modify: `crates/agent/src/activity_source.rs`
- Test: `crates/agent/src/activity_source.rs` (inline `mod tests`)

**Interfaces:**
- Consumes: nothing from earlier tasks.
- Produces: nothing later tasks rely on. `pub fn observe(event: &AgentEvent) -> Option<AgentActivity>` keeps its signature.

- [ ] **Step 1: Write the failing test**

Add to `mod tests` in `crates/agent/src/activity_source.rs`:

```rust
    #[test]
    fn your_own_words_are_not_the_agent_working() {
        // A pane went busy because a PERSON typed. `send_next_queued` and
        // `steer_queued` both emit the user's message, and each one moved the
        // badge before the agent had done anything at all.
        assert_eq!(
            observe(&AgentEvent::Message { role: Role::User, text: "hi".into(), parent: None }),
            None
        );
        // The agent's own words still are.
        assert_eq!(
            observe(&AgentEvent::Message { role: Role::Agent, text: "hi".into(), parent: None }),
            Some(AgentActivity::Working)
        );
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `/Users/e-liang/.cargo/bin/cargo test -p farcooler-agent your_own_words`
Expected: FAIL — `assertion `left == right` failed`, left `Some(Working)`, right `None`.

- [ ] **Step 3: Write minimal implementation**

Replace the `Message` arm in `observe`. The existing arm is:

```rust
        AgentEvent::Message { .. }
        | AgentEvent::ToolCall { .. }
```

Change it to match on the role, keeping every other arm exactly as it is:

```rust
        // The AGENT's words. What the user typed says nothing about what the
        // agent is doing — a pane that went busy because a person typed was
        // reporting the wrong actor, and `send_next_queued` emits exactly that
        // event every time a queued prompt goes out.
        AgentEvent::Message { role: Role::Agent | Role::Thought, .. }
        | AgentEvent::ToolCall { .. }
```

and add a `Role::User` arm beside the bookkeeping group:

```rust
        AgentEvent::Message { role: Role::User, .. } => None,
```

`Role` is already imported in the test module only — add `use farcooler_agent_core::event::Role;` beside the existing `AgentActivity` import at the top of the file if it is not already in scope.

- [ ] **Step 4: Run tests to verify they pass**

Run: `/Users/e-liang/.cargo/bin/cargo test -p farcooler-agent`
Expected: PASS, including the pre-existing `work_in_flight_is_working` and `bookkeeping_events_do_not_move_the_badge`.

- [ ] **Step 5: Commit**

```bash
git add crates/agent/src/activity_source.rs
git commit -m "fix: a pane is not working because you typed"
```

---

### Task 2: The shared reducer reconciles an echo against the queue

**Files:**
- Modify: `apps/shared/AgentKit/Sources/AgentKit/Transcript.swift`
- Test: `apps/shared/AgentKit/Tests/AgentKitTests/TranscriptTests.swift`

**Interfaces:**
- Consumes: nothing from Task 1.
- Produces: `Transcript.appendLocalUserMessage(_ text: String)` keeps its signature and gains reconciliation behavior. Tasks 3 and 4 rely on it being safe to call unconditionally.

- [ ] **Step 1: Write the failing tests**

Add to `apps/shared/AgentKit/Tests/AgentKitTests/TranscriptTests.swift`:

```swift
@Test func aQueuedMessageLeavesTheTranscriptItWasDrawnIn() {
    // Drawn immediately so your words never vanish, then withdrawn once the
    // daemon says it was held: it belongs in the queue, where it can still be
    // edited, not in the conversation it has not joined yet.
    var t = Transcript()
    t.appendLocalUserMessage("use unittest")
    #expect(t.rows.contains { $0.kind == .message(role: .user, text: "use unittest", parent: nil) })

    t.apply(Sequenced(seq: 0, event: .promptQueue(items: [
        QueuedPrompt(id: "0", text: "use unittest")
    ])))

    #expect(!t.rows.contains { $0.kind == .message(role: .user, text: "use unittest", parent: nil) })
    #expect(t.queue.count == 1)
}

@Test func aMessageSentStraightAwayKeepsItsRow() {
    // Nothing queued it, so nothing withdraws it. This is the ordinary path
    // and the one that must not regress.
    var t = Transcript()
    t.appendLocalUserMessage("add tests")
    t.apply(Sequenced(seq: 0, event: .promptQueue(items: [])))
    #expect(t.rows.contains { $0.kind == .message(role: .user, text: "add tests", parent: nil) })
}

@Test func aQueueEntryThatMatchesNothingWithdrawsNothing() {
    // The failure mode is a duplicate row, never a message the user wrote and
    // never saw again.
    var t = Transcript()
    t.appendLocalUserMessage("add tests")
    t.apply(Sequenced(seq: 0, event: .promptQueue(items: [
        QueuedPrompt(id: "0", text: "something else")
    ])))
    #expect(t.rows.contains { $0.kind == .message(role: .user, text: "add tests", parent: nil) })
}
```

If the existing test file uses XCTest rather than swift-testing, mirror whichever style the neighboring tests in that file already use — read it first and follow it exactly.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd apps/shared/AgentKit && swift test --filter Transcript`
Expected: FAIL on `aQueuedMessageLeavesTheTranscriptItWasDrawnIn` — the row is still present, because nothing removes it yet.

- [ ] **Step 3: Write minimal implementation**

In `Transcript`, add the state beside the other private stored properties:

```swift
    /// Messages drawn straight from the composer that the daemon has not yet
    /// accounted for.
    ///
    /// The composer used to PREDICT whether a message would be queued, by
    /// reading fleet state while the thing it predicted read the agent
    /// channel. In the window between a turn ending on one and the other
    /// noticing, it guessed wrong: the echo was suppressed for a queue row
    /// that never came, the CLI's own echo was dropped as a duplicate, and the
    /// message reached the model without ever being drawn.
    ///
    /// So the client draws first and asks after. An entry lives only until the
    /// next `promptQueue`, which is the event that answers the question — a
    /// queue that carries the text means it was held, and one that does not
    /// means it went out.
    private var unconfirmedEchoes: [String] = []
```

In `appendLocalUserMessage`, record the text — the rest of the method is unchanged:

```swift
    public mutating func appendLocalUserMessage(_ text: String) {
        append(.message(role: .user, text: text, parent: nil))
        breakBeforeNextMessage = true
        unconfirmedEchoes.append(text)
    }
```

Replace the `.promptQueue` arm in `apply`:

```swift
        case let .promptQueue(items):
            // Wholesale, like the plan: the daemon sends the whole queue on
            // every change, and a client reconstructing it from adds and
            // removes could disagree with what will actually be sent.
            queue = items
            // Anything drawn from the composer that turns out to be HELD is
            // withdrawn from the conversation — it has not joined it yet, and
            // in the queue it can still be edited or taken back. Anything not
            // named here went out, so it stays and stops being pending: this
            // event is the daemon's answer either way, which is what bounds
            // the list to the few milliseconds between typing and the reply.
            for text in unconfirmedEchoes where items.contains(where: { $0.text == text }) {
                removeLastLocalUserMessage(text)
            }
            unconfirmedEchoes.removeAll()
```

Add the helper beside `appendLocalUserMessage`:

```swift
    /// Withdraw the last user message drawn locally with this exact text.
    ///
    /// The LAST, because a repeated instruction is a thing people really send
    /// twice, and withdrawing the older one would leave the transcript showing
    /// the wrong instance as still waiting.
    private mutating func removeLastLocalUserMessage(_ text: String) {
        guard let index = rows.lastIndex(where: {
            $0.kind == .message(role: .user, text: text, parent: nil)
        }) else { return }
        rows.remove(at: index)
    }
```

`rows` is `public private(set)`, so mutating it from inside `Transcript` is allowed with no visibility change.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd apps/shared/AgentKit && swift test --filter Transcript`
Expected: PASS, all three new cases and every pre-existing case in that file.

- [ ] **Step 5: Commit**

```bash
git add apps/shared/AgentKit/Sources/AgentKit/Transcript.swift apps/shared/AgentKit/Tests/AgentKitTests/TranscriptTests.swift
git commit -m "fix: a held message is withdrawn from the chat, not hidden from it"
```

---

### Task 3: The Mac and iOS clients stop predicting

**Files:**
- Modify: `apps/macos/Sources/FarCooler/AgentStream.swift`
- Modify: `apps/macos/Sources/FarCooler/AgentComposer.swift:662`
- Modify: `apps/ios/FarCooler/AgentStream.swift`
- Modify: `apps/ios/FarCooler/AgentView.swift:170`

**Interfaces:**
- Consumes: `Transcript.appendLocalUserMessage` from Task 2, now safe to call unconditionally.
- Produces: `AgentStream.send(_ text: String, images:) async` on both platforms — the `whileWorking` parameter is gone. Task 5 calls neither; it only prefills.

- [ ] **Step 1: Change the Mac transport**

In `apps/macos/Sources/FarCooler/AgentStream.swift`, change the signature and the echo. Replace the `whileWorking` parameter and the `if !whileWorking` block with an unconditional call:

```swift
    func send(_ text: String, images: [ComposerImage] = []) async {
        guard !refuseIfNeeded() else { return }
        // Always drawn, never predicted.
        //
        // This used to echo only when the composer believed no turn was
        // running — a guess made from fleet state about a decision taken on
        // the agent channel. Guessing wrong meant the message reached the
        // model and was drawn by nobody. The daemon answers with a
        // `PromptQueue` if it held the message, and `Transcript` withdraws the
        // row then; until it does, what you typed is on screen.
        transcript.appendLocalUserMessage(text)
```

Leave the rest of the method — the temp-file image handling and the `agent-prompt` invocation — exactly as it is.

- [ ] **Step 2: Change the Mac call site**

In `apps/macos/Sources/FarCooler/AgentComposer.swift`, delete the `let working = terminal.agent == .working` line and its now-wrong comment, and call:

```swift
        Task { await stream.send(body, images: images) }
```

If `terminal` becomes unused in that scope, leave the property alone — it is still read at line 578 for the blocked case.

- [ ] **Step 3: Change the iOS transport and call site**

In `apps/ios/FarCooler/AgentStream.swift`, apply the same change:

```swift
    func send(_ text: String, images: [(mime: String, data: Data)] = []) async {
        // Always drawn, never predicted — see the Mac's `AgentStream.send`.
        transcript.appendLocalUserMessage(text)
```

In `apps/ios/FarCooler/AgentView.swift:170`, drop the argument:

```swift
                        Task { await stream.send(text, images: images) }
```

- [ ] **Step 4: Build both**

Run: `cd apps/macos && PATH="$HOME/.cargo/bin:$PATH" swift build -c debug 2>&1 | tail -20`
Expected: builds with no error mentioning `whileWorking`.

There is no iOS simulator build in this loop; confirm iOS by inspection that no call site still passes `whileWorking` — run `grep -rn "whileWorking" apps/` and expect only Android matches, which Task 4 clears.

- [ ] **Step 5: Commit**

```bash
git add apps/macos/Sources/FarCooler/AgentStream.swift apps/macos/Sources/FarCooler/AgentComposer.swift apps/ios/FarCooler/AgentStream.swift apps/ios/FarCooler/AgentView.swift
git commit -m "fix: the composer draws what you typed instead of predicting its fate"
```

---

### Task 4: Android mirrors the same reducer change

**Files:**
- Modify: `apps/android/app/src/main/java/com/farcooler/model/Transcript.kt:280`
- Modify: `apps/android/app/src/main/java/com/farcooler/net/AgentStream.kt:149`
- Modify: `apps/android/app/src/main/java/com/farcooler/ui/AgentScreen.kt:243`
- Test: `apps/android/app/src/test/java/com/farcooler/model/TranscriptTest.kt`

**Interfaces:**
- Consumes: the behavior defined in Task 2, reimplemented in Kotlin. Read `Transcript.swift` first and mirror its structure and its comments; the two reducers are deliberately the same shape.
- Produces: `AgentStream.send(text, images)` with no `whileWorking` parameter.

- [ ] **Step 1: Write the failing test**

Add to `apps/android/app/src/test/java/com/farcooler/model/TranscriptTest.kt`, matching the assertion style already used in that file:

```kotlin
    @Test
    fun `a queued message leaves the transcript it was drawn in`() {
        val t = Transcript()
        t.appendLocalUserMessage("use unittest")
        assertTrue(t.rows.any { it.kind is Kind.Message && (it.kind as Kind.Message).text == "use unittest" })

        t.apply(AgentEvent.PromptQueue(listOf(QueuedPrompt("0", "use unittest", null))))

        assertFalse(t.rows.any { it.kind is Kind.Message && (it.kind as Kind.Message).text == "use unittest" })
        assertEquals(1, t.queue.size)
    }
```

Adjust the constructor and accessor names to whatever `Transcript.kt` and `AgentEvent.kt` actually declare — read them first rather than trusting these names.

- [ ] **Step 2: Run test to verify it fails**

Run: `cd apps/android && JAVA_HOME=$(/usr/libexec/java_home -v 17) ./gradlew testDebugUnitTest --tests '*TranscriptTest*'`
Expected: FAIL — the row is still present.

Note: this module needs JDK 17; without it Gradle fails before running anything.

- [ ] **Step 3: Write minimal implementation**

Add the state beside the other private fields in `Transcript.kt`:

```kotlin
    /**
     * Messages drawn straight from the composer that the daemon has not yet
     * accounted for.
     *
     * The composer used to PREDICT whether a message would be queued, reading
     * fleet state about a decision taken on the agent channel. In the window
     * between a turn ending on one and the other noticing it guessed wrong:
     * the echo was suppressed for a queue row that never came, the agent's own
     * echo was dropped as a duplicate, and the message reached the model
     * without ever being drawn.
     *
     * So the client draws first and asks after. An entry lives only until the
     * next PromptQueue, which is the event that answers the question.
     */
    private val unconfirmedEchoes = mutableListOf<String>()
```

Record the text in `appendLocalUserMessage`, before `changed()`:

```kotlin
        breakBeforeNextMessage = true
        unconfirmedEchoes.add(text)
        changed()
```

Replace the `PromptQueue` branch at line 570:

```kotlin
            is AgentEvent.PromptQueue -> {
                queue = event.items
                // Anything drawn from the composer that turns out to be HELD is
                // withdrawn from the conversation — it has not joined it yet,
                // and in the queue it can still be edited or taken back.
                // Anything not named here went out, so it stays and stops being
                // pending: this event is the daemon's answer either way.
                for (text in unconfirmedEchoes) {
                    if (event.items.any { it.text == text }) removeLastLocalUserMessage(text)
                }
                unconfirmedEchoes.clear()
            }
```

Add the helper beside `appendLocalUserMessage`:

```kotlin
    /**
     * Withdraw the last user message drawn locally with this exact text.
     *
     * The LAST, because a repeated instruction is a thing people really send
     * twice, and withdrawing the older one would leave the transcript showing
     * the wrong instance as still waiting.
     */
    private fun removeLastLocalUserMessage(text: String) {
        val index = rows.indexOfLast {
            val kind = it.kind
            kind is TranscriptRow.Kind.Message && kind.role == Role.USER && kind.text == text
        }
        if (index >= 0) rows.removeAt(index)
    }
```

If `rows` is exposed as an immutable `List`, use whatever private mutable backing field `append` already writes to — read `append` first and follow it.

- [ ] **Step 4: Run the test and drop the prediction**

Run: `cd apps/android && JAVA_HOME=$(/usr/libexec/java_home -v 17) ./gradlew testDebugUnitTest --tests '*TranscriptTest*'`
Expected: PASS.

Then remove the parameter: in `AgentStream.kt:149` echo unconditionally, and in `AgentScreen.kt:243` call `stream.send(text, images)`.

Run: `cd apps/android && JAVA_HOME=$(/usr/libexec/java_home -v 17) ./gradlew assembleDebug`
Expected: builds clean.

- [ ] **Step 5: Commit**

```bash
git add apps/android
git commit -m "fix: a phone withdraws a held message the same way a Mac does"
```

---

### Task 5: Edit re-opens a sent message in the composer

Prefill and focus. The original message stays; nothing rewinds and nothing re-runs.

**Files:**
- Modify: `apps/macos/Sources/FarCooler/AgentRows.swift:26` and `:134` (`MessageRow`)
- Modify: `apps/macos/Sources/FarCooler/AgentSurface.swift`
- Modify: `apps/macos/Sources/FarCooler/AgentComposer.swift`

**Interfaces:**
- Consumes: nothing from Tasks 1–4.
- Produces: `MessageRow(role:text:isLive:onEdit:)` where `onEdit: (() -> Void)?` — `nil` for any row that is not the user's.

- [ ] **Step 1: Give the composer an external prefill**

`AgentComposer` holds `@State private var text` at line 31. Add a binding it can be driven through, next to its other stored properties:

```swift
    /// Text handed in from outside — the Edit action on a sent message.
    ///
    /// A binding rather than a method, because the composer owns its text and
    /// a second writer would have to reach into `@State` to set it.
    @Binding var prefill: String?
```

and apply it where the view body can react, on the outermost container of the composer body:

```swift
        .onChange(of: prefill) { _, incoming in
            guard let incoming, !incoming.isEmpty else { return }
            text = incoming
            cursor = incoming.count
            prefill = nil
        }
```

**Do not try to set focus here.** `isFocused` on `AgentComposer` is `let isFocused: Bool` (line 16) — it is the PANE's focus, threaded down from the owner through `AgentSurface` (line 33), and the text view's coordinator reacts to it at line 772. The composer cannot focus itself, and adding a second focus source would fight the coordinator that already owns it. It does not need to: Edit is only reachable by clicking inside a pane that is focused by definition, so the field is already taking keystrokes. Say that in the comment so nobody adds it back.

- [ ] **Step 2: Add the Edit affordance to a user message**

In `MessageRow`, accept `let onEdit: (() -> Void)?` and show it only when there is one, following the hover pattern `QueuedRow` at line 642 already uses for its own controls — read that row first and match it, so the two editable surfaces look like siblings rather than two designs.

```swift
            if let onEdit {
                Button("Edit", action: onEdit)
                    .buttonStyle(.plain)
                    .help("Put this message back in the composer")
            }
```

"Edit" is title case per the house copy rules.

- [ ] **Step 3: Wire it up**

At `AgentRows.swift:26`, pass `onEdit` only for the user's own words:

```swift
        case let .message(role, text, _):
            MessageRow(
                role: role, text: text, isLive: isLast,
                onEdit: role == .user ? { onEditMessage?(text) } : nil)
```

Thread `onEditMessage: ((String) -> Void)?` down from `AgentSurface` the same way `onAnswer` is already threaded to `AgentRowView`, and in `AgentSurface` set the composer's `prefill` binding from it.

- [ ] **Step 4: Build and check by hand**

Run: `cd apps/macos && PATH="$HOME/.cargo/bin:$PATH" ./build-app.sh release`
Expected: builds and assembles the bundle.

Then confirm by hand in the built app: Edit on one of your own messages fills the composer with that text and focuses it, the original message is still in the transcript, and nothing was sent.

- [ ] **Step 5: Commit**

```bash
git add apps/macos/Sources/FarCooler/AgentRows.swift apps/macos/Sources/FarCooler/AgentSurface.swift apps/macos/Sources/FarCooler/AgentComposer.swift
git commit -m "feat: Edit puts a sent message back in the composer"
```

---

## Final verification

- [ ] `/Users/e-liang/.cargo/bin/cargo test --workspace` — exit 0.
- [ ] `cd apps/shared/AgentKit && swift test` — exit 0.
- [ ] `grep -rn "whileWorking" apps/` returns nothing.
- [ ] Rebuild and reinstall: `cd apps/macos && PATH="$HOME/.cargo/bin:$PATH" ./build-app.sh release`, then replace `/Applications/Far Cooler.app` and relaunch. Existing chat panes keep their old `agent-host` process, so toggle a pane out of chat and back before testing, or the old binary will still be serving it.
- [ ] By hand, in a pane with a turn running: type a message, confirm it appears immediately, then confirm it moves into the queue rows under the composer and can be edited, cancelled and steered from there.
