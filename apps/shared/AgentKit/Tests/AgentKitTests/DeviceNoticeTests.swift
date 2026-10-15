import Foundation
import Testing

@testable import AgentKit

/// The in-app notice of devices added to or removed from the account (ov-550
/// fix round 1, review M1), and what tells two devices apart (M2, L4, L1).
@MainActor
struct DeviceNoticeTests {
    static let day: TimeInterval = 86_400
    static let t0 = Date(timeIntervalSince1970: 1_790_000_000)

    static func row(
        _ id: String, _ label: String, key: String, state: String = "trusted", added: TimeInterval,
        removed: TimeInterval? = nil
    ) -> DeviceKeyRow {
        DeviceKeyRow(
            id: id, label: label, keyA: "ssh-ed25519 \(key)", state: removed == nil ? state : "revoked",
            fingerprint: "SHA256:\(key)0123456789abcdefghij",
            createdAt: t0.addingTimeInterval(added), revokedAt: removed.map { t0.addingTimeInterval($0) })
    }

    static let mac = row("d-1", "MacBook Air", key: "AAAAmac", added: 0)
    static let impostor = row("d-2", "Bob’s iPad", key: "AAAAbad", state: "pending", added: day)
    static let old = row("d-3", "Old Phone", key: "AAAAold", added: 0, removed: 2 * day)

    func notice(_ rows: [DeviceKeyRow], own: String? = "ssh-ed25519 AAAAmac farcooler-Mac") -> (DeviceNotice, UserDefaults) {
        let defaults = ScratchDefaults.suite("DeviceNoticeTests-\(UUID().uuidString)")!
        let notice = DeviceNotice()
        notice.defaults = defaults
        notice.load = { ("user_bob", rows) }
        notice.ownKeyA = { _ in own }
        return (notice, defaults)
    }

    @Test("An app that has told nothing yet tells every device added or removed, but its own arrival")
    func firstRunTellsEverythingButItself() async {
        let (notice, _) = notice([Self.mac, Self.impostor, Self.old])
        await notice.refresh()
        #expect(notice.events.map(\.id) == ["added-d-3", "added-d-2", "removed-d-3"])
    }

    @Test("Once dismissed, it stays dismissed in this app, until something newer happens")
    func dismissedUntilSomethingNewer() async {
        var rows = [Self.mac, Self.impostor]
        let (notice, defaults) = notice([])
        notice.load = { ("user_bob", rows) }
        await notice.refresh()
        #expect(notice.events.map(\.id) == ["added-d-2"])
        notice.dismiss()
        #expect(notice.events.isEmpty)
        await notice.refresh()
        #expect(notice.events.isEmpty, "a dismissed device came back")
        #expect(defaults.object(forKey: DeviceNotice.key("user_bob")) != nil)

        // Removed later: that's news again.
        rows[1] = Self.row("d-2", "Bob’s iPad", key: "AAAAbad", added: Self.day, removed: 3 * Self.day)
        await notice.refresh()
        #expect(notice.events.map(\.id) == ["removed-d-2"])
    }

    @Test("Signed out, or the relay unreachable, shows nothing and forgets nothing")
    func nothingWhenNothingCanBeAsked() async {
        let (notice, defaults) = notice([Self.impostor])
        await notice.refresh()
        notice.load = { nil }
        await notice.refresh()
        #expect(notice.events.isEmpty)
        #expect(defaults.object(forKey: DeviceNotice.key("user_bob")) == nil)
    }

