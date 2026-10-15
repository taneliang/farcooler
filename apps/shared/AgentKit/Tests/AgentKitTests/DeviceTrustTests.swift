import Foundation
import Testing

@testable import AgentKit

/// Starting over with no confirmed device (ruling R-75, ov-561), and labels
/// that can't masquerade (ov-560 R1).
@MainActor
struct DeviceTrustTests {
    static let day: TimeInterval = 86_400
    static let t0 = Date(timeIntervalSince1970: 1_790_000_000)

    static func row(
        _ id: String, _ label: String, key: String, state: String, added: TimeInterval, trusted: TimeInterval? = nil,
        reset: Bool = false
    ) -> DeviceKeyRow {
        DeviceKeyRow(
            id: id, label: label, keyA: "ssh-ed25519 \(key)", state: state,
            fingerprint: "SHA256:\(key)0123456789abcdefghij", createdAt: t0.addingTimeInterval(added),
            trustedAt: trusted.map { t0.addingTimeInterval($0) }, trustReset: reset)
    }

    /// The account's first device: trusted the moment it arrived.
    static let mac = row("d-1", "MacBook Air", key: "AAAAmac", state: "revoked", added: 0, trusted: 0)
    /// A device that waited, then was trusted when nothing else was confirmed.
    static let phone = row("d-2", "Bob’s iPhone", key: "AAAAphone", state: "trusted", added: day, trusted: 3 * day, reset: true)

    @Test("A device trusted again with none confirmed is news on the banner, after its arrival")
    func trustedAgainIsAnnounced() {
        let events = DeviceNotice.unseen([Self.mac, Self.phone], since: Self.t0.addingTimeInterval(2 * Self.day), ownKeyA: nil)
        #expect(events.map(\.id) == ["trusted-d-2"])
        #expect(events[0].at == Self.t0.addingTimeInterval(3 * Self.day))
        #expect(DeviceNotice.title(events) == "A device is now confirmed on your account")
        let detail = DeviceNotice.detail(events)
        #expect(detail.hasPrefix("Confirmed “Bob’s iPhone” (\(KeyFingerprint.short(Self.phone.fingerprint))) on "))
        #expect(detail.contains("since none of your devices was confirmed."))
    }

    @Test("A first device's trust is its arrival, not a second event; and this device's own isn't news")
    func firstUseIsNotTwoEvents() {
        let first = Self.row("d-3", "Mac", key: "AAAAfirst", state: "trusted", added: 0, trusted: 0)
        #expect(DeviceNotice.unseen([first], since: nil, ownKeyA: nil).map(\.id) == ["added-d-3"])
        #expect(DeviceNotice.unseen([Self.phone], since: nil, ownKeyA: "ssh-ed25519 AAAAphone x").isEmpty)
        // Dismissed after it: not news again.
        #expect(DeviceNotice.unseen([Self.phone], since: Self.t0.addingTimeInterval(3 * Self.day), ownKeyA: nil).isEmpty)
    }

    @Test("A row says how it was confirmed, and a stranded one says to sign in on the device to keep")
    func rowCopy() {
        #expect(Self.phone.detail(isThisDevice: false) == "Confirmed when it signed in, since none of your devices was confirmed")
        let first = Self.row("d-3", "Mac", key: "AAAAfirst", state: "trusted", added: 0, trusted: 0)
        #expect(first.detail(isThisDevice: false) == "Confirmed as the first device on your account")
        let pending = Self.row("d-4", "iPad", key: "AAAApad", state: "pending", added: 0)
        let stranded = pending.detail(isThisDevice: true, anyConfirmed: false)
        #expect(stranded.hasPrefix("This device. Not confirmed. None of your devices is confirmed"))
        #expect(stranded.contains("Do that on the device you want to keep"))
        #expect(!stranded.contains("Far Cooler team"))
        #expect(DeviceKeyRow.removalWarning(for: Self.phone, among: [Self.phone, pending]).contains("becomes confirmed in its place"))
    }

