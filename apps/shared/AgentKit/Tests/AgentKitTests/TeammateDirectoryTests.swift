import Foundation
import Testing

@testable import AgentKit

/// Adding a teammate by QR (ov-525): the directory's answers the Mac hands the
/// core, and the handle and name it suggests.
@MainActor
struct TeammateDirectoryTests {
    /// A relay that answers `me` and each roster from a script.
    @MainActor final class Relay {
        var rosters: [String: Result<DirectoryReply, AccountError>] = [:]
        var me: Result<DirectoryReply, AccountError> = .success(
            DirectoryReply(
                status: 200,
                body: [
                    "user": ["id": "user_alice", "email": "alice@example.com"],
                    "teams": [
                        ["id": "t-personal", "name": "Personal", "personal": true, "role": "admin"],
                        ["id": "t-platform", "name": "Platform", "personal": false, "role": "admin"],
                        ["id": "t-finance", "name": "Finance", "personal": false, "role": "billing"],
                    ],
                ]))
        private(set) var asked: [String] = []

        var client: DirectoryClient {
            DirectoryClient { path, body in
                self.asked.append(path == "/v1/directory/teams/members" ? "members:\(body["team"] as? String ?? "")" : path)
                switch path {
                case "/v1/directory/me": return self.me
                case "/v1/directory/teams/members":
                    return self.rosters[body["team"] as? String ?? ""] ?? .success(DirectoryReply(status: 404, body: [:]))
                default: return .failure(.endpointMissing)
                }
            }
        }
    }

    static func roster(_ team: String, _ members: [[String: Any]]) -> Result<DirectoryReply, AccountError> {
        .success(DirectoryReply(status: 200, body: ["team": ["id": team, "personal": false], "members": members]))
    }

    @Test("The directory's answers travel as they came: me, then each shared team's roster, never the personal one's")
    func answersTravelAsTheyCame() async throws {
        let relay = Relay()
        relay.rosters["t-platform"] = Self.roster("t-platform", [["user": "user_bob", "email": "bob@example.com", "role": "member"]])
        // Finance refuses a billing member's roster with a 403, and it's left
        // out rather than failing the whole answer: the core says whether it
        // needed it.
        relay.rosters["t-finance"] = .success(DirectoryReply(status: 403, body: ["error": "forbidden"]))
        let data = try #require(await relay.client.teamDirectory())
        let object = try #require(try JSONSerialization.jsonObject(with: data) as? [String: Any])
        let me = try #require(object["me"] as? [String: Any])
        #expect((me["user"] as? [String: Any])?["id"] as? String == "user_alice")
        let rosters = try #require(object["rosters"] as? [[String: Any]])
        #expect(rosters.count == 1)
        #expect(((rosters[0]["members"] as? [[String: Any]])?.first?["email"] as? String) == "bob@example.com")
        #expect(relay.asked == ["/v1/directory/me", "members:t-platform", "members:t-finance"])
    }

    @Test("No answer from me is no directory at all")
    func noMeNoDirectory() async {
        let relay = Relay()
        relay.me = .failure(.unreachable(reason: "offline"))
        #expect(await relay.client.teamDirectory() == nil)
        relay.me = .success(DirectoryReply(status: 500, body: [:]))
        #expect(await relay.client.teamDirectory() == nil)
    }

    @Test("A handle is the email's local part, made a handle and made unique")
    func handles() {
        #expect(TeammateNames.handle(email: "bob@example.com", taken: []) == "bob")
        #expect(TeammateNames.handle(email: "Bob.Smith+fc@example.com", taken: []) == "bob-smith-fc")
        #expect(TeammateNames.handle(email: "bob@example.com", taken: ["bob"]) == "bob-2")
        #expect(TeammateNames.handle(email: "bob@example.com", taken: ["bob", "bob-2"]) == "bob-3")
        #expect(TeammateNames.handle(email: "42@example.com", taken: []) == "teammate")
        #expect(TeammateNames.handle(email: "2bob@example.com", taken: []) == "bob")
        #expect(TeammateNames.handle(email: "user@example.com", taken: []) == "user-2", "the runner keeps `user`")
        let long = TeammateNames.handle(email: String(repeating: "a", count: 40) + "@example.com", taken: [])
        #expect(long.count == 32 && TeammateNames.isValidHandle(long))
        let longTaken = TeammateNames.handle(email: String(repeating: "a", count: 40) + "@example.com", taken: [long])
        #expect(longTaken.count <= 32 && longTaken.hasSuffix("-2") && TeammateNames.isValidHandle(longTaken))
        for odd in ["", "Bob", "2bob", "bob smith", "owner", String(repeating: "a", count: 33), "bob_smith"] {
            #expect(!TeammateNames.isValidHandle(odd), "\(odd) passed")
        }
        #expect(TeammateNames.isValidHandle("bob-smith-2"))
    }

    @Test("A name is the email's local part, in words")
    func names() {
        #expect(TeammateNames.name(email: "bob.smith@example.com") == "Bob Smith")
        #expect(TeammateNames.name(email: "jean-luc_picard@example.com") == "Jean Luc Picard")
        #expect(TeammateNames.name(email: "42@example.com") == "42@example.com")
        #expect(TeammateNames.name(email: String(repeating: "a", count: 80) + "@x.com").count == 64)
    }
}

/// The fingerprint two people compare in person (ov-525, review F10).
struct KeyFingerprintTests {
    @Test("Sixteen characters of the key, eight from each end")
    func short() {
        let full = "SHA256:iDqoxaySm9gzxtvLrNXXpM5PimPLeBknaaNj0Rg7vz4"
        #expect(KeyFingerprint.short(full) == "SHA256:iDqoxayS…j0Rg7vz4")
        #expect(KeyFingerprint.short("SHA256:short") == "SHA256:short")
        #expect(KeyFingerprint.short("not a fingerprint") == "not a fingerprint")
    }
}
