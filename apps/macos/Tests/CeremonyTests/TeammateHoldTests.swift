import AgentKit
import Foundation
import Testing

@testable import Far_Cooler

/// A teammate's device that waits out the hold after their account's trust
/// reset (ruling R-76, ov-565): the sheet says so, and the admin's tap
/// confirms it with each team that holds it before any key is written.
@MainActor
struct TeammateHoldTests {
    static let keyA = TeammateCeremonyTests.keyA
    static let until: Double = 1_790_259_200_000

    /// Platform's roster, telling Bob's keys: `keyA` listed, or held.
    static func directory(held: Bool) throws -> Data {
        let fingerprint = try #require(RunnerFacts.fingerprint(ofOpenSSHKey: keyA))
        let bob: [String: Any] = [
            "user": "user_bob", "email": "bob.smith@example.com", "role": "member",
            "keys": held ? [] : [fingerprint],
            "held": held ? [["key": fingerprint, "until": until]] : [],
        ]
        let object: [String: Any] = [
            "me": [
                "user": ["id": "user_alice", "email": "alice@example.com"],
                "teams": [["id": "t-platform", "name": "Platform", "personal": false, "role": "admin"]],
            ],
            "rosters": [
                [
                    "team": ["id": "t-platform", "name": "Platform", "personal": false],
                    "members": [["user": "user_alice", "email": "alice@example.com", "role": "admin"], bob],
                ]
            ],
        ]
        return try JSONSerialization.data(withJSONObject: object)
    }

    final class Calls: @unchecked Sendable {
        var confirmed: [[String]] = []
        var writes = 0
    }

    /// Bob's code scanned on Alice's Mac, and the tap, with the relay
    /// answering a confirmation `confirms`.
    private func added(held: Bool, confirms: Bool) async throws -> (CeremonyStore, Calls) {
        let code = try #require(CeremonyFFI.offer(name: "Bob’s iPhone", account: "user_bob", keyA: Self.keyA, keyB: nil))
        let directory = try Self.directory(held: held)
        let calls = Calls()
        let store = CeremonyStore()
        store.authenticate = { _ in true }
        store.teamDirectory = { directory }
        store.confirmHold = { team, user, keyA in
            calls.confirmed.append([team, user, keyA])
            return confirms
        }
        store.scan(code, at: Date(), account: "user_alice", email: "alice@example.com", runners: [TeammateCeremonyTests.row])
        await store.teamCheck?.value
        guard case .confirming(let confirmation) = store.phase, let teammate = confirmation.teammate else {
            Issue.record("not confirming a teammate: \(store.phase)")
            return (store, calls)
        }
        #expect(teammate.held == (held ? [CeremonyHold(team: "t-platform", until: Self.until)] : []))
        store.setGrant(true, for: TeammateCeremonyTests.row.id)
        await store.confirm(reason: "test") { runners in
            calls.writes += 1
            return Enrollment.Outcome(written: Set(runners.map(\.id)))
        }
        return (store, calls)
    }

    @Test func aHeldDeviceIsConfirmedWithItsTeamBeforeAnyKeyIsWritten() async throws {
        let (store, calls) = try await added(held: true, confirms: true)
        #expect(calls.confirmed == [["t-platform", "user_bob", Self.keyA]])
        #expect(calls.writes == 1)
        guard case .showingManifest = store.phase else {
            Issue.record("no reply: \(store.phase)")
            return
        }
    }

    @Test func aHoldTheRelayDidNotRecordWritesNothing() async throws {
        let (store, calls) = try await added(held: true, confirms: false)
        #expect(calls.confirmed.count == 1)
        #expect(calls.writes == 0)
        #expect(store.phase == .refused(.holdNotConfirmed))
    }

    @Test func aListedDeviceIsNeverConfirmed() async throws {
        let (_, calls) = try await added(held: false, confirms: false)
        #expect(calls.confirmed.isEmpty)
        #expect(calls.writes == 1)
    }

    /// An answer whose holds can't be read is a hold that can't be confirmed,
    /// never "no holds" (fix round 1, Low 2); and a `held_key` refusal has
    /// its own words (Low 1).
    @Test func unreadableHoldsAreNotNone() {
        #expect(CeremonyStore.holds(in: #"{"offer":"x","held":[]}"#) == [])
        #expect(CeremonyStore.holds(in: #"{"offer":"x"}"#) == [])
        #expect(CeremonyStore.holds(in: #"{"held":[{"team":"t","until":5}]}"#) == [CeremonyHold(team: "t", until: 5)])
        #expect(CeremonyStore.holds(in: #"{"held":5}"#) == nil)
        #expect(CeremonyStore.holds(in: #"{"held":[{"team":1}]}"#) == nil)
        #expect(CeremonyStore.holds(in: "not json") == nil)
        #expect(Refusal.from(#"{"error":"held_key"}"#) == .heldKey)
    }

    /// What the sheet says: why it's held, and what adding it vouches for,
    /// in the admin's words, never the scanned device as "this".
    @Test func theSheetSaysWhyItIsHeld() {
        let hold = CeremonyHold(team: "t-platform", until: Self.until)
        let sentence = CeremonyHold.sentence([hold], email: "bob@example.com")
        #expect(sentence.hasPrefix("Their device is waiting out a hold until "))
        #expect(!sentence.contains("This device") && !sentence.contains("this device"))
        #expect(sentence.contains(hold.ends.formatted(date: .abbreviated, time: .shortened)))
        #expect(sentence.contains("Add it only if bob@example.com is with you"))
        #expect(sentence.hasSuffix("adding it confirms the device for your team now."))
        let refusal = Refusal.holdNotConfirmed
        #expect(refusal.detail.contains("nothing was added"))
    }
}
