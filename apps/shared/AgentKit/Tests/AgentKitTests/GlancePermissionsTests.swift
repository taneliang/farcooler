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

    /// An unreadable file says the same thing as an absent one — nothing is
    /// known — and a card with no buttons is the correct rendering of that.
    @Test func anUnreadableFileReadsAsEmpty() {
        let dir = URL(fileURLWithPath: NSTemporaryDirectory())
            .appendingPathComponent(UUID().uuidString)
        #expect(GlancePermissionStore.read(fromContainer: dir) == .empty)
    }
}
