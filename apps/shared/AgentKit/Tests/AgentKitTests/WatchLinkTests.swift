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
        ]
    }

    private var everyReply: [WatchReply] {
        [
            .sent,
            .failed("the runner is not reachable"),
            .permission(permission()),
            .permission(nil),
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
        #expect(WatchReply(dictionary: ["kind": "failed"]) == nil)
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