    @Test("The banner names each device by a label that can't run on, its key words and its day (M2)")
    func theBannerSaysWhichDevice() {
        let sneaky = Self.row(
            "d-9", "MacBook Air” is expected. No action is needed. “ok", key: "AAAAx", state: "pending", added: Self.day)
        let events = DeviceNotice.unseen([sneaky], since: nil, ownKeyA: nil)
        #expect(DeviceNotice.title(events) == "A device was added to your account")
        let detail = DeviceNotice.detail(events)
        #expect(detail.filter { $0 == "“" || $0 == "”" }.count == 2)
        #expect(detail.contains("(\(KeyFingerprint.short(sneaky.fingerprint)))"))
        #expect(detail.hasSuffix("If you don’t recognize a change, review Device Keys in Settings."))
        #expect(DeviceNotice.title(DeviceNotice.unseen([Self.old], since: nil, ownKeyA: nil)) == "Your account’s devices changed")
    }

    @Test("Each row shows its key words and the day it was added")
    func rowsCanBeToldApart() {
        let twin = Self.row("d-4", "MacBook Air", key: "AAAAtwin", added: Self.day)
        #expect(Self.mac.identity != twin.identity)
        #expect(Self.mac.identity.hasPrefix(KeyFingerprint.short(Self.mac.fingerprint)))
        #expect(Self.mac.identity.contains("Added "))
    }

    @Test("With no confirmed device left, a pending row doesn't send the person to one (L4)")
    func noConfirmedDeviceLeft() {
        let pending = Self.row("d-5", "iPhone", key: "AAAAp", state: "pending", added: 0)
        #expect(pending.detail(isThisDevice: false, anyConfirmed: true).contains("Scan its code"))
        let stranded = pending.detail(isThisDevice: false, anyConfirmed: false)
        #expect(!stranded.contains("Scan its code"))
        #expect(stranded.contains("None of your devices is confirmed"))
    }

    @Test("Removing the last confirmed device says what it leaves behind (M2)")
    func lastConfirmedDeviceIsWarned() {
        let rows = [Self.mac, Self.impostor]
        #expect(DeviceKeyRow.removalWarning(for: Self.mac, among: rows).hasPrefix("This is the only device"))
        #expect(!DeviceKeyRow.removalWarning(for: Self.impostor, among: rows).hasPrefix("This is the only device"))
        let two = [Self.mac, Self.row("d-6", "iPhone", key: "AAAAi", state: "certified", added: 0)]
        #expect(!DeviceKeyRow.removalWarning(for: Self.mac, among: two).hasPrefix("This is the only device"))
    }

    @Test("`me`'s dates and fingerprints are read, and removed rows only when asked for")
    func readsDatesFromMe() async throws {
        let client = DirectoryClient { _, _ in
            .success(
                DirectoryReply(
                    status: 200,
                    body: [
                        "user": ["id": "user_bob"], "teams": [],
                        "devices": [
                            [
                                "id": "d-1", "label": "Mac", "keyA": "ssh-ed25519 AAAAmac", "state": "trusted",
                                "keyAFingerprint": "SHA256:abc", "createdAt": 1_790_000_000_000,
                            ],
                            [
                                "id": "d-2", "label": "Old", "keyA": "ssh-ed25519 AAAAold", "state": "revoked",
                                "createdAt": 1_790_000_000_000, "revokedAt": 1_790_086_400_000,
                            ],
                        ],
                    ]))
        }
        let all = try await client.deviceKeys(includingRemoved: true).get()
        #expect(all.map(\.id) == ["d-1", "d-2"])
        #expect(all[0].fingerprint == "SHA256:abc")
        #expect(all[0].createdAt == Self.t0)
        #expect(all[1].revokedAt == Self.t0.addingTimeInterval(Self.day))
        #expect(try await client.deviceKeys().get().map(\.id) == ["d-1"])
    }

    @Test("Sign-out names this install's push token, so its row goes (L1)")
    func signOutNamesThePushToken() {
        let body = Account.logoutBody(refresh: "rt", pulse: nil, push: "abc123")
        #expect(body?["pushToken"] as? String == "abc123")
        #expect(Account.logoutBody(refresh: nil, pulse: nil, push: "abc123")?["pushToken"] as? String == "abc123")
        #expect(Account.logoutBody(refresh: "rt", pulse: nil, push: "")?["pushToken"] == nil)
    }
}
