import Foundation
import Testing

@testable import AgentKit

/// A device trusted by a reset waits out a hold before team admins can add
/// it (ruling R-76, ov-565): what the apps say, and the admin's confirm.
@MainActor
struct DeviceHoldTests {
    static let t0 = Date(timeIntervalSince1970: 1_790_000_000)
    static let until = t0.addingTimeInterval(72 * 3600)

    static func row(state: String, reset: Bool = false, heldUntil: Date? = until) -> DeviceKeyRow {
        DeviceKeyRow(
            id: "d-2", label: "Bob’s iPhone", keyA: "ssh-ed25519 AAAAphone", state: state,
            fingerprint: "SHA256:AAAAphone0123456789abcdefghij", createdAt: t0, trustedAt: t0, trustReset: reset,
            heldUntil: heldUntil)
    }

    @Test("A held row says when team admins can add it, and stops saying so once the hold ends")
    func rowSaysItIsHeld() {
        let held = Self.row(state: "trusted", reset: true)
        let sentence = DeviceKeyRow.holdSentence(until: Self.until)
        #expect(
            held.detail(isThisDevice: false, now: Self.t0)
                == "Confirmed when it signed in, since none of your devices was confirmed. \(sentence)")
        #expect(sentence.hasPrefix("Your team admins can’t add it to runners until "))
        #expect(sentence.hasSuffix(", unless one confirms it with you in person."))
        #expect(held.detail(isThisDevice: true, now: Self.t0).hasPrefix("This device. Confirmed when it signed in"))
        #expect(!held.detail(isThisDevice: false, now: Self.until).contains("team admins"))
        // A device confirmed by a held one is held too, and says so.
        #expect(Self.row(state: "certified").detail(isThisDevice: false, now: Self.t0).hasSuffix(sentence))
        // An unconfirmed row has nothing to wait out.
        #expect(!Self.row(state: "pending").detail(isThisDevice: false, now: Self.t0).contains("team admins"))
        #expect(!Self.row(state: "trusted", heldUntil: nil).detail(isThisDevice: false, now: Self.t0).contains("team admins"))
    }

    @Test("The banner's reset line says the device is held, while it is")
    func bannerSaysItIsHeld() {
        let event = DeviceEvent(kind: .trusted, row: Self.row(state: "trusted", reset: true), at: Self.t0)
        let sentence = DeviceKeyRow.holdSentence(until: Self.until)
        #expect(DeviceNotice.detail([event], now: Self.t0).contains("was confirmed. \(sentence) If you don’t recognize"))
        #expect(!DeviceNotice.detail([event], now: Self.until).contains("team admins"))
    }

    @Test("`me`'s heldUntil is read")
    func readsHeldUntil() async throws {
        let client = DirectoryClient { _, _ in
            .success(
                DirectoryReply(
                    status: 200,
                    body: [
                        "user": ["id": "user_bob"], "teams": [],
                        "devices": [
                            [
                                "id": "d-2", "label": "Phone", "keyA": "ssh-ed25519 AAAAphone", "state": "trusted",
                                "heldUntil": 1_790_259_200_000,
                            ],
                            ["id": "d-3", "label": "Mac", "keyA": "ssh-ed25519 AAAAmac", "state": "trusted", "heldUntil": NSNull()],
                        ],
                    ]))
        }
        let rows = try await client.deviceKeys().get()
        #expect(rows[0].heldUntil == Self.until)
        #expect(rows[1].heldUntil == nil)
    }

    /// The relay, as a script: what it was asked, and what it answers.
    @MainActor final class Relay {
        var asked: [(path: String, body: [String: Any])] = []
        var answer = DirectoryReply(status: 200, body: ["ok": true])
        var client: DirectoryClient {
            DirectoryClient { path, body in
                self.asked.append((path, body))
                return .success(self.answer)
            }
        }
    }

    @Test("Confirming posts the team, the person and the scanned key, and a refusal is a failure")
    func confirmPostsTheScannedKey() async {
        let relay = Relay()
        let confirmed = await relay.client.confirmHeldDevice(team: "t-1", user: "user_bob", keyA: "ssh-ed25519 AAAA")
        #expect((try? confirmed.get()) != nil)
        #expect(relay.asked.map(\.path) == ["/v1/directory/teams/devices/confirm"])
        #expect(relay.asked[0].body["team"] as? String == "t-1")
        #expect(relay.asked[0].body["user"] as? String == "user_bob")
        #expect(relay.asked[0].body["keyA"] as? String == "ssh-ed25519 AAAA")

        relay.answer = DirectoryReply(status: 403, body: ["error": "forbidden"])
        guard case .failure(let why) = await relay.client.confirmHeldDevice(team: "t-1", user: "user_bob", keyA: "k")
        else {
            Issue.record("a 403 confirmed something")
            return
        }
        #expect(why == .roleCannotAdd)
        relay.answer = DirectoryReply(status: 200, body: [:])
        if case .success = await relay.client.confirmHeldDevice(team: "t-1", user: "user_bob", keyA: "k") {
            Issue.record("a 200 that doesn't say ok confirmed nothing")
        }
    }
}

/// A team admin lifting a hold is news on the banner, read from `me`
/// (ov-565 fix round 1, M1).
@MainActor
struct HoldConfirmationNoticeTests {
    static let t0 = DeviceHoldTests.t0

    static let row = DeviceKeyRow(
        id: "d-2", label: "Bob’s iPhone", keyA: "ssh-ed25519 AAAAphone", state: "trusted",
        fingerprint: "SHA256:AAAAphone0123456789abcdefghij", createdAt: t0, trustedAt: t0, trustReset: true,
        holdConfirmations: [HoldConfirmation(team: "Acme\u{202E}", at: t0.addingTimeInterval(3600))])

    @Test("A lifted hold is announced with the team's name, after it happened, even for this device")
    func liftedHoldIsAnnounced() throws {
        let events = DeviceNotice.unseen([Self.row], since: Self.t0, ownKeyA: "ssh-ed25519 AAAAphone x")
        #expect(events.map(\.kind) == [.confirmed])
        #expect(try #require(events.first).team == "Acme\u{202E}")
        #expect(DeviceNotice.title(events) == "A team admin confirmed one of your devices")
        let detail = DeviceNotice.detail(events)
        #expect(detail.hasPrefix("An admin of “Acme” confirmed “Bob’s iPhone” ("))
        #expect(detail.contains("so that team’s runners can add it now."))
        #expect(DeviceNotice.unseen([Self.row], since: Self.t0.addingTimeInterval(3600), ownKeyA: nil).isEmpty)
    }

    @Test("`me`'s holdConfirmations are read")
    func readsConfirmations() async throws {
        let client = DirectoryClient { _, _ in
            .success(
                DirectoryReply(
                    status: 200,
                    body: [
                        "user": ["id": "user_bob"], "teams": [],
                        "devices": [
                            [
                                "id": "d-2", "label": "Phone", "keyA": "ssh-ed25519 AAAAphone", "state": "trusted",
                                "holdConfirmations": [["team": "Acme", "at": 1_790_003_600_000]],
                            ]
                        ],
                    ]))
        }
        let rows = try await client.deviceKeys().get()
        #expect(rows[0].holdConfirmations == [HoldConfirmation(team: "Acme", at: Self.t0.addingTimeInterval(3600))])
    }
}