    /// Fix round 1, M1: a new device trusted by the reset arrives trusted, so
    /// its trust and its arrival are one moment, and it's no first device.
    @Test("A new device trusted by the reset is announced as confirmed, never added or first")
    func newDeviceResetIsATrustEvent() {
        let laptop = Self.row("d-5", "Laptop", key: "AAAAlap", state: "trusted", added: 2 * Self.day, trusted: 2 * Self.day, reset: true)
        let events = DeviceNotice.unseen([Self.mac, laptop], since: Self.t0.addingTimeInterval(Self.day), ownKeyA: nil)
        #expect(events.map(\.id) == ["trusted-d-5"])
        #expect(DeviceNotice.title(events) == "A device is now confirmed on your account")
        #expect(laptop.detail(isThisDevice: false) == "Confirmed when it signed in, since none of your devices was confirmed")
    }

    /// Fix round 1, Low 4: removing the last confirmed device from this one
    /// makes this one the confirmed device at once, if it's pending.
    @Test("Removing the last confirmed device starts over here, and only then")
    func removalStartsOverHere() {
        let own = "ssh-ed25519 AAAApad x"
        let pending = Self.row("d-4", "iPad", key: "AAAApad", state: "pending", added: 0)
        let other = Self.row("d-6", "Pixel", key: "AAAApix", state: "pending", added: 0)
        #expect(DeviceKeyRow.startsOverHere([pending, other], ownKeyA: own))
        #expect(!DeviceKeyRow.startsOverHere([pending, Self.phone], ownKeyA: own), "another is still confirmed")
        #expect(!DeviceKeyRow.startsOverHere([other], ownKeyA: own), "this device isn't listed")
        #expect(!DeviceKeyRow.startsOverHere([pending], ownKeyA: nil))
    }

    @Test("`me`'s trustedAt and trustReset are read")
    func readsTrustedAt() async throws {
        let client = DirectoryClient { _, _ in
            .success(
                DirectoryReply(
                    status: 200,
                    body: [
                        "user": ["id": "user_bob"], "teams": [],
                        "devices": [
                            [
                                "id": "d-2", "label": "Phone", "keyA": "ssh-ed25519 AAAAphone", "state": "trusted",
                                "createdAt": 1_790_000_000_000, "trustedAt": 1_790_086_400_000, "trustReset": true,
                            ]
                        ],
                    ]))
        }
        let rows = try await client.deviceKeys().get()
        #expect(rows[0].trustedAt == Self.t0.addingTimeInterval(Self.day))
        #expect(rows[0].wasTrustedAgain)
    }

    @Test("Signing in registers again, even when this launch already did, so it can be the device kept")
    func signInRegistersAgain() async {
        let relay = DeviceKeyRegistrationTests.Relay()
        let signer = DeviceKeyRegistrationTests.Signer()
        let registration = DeviceKeyRegistrationTests().registration(signer)
        await registration.registerIfNeeded(account: "user_bob", directory: relay.client)
        #expect(await registration.registerAgain(account: "user_bob", directory: relay.client) == .success(.trusted))
        #expect(signer.asked.map(\.account) == ["user_bob", "user_bob"])
    }

    /// One table for every platform: the relay, the web and Android read it
    /// too (`test/fixtures/device-labels.json`).
    @Test("Every label in the shared table is shown without what could masquerade")
    func sharedLabelTable() throws {
        struct Table: Decodable {
            struct Case: Decodable { let why: String; let label: String; let shown: String }
            let cases: [Case]
        }
        var root = URL(fileURLWithPath: #filePath)
        // …/apps/shared/AgentKit/Tests/AgentKitTests/<this file>
        for _ in 0..<6 { root.deleteLastPathComponent() }
        let data = try Data(contentsOf: root.appendingPathComponent("test/fixtures/device-labels.json"))
        let table = try JSONDecoder().decode(Table.self, from: data)
        #expect(table.cases.count >= 10)
        for c in table.cases {
            let row = DeviceKeyRow(id: "d", label: c.label, keyA: "ssh-ed25519 AAAA", state: "trusted")
            #expect(row.shownLabel == c.shown, "\(c.why)")
        }
    }
}
