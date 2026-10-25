import Foundation
import Testing

@testable import AgentKit

// ov-746: an answered ask says who answered on a runner with members.

@Suite struct AskAnsweredWhoTests {
    private func ask(person: [String: String]? = nil, for other: [String: String]? = nil) throws -> AgentRow.Ask {
        var body: [String: Any] = [
            "kind": "Question", "text": "Which color?", "tool": "AskUserQuestion", "asked_ms": 1,
            "answered": true, "answered_by": "iPhone",
        ]
        if let person { body["answered_person"] = person }
        if let other { body["answered_for"] = other }
        let row: [String: Any] = ["id": "ask:1", "kind": ["Ask": body], "ord": 0, "provisional": false, "rev": 1]
        let page = try JSONSerialization.data(withJSONObject: ["epoch": 1, "rev": 1, "moreBefore": false, "rows": [row]])
        guard case .ask(let ask) = try AgentRowPage.decode(page).rows[0].kind else { throw AgentRowDecodeError(what: "not an ask") }
        return ask
    }

    private let bob = ["handle": "bob", "name": "Bob"]
    private let alice = ["handle": "alice", "name": "Alice"]
    private let owner = ["handle": "", "name": ""]

    @Test("A solo runner names no one, so the title is the device's, as it was")
    func soloIsUnchanged() throws {
        let solo = try ask()
        #expect(solo.answeredPerson == nil && solo.answeredFor == nil)
        #expect(AgentConversation.askTitle(solo, viewer: "") == "Answered on iPhone")
        #expect(AgentConversation.askTitle(solo) == "Answered on iPhone")
    }

    @Test("Who answered, for you and for others")
    func sentences() throws {
        #expect(AgentConversation.askTitle(try ask(person: bob), viewer: "bob") == "You answered")
        #expect(AgentConversation.askTitle(try ask(person: bob), viewer: "") == "Bob answered")
        #expect(AgentConversation.askTitle(try ask(person: bob, for: alice), viewer: "") == "Bob answered for Alice")
        #expect(AgentConversation.askTitle(try ask(person: alice, for: bob), viewer: "bob") == "Alice answered for you")
        #expect(AgentConversation.askTitle(try ask(person: alice, for: bob), viewer: "alice") == "You answered for Bob")
        #expect(AgentConversation.askTitle(try ask(person: alice, for: owner), viewer: "bob") == "Alice answered for the owner")
    }

    @Test("The owner is the empty handle, and says You on the owner's own device")
    func ownerIsEmptyHandle() throws {
        #expect(AgentConversation.askTitle(try ask(person: owner), viewer: "") == "You answered")
        #expect(AgentConversation.askTitle(try ask(person: owner), viewer: "bob") == "The owner answered")
        #expect(AgentConversation.askTitle(try ask(person: owner, for: bob), viewer: "bob") == "The owner answered for you")
    }

    @Test("A device not yet told who it is, and a released one, never get a You")
    func unknownViewer() throws {
        #expect(AgentConversation.askTitle(try ask(person: bob), viewer: nil) == "Answered on iPhone")
        #expect(AgentConversation.askTitle(try ask(person: owner), viewer: RunnerPeople.released) == "The owner answered")
    }

    @Test("A member with no name is said by handle, as sent")
    func nameless() throws {
        #expect(AgentConversation.askTitle(try ask(person: ["handle": "bob", "name": ""]), viewer: "") == "bob answered")
    }

    @Test("A held ask still waiting reads as the question, whoever is named")
    func stillWaiting() throws {
        var waiting = try ask(person: bob)
        waiting.answered = false
        waiting.held = "hook-ask-1"
        #expect(AgentConversation.askTitle(waiting, viewer: "") == "Claude is asking a question")
    }

    @Test("A name is used exactly as sent, never re-cased")
    func nameKeptAsSent() throws {
        let berg = ["handle": "vdb", "name": "van der Berg"]
        #expect(AgentConversation.askTitle(try ask(person: berg), viewer: "") == "van der Berg answered")
        #expect(AgentConversation.askTitle(try ask(person: bob, for: berg), viewer: "") == "Bob answered for van der Berg")
        #expect(AgentConversation.askTitle(try ask(person: ["handle": "bob", "name": ""]), viewer: "") == "bob answered")
    }

    @Test("An answer for the answerer's own ask drops the for clause")
    func forSelfDropped() throws {
        #expect(AgentConversation.askTitle(try ask(person: bob, for: bob), viewer: "") == "Bob answered")
        #expect(AgentConversation.askTitle(try ask(person: bob, for: bob), viewer: "bob") == "You answered")
        #expect(AgentConversation.askTitle(try ask(person: owner, for: owner), viewer: "bob") == "The owner answered")
    }
}
