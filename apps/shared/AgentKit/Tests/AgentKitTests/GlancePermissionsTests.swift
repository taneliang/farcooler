import Foundation
import Testing

@testable import AgentKit

/// The rules a lock screen card follows before it offers to answer an agent.
///
/// All of it is arithmetic and bookkeeping on purpose. The parts that cannot be
/// tested here — whether forty characters really is one line on a card, whether
/// three lines of button really fit — are the parts a device has to settle, and
/// they are isolated behind two numbers the widget passes in for exactly that
/// reason.
struct GlancePermissionsTests {
    private func option(_ id: String, _ name: String, _ kind: String) -> GlancePermissionOption {
        GlancePermissionOption(id: id, name: name, kind: kind)
    }

    private func permission(
        _ options: [GlancePermissionOption], request: String = "r1", terminal: String = "t1"
    ) -> GlancePermission {
        GlancePermission(
            terminal: terminal, request: request, options: options,
            observedAt: Date(timeIntervalSince1970: 1000))
    }

    // MARK: - Which answers fit

    /// The good case needs no rule: three short answers in the agent's order is
    /// exactly what the agent asked.
    @Test func everyAnswerIsShownWhenEveryAnswerFits() {
        let all = [
            option("a", "Yes", "allow_once"),
            option("b", "Always", "allow_always"),
            option("c", "No", "reject_once"),
        ]
        let fit = permission(all).fit(lines: 3, columns: 40)
        #expect(fit.shown.map(\.id) == ["a", "b", "c"])
        #expect(fit.hidden == 0)
    }

    /// Claude's real vocabulary: a short yes, a long "always", a long no. It
    /// does not all fit, so the pair survives and the middle is counted.
    ///
    /// The two that survive are still in the order the agent listed them —
    /// a subsequence preserves order, which is why the pair is built from
    /// indices rather than assembled yes-first.
    @Test func aLongListFallsBackToThePairAndSaysHowManyItDropped() {
        let all = [
            option("a", "Yes", "allow_once"),
            option("b", "Yes, and don’t ask again for cargo commands", "allow_always"),
            option("c", "No, and tell Claude what to do differently", "reject_once"),
        ]
        let fit = permission(all).fit(lines: 3, columns: 40)
        #expect(fit.shown.map(\.id) == ["a", "c"])
        #expect(fit.hidden == 1)
    }

    /// The rule that matters most, and the one `PermissionView` names: a
    /// surface whose yes is visible and whose no is not is arguing for yes.
    /// Given room for only the yes, this shows neither.
    @Test func aYesIsNeverShownWithoutItsNo() {
        let all = [
            option("a", "Yes", "allow_once"),
            option("b", "No, and tell Claude what to do differently", "reject_once"),
        ]
        // One line of room: the yes alone would fit and the pair cannot.
        let fit = permission(all).fit(lines: 1, columns: 40)
        #expect(fit.shown.isEmpty)
        #expect(fit.hidden == 2)
    }

    /// A name is never shortened to make it fit. `Allow Bash(cargo test -p
    /// farcooler-core)` is a real fixture in this repo, and at a narrower
    /// column count it costs two lines rather than an ellipsis.
    @Test func aLongNameCostsLinesRatherThanCharacters() {
        let all = [
            option("a", "Allow Bash(cargo test -p farcooler-core)", "allow_once"),
            option("b", "Deny", "reject_once"),
        ]
        #expect(permission(all).fit(lines: 3, columns: 20).shown.map(\.id) == ["a", "b"])
        // Two lines for the long one plus one for the short one is three; at a
        // budget of two there is no honest way to show both.
        #expect(permission(all).fit(lines: 2, columns: 20).shown.isEmpty)
    }

    /// An agent whose vocabulary this build has never met. `PermissionView`
    /// draws every button alike for the same input; a card with three lines
    /// does not have that option, so it draws none and points at the app.
    @Test func anUnrecognizedVocabularyGetsNoButtons() {
        let all = [
            option("a", "Proceed", "continue"),
            option("b", "Halt", "stop"),
            option("c", "Ask me later", "defer"),
        ]
        let fit = permission(all).fit(lines: 1, columns: 40)
        #expect(fit.shown.isEmpty)
        #expect(fit.hidden == 3)
    }

