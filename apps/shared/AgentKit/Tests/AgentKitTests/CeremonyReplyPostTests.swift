import Foundation
import Testing

@testable import AgentKit

/// The phone posting a reply sealed to a browser's key (ov-770): one POST,
/// to the reply route, with the relay's three fields and nothing more.
@MainActor
struct CeremonyReplyPostTests {
    @MainActor final class Relay {
        var answer: Result<DirectoryReply, AccountError>
        private(set) var sent: [(path: String, body: [String: Any])] = []

        init(_ answer: Result<DirectoryReply, AccountError>) { self.answer = answer }

        var client: DirectoryClient {
            DirectoryClient { path, body in
                self.sent.append((path, body))
                return self.answer
            }
        }
    }

    static let ceremony = "0123456789abcdef0123456789abcdef"

    @Test("The sealed reply is posted once, to the reply route, as ceremony, account and sealed")
    func postsTheThreeFields() async {
        let relay = Relay(.success(DirectoryReply(status: 200, body: ["ok": true])))
        let outcome = await relay.client.postCeremonyReply(
            ceremony: Self.ceremony, account: "user_alice", sealed: "c2VhbGVk")
        #expect(outcome == .posted)
        #expect(relay.sent.map(\.path) == ["/v1/ceremony/reply"])
        #expect(Set(relay.sent[0].body.keys) == ["ceremony", "account", "sealed"])
        #expect(relay.sent[0].body["ceremony"] as? String == Self.ceremony)
        #expect(relay.sent[0].body["account"] as? String == "user_alice")
        #expect(relay.sent[0].body["sealed"] as? String == "c2VhbGVk")
    }

    @Test("Each of the relay's answers maps to its own outcome")
    func mapsTheRelaysAnswers() async {
        let cases: [(Result<DirectoryReply, AccountError>, CeremonyReplyPost)] = [
            (.success(DirectoryReply(status: 409, body: ["error": "taken"])), .alreadyPosted),
            (.success(DirectoryReply(status: 403, body: ["error": "forbidden"])), .refused),
            (.success(DirectoryReply(status: 413, body: [:])), .failed),
            (.success(DirectoryReply(status: 500, body: [:])), .failed),
            (.failure(.endpointMissing), .failed),
        ]
        for (answer, expected) in cases {
            let relay = Relay(answer)
            let outcome = await relay.client.postCeremonyReply(ceremony: Self.ceremony, account: "a", sealed: "s")
            #expect(outcome == expected)
            #expect(relay.sent.count == 1, "never retried")
        }
    }
}
