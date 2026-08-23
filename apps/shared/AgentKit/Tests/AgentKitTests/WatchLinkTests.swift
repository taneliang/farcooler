import Foundation
import Testing

@testable import AgentKit

/// The vocabulary the watch and the phone share, checked the way the transport
/// will actually use it.
///
/// Two things are being defended here and only one of them is round-tripping.
/// The other is that every value handed to `WCSession` is a property-list type:
/// that check has no counterpart at compile time, and the runtime's answer to
/// failing it is to drop the message, which reaches the person as an Allow
/// button that did nothing.
struct WatchLinkTests {
    // MARK: - The property-list rule

    /// Whether `value` is something `WCSession` will carry, all the way down.
    ///
    /// Recursive rather than a look at the top level, because the top level of
    /// every message here is Strings and would pass no matter what went wrong.
    /// The place a non-property-list value can actually hide is inside the
    /// options array of a permission reply — one level of array and one of
    /// dictionary below anything a spot check would see.
    private func isPropertyList(_ value: Any) -> Bool {
        switch value {
        case is String, is Int, is Double, is Bool, is Data, is Date:
            return true
        case let array as [Any]:
            return array.allSatisfy(isPropertyList)
        case let dictionary as [String: Any]:
            return dictionary.values.allSatisfy(isPropertyList)
        default:
            return false
        }
    }

    /// A value `WCSession` refuses, so the checker above is known to be capable
    /// of failing. A recursive test that cannot fail is worse than no test: it
    /// reads as coverage of the one rule nothing else enforces.
    private struct NotAPropertyList {}

    @Test func theTypeCheckCanActuallyFail() {
        #expect(isPropertyList(["fine": "yes"]))
        #expect(!isPropertyList(["bad": NotAPropertyList()]))
        // Nested exactly where a real one would hide: inside an array of
        // dictionaries, which is the shape a permission's options take.
        #expect(!isPropertyList(["options": [["kind": NotAPropertyList()]]]))
    }

    private func permission() -> WatchPermission {
        WatchPermission(
            id: "perm-1",
            toolCall: "Bash(rm -rf build)",
            options: [
                WatchPermissionOption(id: "allow", name: "Allow", kind: "allow_once"),
                WatchPermissionOption(id: "reject", name: "Deny", kind: "reject_once"),
            ])
    }

    private var everyRequest: [WatchRequest] {
        [
            .prompt(terminal: "t1", text: "keep going"),
            .answer(terminal: "t1", request: "perm-1", option: "allow"),
            .pendingPermission(terminal: "t1"),
            .transcript(terminal: "t1"),
        ]
    }

    private func transcript() -> WatchTranscript {
        WatchTranscript(
            entries: [
                WatchTranscriptEntry(role: "User", text: "does the watch build?"),
                WatchTranscriptEntry(role: "Agent", text: "Yes — all five targets."),
            ],
            complete: false)
    }

    private var everyReply: [WatchReply] {
        [
            .sent,
            .failed("the runner is not reachable"),
            .permission(permission()),
            .permission(nil),
            .transcript(transcript()),
            // An agent that has written nothing. A real answer, and one whose
            // `complete` is true — it must survive the trip as itself rather
            // than as the empty transcript a failed decode would produce.
            .transcript(WatchTranscript(entries: [], complete: true)),
        ]
    }