    @Test func noOptionsAtAllIsNotOverflow() {
        let fit = permission([]).fit(lines: 3, columns: 40)
        #expect(fit.shown.isEmpty)
        #expect(fit.hidden == 0)
    }

    /// The same derivation the phone's `ApprovalControls` and the watch's
    /// `PermissionView.plainYes` run, so all three agree about which answer is
    /// filled in.
    @Test func thePlainYesIsTheAllowOnce() {
        let all = [
            option("a", "Always allow", "allow_always"),
            option("b", "Allow", "allow_once"),
            option("c", "Deny", "reject_once"),
        ]
        #expect(permission(all).plainYes?.id == "b")
        #expect(permission([option("z", "Go on", "proceed")]).plainYes == nil)
    }

    // MARK: - Not answering the same permission twice

    private var claimable: GlancePermissions {
        GlancePermissions().recording(
            permission([option("a", "Yes", "allow_once"), option("b", "No", "reject_once")]),
            for: "t1")
    }

    /// The claim is the whole of the double-tap guard. Nothing downstream can
    /// catch a duplicate: `terminal.agent_answer` posts a message and returns,
    /// and no event ever retires a permission.
    @Test func aSecondTapOnAClaimedPermissionIsRefused() {
        let now = Date(timeIntervalSince1970: 2000)
        let claimed = claimable.claiming(
            terminal: "t1", request: "r1", option: "a", optionName: "Yes", at: now)
        #expect(claimed != nil)
        #expect(
            claimed?.claiming(
                terminal: "t1", request: "r1", option: "b", optionName: "No", at: now) == nil)
    }

    /// …but a claim that established nothing was sent hands the buttons back.
    /// That distinction is the reason `nothingSent` exists as a case at all.
    @Test func aClaimThatSentNothingCanBeTriedAgain() {
        let now = Date(timeIntervalSince1970: 2000)
        let settled = claimable
            .claiming(terminal: "t1", request: "r1", option: "a", optionName: "Yes", at: now)?
            .settling(
                terminal: "t1", request: "r1", outcome: .nothingSent,
                message: "Nothing was sent.", at: now)
        #expect(settled?.answer(for: "t1")?.refusesAnotherTap == false)
        #expect(
            settled?.claiming(
                terminal: "t1", request: "r1", option: "b", optionName: "No", at: now) != nil)
    }

    /// An answer we could not confirm goes on refusing. The failure this
    /// prevents is concrete: a reject that landed, reported as unsent, followed
    /// by a tap on the option that allows.
    @Test func anUnconfirmedAnswerGoesOnRefusing() {
        let now = Date(timeIntervalSince1970: 2000)
        let settled = claimable
            .claiming(terminal: "t1", request: "r1", option: "a", optionName: "Yes", at: now)?
            .settling(
                terminal: "t1", request: "r1", outcome: .unsure, message: "May have gone through.",
                at: now)
        #expect(settled?.answer(for: "t1")?.refusesAnotherTap == true)
        #expect(
            settled?.claiming(
                terminal: "t1", request: "r1", option: "b", optionName: "No", at: now) == nil)
    }

    /// The sentence has to survive the settling whatever the outcome — it is
    /// the only thing the card has to say about what happened.
    @Test func theOutcomeCarriesItsSentence() {
        let now = Date(timeIntervalSince1970: 2000)
        let settled = claimable
            .claiming(terminal: "t1", request: "r1", option: "a", optionName: "Yes", at: now)?
            .settling(terminal: "t1", request: "r1", outcome: .sent, message: "Sent “Yes”.", at: now)
        #expect(settled?.answer(for: "t1")?.message == "Sent “Yes”.")
        #expect(settled?.answer(for: "t1")?.optionName == "Yes")
    }

    /// Settling something nobody claimed changes nothing. A stray settle would
    /// otherwise invent an answer for a permission this phone never sent.
    @Test func settlingAnUnclaimedRequestIsANoOp() {
        let before = claimable
        #expect(
            before.settling(
                terminal: "t1", request: "r9", outcome: .sent, message: "x",
                at: Date()) == before)
    }

    // MARK: - Recording what an agent is waiting on

    /// Nil is an observation, not an absence of one: the caller established
    /// that this agent is waiting on nothing.
    @Test func recordingNothingRemovesTheRecord() {
        #expect(claimable.recording(nil, for: "t1").permission(for: "t1") == nil)
    }

    /// A different permission makes the standing answer misleading — it is
    /// about a question that is over — so it goes with it.
    @Test func aNewPermissionDropsTheAnswerToTheOldOne() {
        let now = Date(timeIntervalSince1970: 2000)
        let answered = claimable
            .claiming(terminal: "t1", request: "r1", option: "a", optionName: "Yes", at: now)?
            .settling(terminal: "t1", request: "r1", outcome: .sent, message: "Sent “Yes”.", at: now)
        let next = answered?.recording(
            permission([option("a", "Yes", "allow_once")], request: "r2"), for: "t1")
        #expect(next?.permission(for: "t1")?.request == "r2")
        #expect(next?.answer(for: "t1") == nil)
    }

    /// …and re-observing the SAME permission keeps it, so a failure stays on
    /// screen while the card reporting it is still about the request that
    /// failed.
    @Test func reObservingTheSamePermissionKeepsTheAnswer() {
        let now = Date(timeIntervalSince1970: 2000)
        let answered = claimable
            .claiming(terminal: "t1", request: "r1", option: "a", optionName: "Yes", at: now)?
            .settling(
                terminal: "t1", request: "r1", outcome: .nothingSent, message: "Nothing sent.",
                at: now)
        let next = answered?.recording(
            permission([option("a", "Yes", "allow_once")], request: "r1"), for: "t1")
        #expect(next?.answer(for: "t1")?.message == "Nothing sent.")
    }

    /// Clearing after a successful send takes the buttons down and leaves the
    /// account of the answer standing.
    @Test func clearingAPermissionLeavesTheAnswerBehind() {
        let now = Date(timeIntervalSince1970: 2000)
        let sent = claimable
            .claiming(terminal: "t1", request: "r1", option: "a", optionName: "Yes", at: now)?
            .settling(terminal: "t1", request: "r1", outcome: .sent, message: "Sent “Yes”.", at: now)
            .clearingPermission(for: "t1")
        #expect(sent?.permission(for: "t1") == nil)
        #expect(sent?.answer(for: "t1")?.outcome == .sent)
    }

    // MARK: - Filing from the fleet poll

    /// A new ask is filed, which is what puts Allow and Deny on the card for a
    /// claude TUI pane.
    @Test func filingANewAskRecordsIt() {
        let next = GlancePermissions().filing(
            permission([option("allow", "Allow Bash(touch x)", "allow_once")], request: "hook-ask-1"),
            for: "t1")
        #expect(next?.permission(for: "t1")?.request == "hook-ask-1")
    }

    /// The ask already on file is not written again. The poll files every
    /// blocked pane every three seconds.
    @Test func filingTheAskAlreadyOnFileWritesNothing() {
        #expect(
            claimable.filing(permission([option("a", "Yes", "allow_once")]), for: "t1") == nil)
    }

    /// Nothing pending, where something was: filed, so the buttons come down
    /// when the ask resolved somewhere else.
    @Test func filingNothingClearsAnAskThatResolvedElsewhere() {
        let next = claimable.filing(nil, for: "t1")
        #expect(next != nil)
        #expect(next?.permission(for: "t1") == nil)
    }

    /// Nothing pending, and nothing on file: no write.
    @Test func filingNothingOverNothingWritesNothing() {
        #expect(GlancePermissions().filing(nil, for: "t1") == nil)
    }

    /// A read that left before this phone's answer and came back after it
    /// still shows the ask. Filing it would drop the answer and put the
    /// buttons back for a question that is over.
    @Test func anAskThisPhoneAnsweredIsNotFiledAgain() {
        let now = Date(timeIntervalSince1970: 2000)
        let sent = claimable
            .claiming(terminal: "t1", request: "r1", option: "a", optionName: "Yes", at: now)?
            .settling(terminal: "t1", request: "r1", outcome: .sent, message: "Sent “Yes”.", at: now)
            .clearingPermission(for: "t1")
        #expect(sent?.filing(permission([option("a", "Yes", "allow_once")]), for: "t1") == nil)
    }

    /// …unless nothing was sent, which hands the buttons back.
    @Test func anAskWhoseAnswerSentNothingIsFiledAgain() {
        let now = Date(timeIntervalSince1970: 2000)
        let unsent = claimable
            .claiming(terminal: "t1", request: "r1", option: "a", optionName: "Yes", at: now)?
            .settling(
                terminal: "t1", request: "r1", outcome: .nothingSent, message: "Nothing sent.",
                at: now)
            .clearingPermission(for: "t1")
        let next = unsent?.filing(permission([option("a", "Yes", "allow_once")]), for: "t1")
        #expect(next?.permission(for: "t1")?.request == "r1")
    }

    /// A tap on a hook ask the runner no longer holds settles as `.over`, and
    /// the buttons stay off: a second tap could only be refused again.
    ///
    /// Mutation: `.over` handing the buttons back. Red: a second claim.
    @Test func aNotHeldHookAskIsOverAndKeepsTheButtonsOff() {
        let now = Date(timeIntervalSince1970: 2000)
        let over = claimable
            .claiming(terminal: "t1", request: "r1", option: "a", optionName: "Allow", at: now)?
            .settling(
                terminal: "t1", request: "r1", outcome: .over,
                message: "Answered on another device.", at: now)
        #expect(over?.answer(for: "t1")?.refusesAnotherTap == true)
        #expect(
            over?.claiming(terminal: "t1", request: "r1", option: "b", optionName: "Deny", at: now)
                == nil)
    }

    /// A read that still shows the ask after it was refused as over must not
    /// file it again and put the buttons back.
    ///
    /// Mutation: `.over` handing the buttons back. Red: the ask is refiled.
    @Test func anOverAnswerIsNotRefiledAsPending() {
        let now = Date(timeIntervalSince1970: 2000)
        let over = claimable
            .claiming(terminal: "t1", request: "r1", option: "a", optionName: "Allow", at: now)?
            .settling(
                terminal: "t1", request: "r1", outcome: .over,
                message: "Too late here. Answer it in the terminal.", at: now)
            .clearingPermission(for: "t1")
        #expect(over?.filing(permission([option("a", "Yes", "allow_once")]), for: "t1") == nil)
    }

    /// Neither list may grow for the life of an install. The file is read on a
    /// render path.
    @Test func neitherListGrowsWithoutBound() {
        var store = GlancePermissions()
        for index in 0..<(GlancePermissions.limit + 5) {
            let id = "t\(index)"
            store = store.recording(
                permission([option("a", "Yes", "allow_once")], terminal: id), for: id)
            store =
                store.claiming(
                    terminal: id, request: "r1", option: "a", optionName: "Yes", at: Date())
                ?? store
        }
        #expect(store.pending.count == GlancePermissions.limit)
        #expect(store.answers.count == GlancePermissions.limit)
    }

    // MARK: - Staleness

    /// A card can sit on a lock screen overnight. An answer from yesterday must
    /// not be reported as news, and it must not go on refusing taps forever.
    @Test func anOldAnswerStopsCounting() {
        let sent = Date(timeIntervalSince1970: 10_000)
        let answer = GlanceAnswer(
            terminal: "t1", request: "r1", option: "a", optionName: "Yes",
            outcome: .sent, message: "Sent “Yes”.", at: sent)
        #expect(answer.isFresh(at: sent.addingTimeInterval(60)))
        #expect(!answer.isFresh(at: sent.addingTimeInterval(GlanceAnswer.freshFor + 1)))
        // A clock that moved backwards expires a record rather than making it
        // immortal.
        #expect(!answer.isFresh(at: sent.addingTimeInterval(-(GlanceAnswer.freshFor + 1))))
    }

    // MARK: - The file itself

    /// The app writes it and the widget extension reads it. Two binaries, one
    /// encoder — the same reason `SnapshotStore` pins its date strategy.
    @Test func theFileComesBackAsItWentIn() throws {
        let dir = URL(fileURLWithPath: NSTemporaryDirectory())
            .appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: dir) }

        let stored = claimable.claiming(
            terminal: "t1", request: "r1", option: "a", optionName: "Yes",
            at: Date(timeIntervalSince1970: 2000))!
        try GlancePermissionStore.write(stored, toContainer: dir)
        #expect(GlancePermissionStore.read(fromContainer: dir) == stored)
    }

    /// Filing through the store redraws the card, and only on a change. A
    /// writer that did not redraw left the lock screen card without buttons
    /// until the next push, whichever writer came second.
    @Test func filingThroughTheStoreRedrawsOnAChangeOnly() throws {
        let dir = URL(fileURLWithPath: NSTemporaryDirectory())
            .appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: dir) }

        var redrawn: [String] = []
        let ask = permission([option("allow", "Allow touch x", "allow_once")], request: "hook-ask-1")
        GlancePermissionStore.file(ask, for: "t1", inContainer: dir) { redrawn.append($0) }
        #expect(GlancePermissionStore.read(fromContainer: dir).permission(for: "t1") == ask)
        GlancePermissionStore.file(ask, for: "t1", inContainer: dir) { redrawn.append($0) }
        GlancePermissionStore.file(nil, for: "t1", inContainer: dir) { redrawn.append($0) }
        #expect(redrawn == ["t1", "t1"])
        #expect(GlancePermissionStore.read(fromContainer: dir).permission(for: "t1") == nil)
    }

    // MARK: - Buttons from the card's own ask (ov-57, T0 contract C5)

    private let until = Date(timeIntervalSince1970: 1_790_551_063)
    private var before: Date { until.addingTimeInterval(-30) }

    private func card(
        status: String = "blocked", terminal: String = "t1", workspace: String = "Billing",
        ask: CardAsk? = CardAsk(id: "hook-ask-1", tool: "Bash", until: Date(timeIntervalSince1970: 1_790_551_063))
    ) -> AgentCardState {
        AgentCardState(
            terminal: terminal, label: "claude", workspace: workspace, status: status,
            detail: "Run this command?", ask: ask)
    }

    /// What the store holds for a Bash ask the app saw: the allow's name IS the
    /// command line (`permission_options` in agent-core).
    private func storeRecord(request: String = "hook-ask-1") -> GlancePermissions {
        GlancePermissions(pending: [
            GlancePermission(
                terminal: "t1", request: request,
                options: [
                    option("allow-from-store", "Allow touch x", "allow_once"),
                    option("deny-from-store", "Deny", "reject_once"),
                ],
                observedAt: Date(timeIntervalSince1970: 1000))
        ])
    }

    /// A suspended app never saw the ask, so the store is empty and the card's
    /// ask alone gives the buttons: exactly "Allow" and "Deny", and the ids the
    /// daemon's hook ask takes.
    ///
    /// Mutation: `CardAskSource` ignoring `state.ask`. Red: nil.
    @Test func aCardAskGivesAllowAndDenyWithNoCommand() {
        let permission = CardAskSource.permission(store: .empty, state: card(), now: before)
        #expect(permission?.terminal == "t1")
        #expect(permission?.request == "hook-ask-1")
        #expect(permission?.options == [
            option("allow", "Allow", "allow_once"), option("deny", "Deny", "reject_once"),
        ])
        #expect(permission?.plainYes?.id == "allow")
        #expect(permission?.fit(lines: 3, columns: 40).shown.count == 2)
    }

    /// The app saw this very ask, so its record is the better source for the
    /// option ids. The words are still the card's: a store name for Bash is the
    /// command line, and the locked card never draws one.
    ///
    /// Mutation: the store record returned as filed. Red: "Allow touch x".
    @Test func aStoreRecordForTheSameAskWinsButKeepsTheCardsWords() {
        let permission = CardAskSource.permission(store: storeRecord(), state: card(), now: before)
        #expect(permission?.request == "hook-ask-1")
        #expect(permission?.options == [
            option("allow-from-store", "Allow", "allow_once"),
            option("deny-from-store", "Deny", "reject_once"),
        ])
        #expect(permission?.observedAt == Date(timeIntervalSince1970: 1000))
    }

    /// A record about an older ask on the same pane says nothing about this one.
    ///
    /// Mutation: the store record preferred whatever its request. Red: the old id.
    @Test func aStoreRecordForAnotherAskLosesToTheCard() {
        let permission = CardAskSource.permission(
            store: storeRecord(request: "hook-ask-0"), state: card(), now: before)
        #expect(permission?.request == "hook-ask-1")
        #expect(permission?.options.map { $0.id } == ["allow", "deny"])
    }

    /// Past the hold, a tap could only be refused, so there are no buttons,
    /// whichever source would have given them. At `until` exactly is past.
    ///
    /// Mutation: the `until` check removed. Red: buttons at and after `until`.
    @Test func anExpiredCardAskGivesNoButtons() {
        for now in [until, until.addingTimeInterval(1)] {
            #expect(CardAskSource.permission(store: .empty, state: card(), now: now) == nil)
            #expect(CardAskSource.permission(store: storeRecord(), state: card(), now: now) == nil)
        }
    }

    /// The push is as fresh as the last thing that happened, and a leader it
    /// says is working or done gets no buttons, whatever the ask or the store.
    ///
    /// Mutation: the status gate removed. Red: buttons on a working card.
    @Test func aCardAskOnAWorkingLeaderGivesNoButtons() {
        for status in ["working", "done", ""] {
            #expect(
                CardAskSource.permission(store: storeRecord(), state: card(status: status), now: before)
                    == nil, "\(status)")
        }
        #expect(
            CardAskSource.permission(store: storeRecord(), state: card(terminal: ""), now: before)
                == nil)
    }

    /// A relay older than the ask sends none, and the card behaves as it did
    /// before: the store's record, as filed.
    ///
    /// Mutation: no ask treated as no buttons. Red: nil.
    @Test func aCardWithNoAskKeepsTheStoreRecord() {
        let permission = CardAskSource.permission(
            store: storeRecord(), state: card(ask: nil), now: before)
        #expect(permission == storeRecord().permission(for: "t1"))
        #expect(CardAskSource.permission(store: .empty, state: card(ask: nil), now: before) == nil)
    }

    /// The locked card's line: the tool and the workspace, whichever it has,
    /// and never the command.
    ///
    /// Mutation: the workspace left out. Red: "Bash".
    @Test func theLockedCaptionIsToolAndWorkspace() {
        #expect(CardAskWording.caption(tool: "Bash", workspace: "Billing") == "Bash · Billing")
        #expect(CardAskWording.caption(tool: "Bash", workspace: "") == "Bash")
        #expect(CardAskWording.caption(tool: nil, workspace: "Billing") == "Billing")
        #expect(CardAskWording.caption(tool: nil, workspace: "") == nil)
    }

    // MARK: - What the card's leader offers (ov-57 T-iOS-1)

    /// A suspended app, a locked phone, a card ask: buttons, the tool and the
    /// workspace in place of the question, and the hold's end for a countdown.
    ///
    /// Mutation: `caption` read from the store rather than the card ask. Red: nil.
    @Test func aCardAskLeadsWithItsCaptionAndHoldEnd() {
        let leader = CardLeaderAsk.current(store: .empty, state: card(), now: before)
        #expect(leader.isPresent)
        #expect(leader.offersButtons)
        #expect(leader.caption == "Bash · Billing")
        #expect(leader.until == until)
        #expect(leader.permission?.request == "hook-ask-1")
    }

    /// An ask the app filed and the card did not carry keeps today's card:
    /// no caption (the question stays), no countdown.
    ///
    /// Mutation: the caption drawn with no card ask. Red: "Billing".
    @Test func aStoreOnlyAskHasNoCaptionOrCountdown() {
        let leader = CardLeaderAsk.current(store: storeRecord(), state: card(ask: nil), now: before)
        #expect(leader.offersButtons)
        #expect(leader.caption == nil)
        #expect(leader.until == nil)
    }

    /// A tap that came back `.over` keeps the buttons off and its sentence on,
    /// even though the card still carries the ask.
    ///
    /// Mutation: `offersButtons` ignoring the answer. Red: buttons back.
    @Test func anOverAnswerKeepsTheCardsButtonsOff() {
        let now = before
        let over = GlancePermissions()
            .claiming(terminal: "t1", request: "hook-ask-1", option: "allow", optionName: "Allow", at: now)!
            .settling(
                terminal: "t1", request: "hook-ask-1", outcome: .over,
                message: "Answered on another device.", at: now)
        let leader = CardLeaderAsk.current(store: over, state: card(), now: now)
        #expect(leader.isPresent)
        #expect(!leader.offersButtons)
        #expect(leader.answer?.outcome == .over)
    }

    /// An answer about an older ask says nothing about this one, and the card
    /// offers this one's buttons with no sentence under them.
    ///
    /// Mutation: the request check on the answer removed. Red: the old answer
    /// is drawn and the buttons are off.
    @Test func anAnswerToAnotherAskIsDropped() {
        let now = before
        let old = GlancePermissions()
            .claiming(terminal: "t1", request: "hook-ask-0", option: "allow", optionName: "Allow", at: now)!
            .settling(terminal: "t1", request: "hook-ask-0", outcome: .sent, message: "Sent", at: now)
        let leader = CardLeaderAsk.current(store: old, state: card(), now: now)
        #expect(leader.answer == nil)
        #expect(leader.offersButtons)
    }

    /// Past the hold the buttons go, but a sentence about this phone's own tap
    /// stays until it is stale.
    ///
    /// Mutation: the answer dropped with the permission. Red: nothing present.
    @Test func pastTheHoldOnlyTheSentenceIsLeft() {
        let sent = GlancePermissions()
            .claiming(terminal: "t1", request: "hook-ask-1", option: "deny", optionName: "Deny", at: before)!
            .settling(terminal: "t1", request: "hook-ask-1", outcome: .sent, message: "Sent “Deny”.", at: before)
        let leader = CardLeaderAsk.current(store: sent, state: card(), now: until.addingTimeInterval(5))
        #expect(leader.permission == nil)
        #expect(leader.answer?.message == "Sent “Deny”.")
        #expect(!leader.offersButtons)
        #expect(CardLeaderAsk.current(store: .empty, state: card(), now: until) == .none)
    }

    /// Provisional D2: an answer that allows needs an unlock, one that refuses
    /// does not.
    ///
    /// Mutation: `needsUnlock` true for every option. Red: Deny needs an unlock.
    @Test func onlyAnAllowNeedsAnUnlock() {
        #expect(option("allow", "Allow", "allow_once").needsUnlock)
        #expect(option("a", "Yes", "allow_always").needsUnlock)
        #expect(!option("deny", "Deny", "reject_once").needsUnlock)
        #expect(!option("z", "Go on", "proceed").needsUnlock)
    }

    /// An unreadable file says the same thing as an absent one — nothing is
    /// known — and a card with no buttons is the correct rendering of that.
    @Test func anUnreadableFileReadsAsEmpty() {
        let dir = URL(fileURLWithPath: NSTemporaryDirectory())
            .appendingPathComponent(UUID().uuidString)
        #expect(GlancePermissionStore.read(fromContainer: dir) == .empty)
    }
}
