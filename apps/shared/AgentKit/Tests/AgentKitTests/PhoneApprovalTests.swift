import Foundation
import Testing

@testable import AgentKit

/// A Mac asks the person's iPhone to approve an admin action (ov-858): the
/// order of the calls, what the iPhone is shown, and the words.
struct PhoneApprovalTests {
    final class Core: StepUpCalling, @unchecked Sendable {
        var calls: [(method: String, args: [String: Any])] = []
        var states: [String]
        init(_ states: [String]) { self.states = states }

        func call(_ method: String, _ args: [String: Any]) async throws -> Data {
            calls.append((method, args))
            switch method {
            case "approval.ask": return Data(#"{"id":"a-1","state":"pending","expiresAt":9}"#.utf8)
            case "approval.status":
                let state = states.count > 1 ? states.removeFirst() : (states.first ?? "pending")
                return Data(#"{"id":"a-1","state":"\#(state)","expiresAt":9}"#.utf8)
            default: return Data(#"{"clients":[]}"#.utf8)
            }
        }

        func refusalWord(_ error: Error) -> String? { nil }
        var methods: [String] { calls.map(\.method) }
    }

    @Test("The Mac asks, waits while it is pending, and sends only once it is granted")
    func waitsThenSends() async throws {
        let core = Core(["pending", "pending", "granted"])
        var waiting: [Bool] = []
        let answer = try await PhoneApproval(core: core, pause: 1_000, looks: 10)
            .perform("client.revoke", ["clientId": "pad-b"], waiting: { waiting.append($0) })
        #expect(core.methods == ["approval.ask", "approval.status", "approval.status", "approval.status", "approval.finish"], "\(core.methods)")
        #expect(core.calls[0].args["method"] as? String == "client.revoke")
        #expect(core.calls[4].args["id"] as? String == "a-1")
        #expect(waiting == [true, false] && String(data: answer, encoding: .utf8) == #"{"clients":[]}"#)
    }

    @Test("A no, a lapse, or a silence sends nothing")
    func nothingIsSentWithoutAGrant() async throws {
        for (states, word) in [(["denied"], "approval_denied"), (["expired"], "approval_expired"), (["pending"], "approval_expired")] {
            let core = Core(states)
            await #expect(throws: StepUpError.refused(word)) {
                try await PhoneApproval(core: core, pause: 1_000, looks: 3).perform("client.revoke", [:], waiting: { _ in })
            }
            #expect(!core.methods.contains("approval.finish"), "\(word)")
        }
    }

    static let listed = #"""
        {"items":[{"id":"a-1","state":"pending","action":{"method":"member.set_access","detail":{"handle":"cosmo","access":"viewer"}},
        "deviceId":"mac-d","deviceName":"Dans-Mac","targetName":"Cosmo","expiresAt":9,"statement":"c3RhdGVtZW50"},
        {"id":"bad","action":{"method":"x"}}]}
        """#

    @Test("The iPhone is shown the action, its target and which Mac asked, and signs the statement")
    func whatTheIPhoneShows() throws {
        let items = PhoneApprovalItem.items(in: Data(Self.listed.utf8))
        let item = try #require(items.first)
        #expect(items.count == 1, "an entry with no statement is left out")
        #expect(item.sentence(runner: "box") == "Make Cosmo a Viewer on box.")
        #expect(item.mac == "Dans-Mac" && PhoneApprovalWords.asked(by: item.mac) == "Dans-Mac asked for this. Only approve it if that was you.")
        #expect(item.statement == Data("statement".utf8))
    }

    @Test("Each action is worded for what it does")
    func sentences() throws {
        func item(_ method: String, _ detail: String) throws -> PhoneApprovalItem {
            let json = #"{"items":[{"id":"a","action":{"method":"\#(method)","detail":\#(detail)},"deviceName":"","targetName":"Cosmos-iPad","statement":"AA=="}]}"#
            return try #require(PhoneApprovalItem.items(in: Data(json.utf8)).first)
        }
        #expect(try item("client.revoke", #"{"withAdded":false}"#).sentence(runner: "box") == "Remove Cosmos-iPad from box.")
        #expect(try item("client.revoke", #"{"withAdded":true}"#).sentence(runner: "box") == "Remove Cosmos-iPad and the devices it added.")
        #expect(try item("client.set_cap", #"{"cap":"viewer"}"#).sentence(runner: "box") == "Limit Cosmos-iPad on box to Viewer.")
        #expect(try item("client.approve_step_up", "{}").sentence(runner: "box") == "Allow Cosmos-iPad to approve changes to who has access.")
        #expect(try item("client.revoke", "{}").mac == "A Mac")
    }

    @Test("The screen's text names the whole target, and an action it can't name is never shown")
    func theFullTargetOrNothing() throws {
        func items(_ method: String, _ detail: String, target: String = "Cosmo") -> [PhoneApprovalItem] {
            let json = #"{"items":[{"id":"a","action":{"method":"\#(method)","detail":\#(detail)},"deviceName":"Mac","targetName":"\#(target)","statement":"AA=="}]}"#
            return PhoneApprovalItem.items(in: Data(json.utf8))
        }
        let said = try #require(items("member.set_access", #"{"access":"admin"}"#).first).sentence(runner: "box")
        #expect(said.contains("Cosmo") && said.contains("Admin"), "\(said): who and which access")
        #expect(items("client.enroll", #"{"label":"X"}"#).isEmpty, "a key's registration is never shown")
        #expect(items("member.set", "{}").isEmpty && items("member.set_role", "{}").isEmpty)
        #expect(items("member.set_access", #"{"access":"nonsense"}"#).isEmpty, "an access it can't name")
        #expect(items("client.revoke", "{}", target: "").isEmpty, "a target it can't name")
    }

    @Test("Each refusal has a sentence with no runner word in it")
    func words() {
        for word in ["no_approver", "approval_denied", "approval_expired"] {
            let said = StepUpWords.sentence(forRefusal: word, runner: "box", device: "Mac")
            #expect(said != "Couldn’t do that. Try again." && !said.contains("_"), "\(word): \(said)")
        }
    }
}