    @Test func everyRequestCarriesOnlyPropertyListTypes() {
        for request in everyRequest {
            #expect(isPropertyList(request.dictionary))
            #expect(
                PropertyListSerialization.propertyList(request.dictionary, isValidFor: .binary))
        }
    }

    /// The reply with a permission in it is the one that matters: its options
    /// are an array of dictionaries, which is the only nested structure in this
    /// vocabulary and therefore the only place a struct could survive unnoticed.
    @Test func everyReplyCarriesOnlyPropertyListTypes() {
        for reply in everyReply {
            #expect(isPropertyList(reply.dictionary))
            #expect(PropertyListSerialization.propertyList(reply.dictionary, isValidFor: .binary))
        }
    }

    // MARK: - Round trips

    @Test func everyRequestRoundTripsThroughItsDictionary() {
        for request in everyRequest {
            #expect(WatchRequest(dictionary: request.dictionary) == request)
        }
    }

    @Test func everyReplyRoundTripsThroughItsDictionary() {
        for reply in everyReply {
            #expect(WatchReply(dictionary: reply.dictionary) == reply)
        }
    }

    /// "Nothing pending" is an answer, not a failure — an agent can be blocked
    /// on a trust gate or a plain question. The two must not decode to each
    /// other, because one of them puts an error in front of the person.
    @Test func nothingPendingIsNotAFailure() {
        let nothing = WatchReply(dictionary: WatchReply.permission(nil).dictionary)
        #expect(nothing == .permission(nil))
        #expect(nothing != .failed("nothing pending"))
    }

    // MARK: - What an unrecognized or incomplete message does

    /// A watch meeting a phone that speaks a word it has not learned yet drops
    /// that one message rather than trapping. This is the whole reason these
    /// types are coded by hand — `Codable` would throw here, and a throw on the
    /// receiving side of a `WCSession` handler is a crash.
    @Test func anUnknownKindDecodesToNil() {
        #expect(WatchRequest(dictionary: ["kind": "interrupt", "terminal": "t1"]) == nil)
        #expect(WatchReply(dictionary: ["kind": "queued"]) == nil)
    }

    @Test func aMessageWithNoKindDecodesToNil() {
        #expect(WatchRequest(dictionary: ["terminal": "t1", "text": "hi"]) == nil)
        #expect(WatchReply(dictionary: [:]) == nil)
    }

    @Test func aMissingRequiredKeyDecodesToNil() {
        #expect(WatchRequest(dictionary: ["kind": "prompt", "terminal": "t1"]) == nil)
        #expect(WatchRequest(dictionary: ["kind": "answer", "terminal": "t1"]) == nil)
        #expect(WatchRequest(dictionary: ["kind": "pendingPermission"]) == nil)
        #expect(WatchRequest(dictionary: ["kind": "transcript"]) == nil)
        #expect(WatchReply(dictionary: ["kind": "failed"]) == nil)
        // `complete` is not optional and must not default. "These are all the
        // words there were" is a claim, and a reply that forgot to make it must
        // not have it made on its behalf in either direction.
        #expect(WatchReply(dictionary: ["kind": "transcript", "entries": []]) == nil)
        #expect(WatchReply(dictionary: ["kind": "transcript", "complete": true]) == nil)
    }

    /// One word naming both a request and a reply is safe only because each
    /// side refuses what the other sends. Pinned, because the day that stops
    /// being true is the day a watch reads a request as a conversation.
    @Test func aTranscriptRequestAndReplyCannotBeMistakenForEachOther() {
        let request = WatchRequest.transcript(terminal: "t1").dictionary
        let reply = WatchReply.transcript(transcript()).dictionary
        #expect(WatchReply(dictionary: request) == nil)
        #expect(WatchRequest(dictionary: reply) == nil)
    }

    /// The one field on this wire whose spelling is pinned to a type the watch
    /// target cannot see.
    ///
    /// `WatchTranscriptEntry.isYours` compares against the literal `"User"`
    /// because `Role` lives in `AgentEvent.swift`, which the watch does not
    /// compile. That is a spelling in two places, which is what the whole of
    /// `WatchLink.swift` exists to avoid — so the seam gets a test instead.
    /// Change `Role.user`'s raw value and this fails, rather than the wearer's
    /// own words silently arriving as the agent's.
    @Test func theWearersOwnWordsAreRecognizedByTheStreamsSpelling() {
        #expect(WatchTranscriptEntry(role: Role.user.rawValue, text: "hi").isYours)
        #expect(!WatchTranscriptEntry(role: Role.agent.rawValue, text: "hi").isYours)
        // A role invented after this build shipped is somebody else's, which is
        // true, rather than the wearer's, which would not be.
        #expect(!WatchTranscriptEntry(role: "Narrator", text: "hi").isYours)
    }

    /// One unreadable entry costs the whole transcript, the way one unreadable
    /// option costs the whole permission.
    @Test func aTranscriptWithOneUnreadableEntryDecodesToNil() {
        let dictionary: [String: Any] = [
            "kind": "transcript",
            "entries": [["role": "Agent", "text": "fine"], ["role": "Agent"]],
            "complete": true,
        ]
        #expect(WatchReply(dictionary: dictionary) == nil)
    }

    // MARK: - What fits on a wrist

    private func entry(_ text: String) -> WatchTranscriptEntry {
        WatchTranscriptEntry(role: "Agent", text: text)
    }

    /// A short conversation arrives whole and in the order it happened.
    @Test func aShortConversationIsSentWholeAndOldestFirst() {
        let said = [entry("first"), entry("second"), entry("third")]
        let fitted = WatchTranscript.fitting(said, whole: true)
        #expect(fitted.entries.map(\.text) == ["first", "second", "third"])
        #expect(fitted.complete)
    }

    /// The caller's own doubt is carried through. A replay off a bounded window,
    /// or a stream with a gap in it, is not the whole conversation however
    /// comfortably it fits.
    @Test func aCallerThatKnowsItIsMissingSomethingSaysSo() {
        #expect(!WatchTranscript.fitting([entry("all there is")], whole: false).complete)
        #expect(WatchTranscript.fitting([], whole: true).complete)
        #expect(WatchTranscript.fitting([], whole: true).entries.isEmpty)
    }

    /// Past `entryLimit` it is the NEWEST messages that survive. Somebody
    /// raising their wrist wants what just happened, not the start of the day.
    @Test func theNewestMessagesAreTheOnesThatFit() {
        let said = (1...40).map { entry("message \($0)") }
        let fitted = WatchTranscript.fitting(said, whole: true)
        #expect(fitted.entries.count == WatchTranscript.entryLimit)
        #expect(fitted.entries.first?.text == "message 29")
        #expect(fitted.entries.last?.text == "message 40")
        #expect(!fitted.complete)
    }

    /// The byte budget bites before the count does when the messages are big,
    /// and it still keeps the newest.
    @Test func aFewHugeMessagesAreCutByBytesRatherThanByCount() {
        let big = String(repeating: "x", count: 6 * 1024)
        let said = (1...5).map { entry("\($0)" + big) }
        let fitted = WatchTranscript.fitting(said, whole: true)
        // Two whole ones fit in 16 KB; a third would not.
        #expect(fitted.entries.count == 2)
        #expect(fitted.entries.last?.text.hasPrefix("5") == true)
        #expect(fitted.entries.map(\.text.utf8.count).reduce(0, +) <= WatchTranscript.byteBudget)
        #expect(!fitted.complete)
    }

    /// One message bigger than the whole budget is still the message that was
    /// asked for. It is kept, clipped at the END so it reads from the top, and
    /// it is the only one — four older messages in its place would be four
    /// messages nobody wanted.
    @Test func oneEnormousMessageIsClippedRatherThanDropped() {
        let older = entry("something from an hour ago")
        let huge = entry(String(repeating: "y", count: WatchTranscript.byteBudget * 2))
        let fitted = WatchTranscript.fitting([older, huge], whole: true)
        #expect(fitted.entries.count == 1)
        #expect(fitted.entries[0].text.hasPrefix("yyy"))
        #expect(fitted.entries[0].text.hasSuffix("…"))
        #expect(fitted.entries[0].text.utf8.count <= WatchTranscript.byteBudget)
        #expect(!fitted.complete)
    }

    /// The clip lands between characters, never inside one.
    ///
    /// Slicing the UTF-8 view would be the obvious way to write it and would
    /// end every clipped message in a black diamond — in exactly the languages
    /// least able to spare one.
    @Test func aClippedMessageIsStillValidText() throws {
        let text = String(repeating: "日", count: WatchTranscript.byteBudget)
        let fitted = WatchTranscript.fitting([entry(text)], whole: true)
        let kept = try #require(fitted.entries.first).text
        #expect(!kept.contains("\u{FFFD}"))
        #expect(kept.utf8.count <= WatchTranscript.byteBudget)
        #expect(kept.dropLast().allSatisfy { $0 == "日" })
    }

    /// Whatever the budget did, the payload has to be something `WCSession`
    /// will actually carry — including at its very largest.
    @Test func aFullSizeTranscriptIsStillAPropertyList() {
        let said = (1...40).map { entry("message \($0) " + String(repeating: "z", count: 2_000)) }
        let reply = WatchReply.transcript(WatchTranscript.fitting(said, whole: true))
        #expect(isPropertyList(reply.dictionary))
        #expect(PropertyListSerialization.propertyList(reply.dictionary, isValidFor: .binary))
        #expect(WatchReply(dictionary: reply.dictionary) == reply)
    }

    /// A key present with the wrong type is the shape a mismatched build sends,
    /// and it must read as "cannot understand this" rather than as an empty
    /// prompt sent to a live agent.
    @Test func aKeyOfTheWrongTypeDecodesToNil() {
        #expect(WatchRequest(dictionary: ["kind": "prompt", "terminal": "t1", "text": 7]) == nil)
        #expect(WatchReply(dictionary: ["kind": "failed", "reason": ["a", "b"]]) == nil)
    }

    /// One unreadable option must cost the whole permission, not itself. The
    /// options are the answers the agent offered; showing three of four would
    /// have the person choose something they did not mean, believing they saw
    /// everything on offer.
    @Test func aPermissionWithOneUnreadableOptionDecodesToNil() {
        var dictionary = permission().dictionary
        dictionary["options"] = [
            ["id": "allow", "name": "Allow", "kind": "allow_once"],
            ["id": "reject", "name": "Deny"],
        ]
        #expect(WatchPermission(dictionary: dictionary) == nil)
        #expect(WatchReply(dictionary: ["kind": "permission", "permission": dictionary]) == nil)
    }

    @Test func aPermissionKeepsItsOptionsInTheOrderTheAgentOfferedThem() {
        let decoded = WatchPermission(dictionary: permission().dictionary)
        #expect(decoded?.options.map(\.id) == ["allow", "reject"])
        #expect(decoded?.toolCall == "Bash(rm -rf build)")
    }

    /// The field names have to match `PermissionOption` so the phone's
    /// conversion is a copy and a dropped field is visible in the diff.
    @Test func aPermissionOptionMirrorsTheEventStreamsOption() {
        let option = PermissionOption(id: "allow", name: "Allow", kind: "allow_once")
        let carried = WatchPermissionOption(id: option.id, name: option.name, kind: option.kind)
        #expect(carried.dictionary as? [String: String] == [
            "id": "allow", "name": "Allow", "kind": "allow_once",
        ])
    }
}
