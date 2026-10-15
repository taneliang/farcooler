import Foundation
import Testing

@testable import Far_Cooler

/// This Mac asks before joining a teammate's runners (ov-543), as the phone does
/// (ov-525): a reply that says another account granted it is held until the
/// person says Join, and Cancel adds nothing. A reply from your own account
/// joins without asking.
@MainActor
struct TeammateJoinPromptTests {
    static let keyA = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIB1iLbeqDzK4CDeUC3t+ffVPDI9Gk+sBwIZqJZW1NfS5 device-a"

    static func runner(_ label: String) -> CeremonyRunner {
        CeremonyRunner(
            id: "alice@\(label)", label: label, alias: label, user: "alice",
            host_key: "SHA256:iDqoxaySm9gzxtvLrNXXpM5PimPLeBknaaNj0Rg7vz4",
            reach: .direct(host: "\(label).example.com", port: 22), pending: false)
    }

    /// This Mac showing a code, and the reply to it, from `grantedBy`.
    private func replied(grantedBy: String?, labels: [String] = ["build-box"]) throws -> (CeremonyStore, String) {
        let store = CeremonyStore()
        store.present(name: "Bob’s Mac", account: "user_bob", keyA: Self.keyA, keyB: nil)
        guard case .showingOffer(let offer) = store.phase else {
            Issue.record("no code: \(store.phase)")
            throw CancellationError()
        }
        let reply = try #require(
            CeremonyFFI.reply(
                offer: offer, runners: labels.map(Self.runner), budget: codeBudgetBytes, grantedBy: grantedBy))
        return (store, reply)
    }

    @Test func aTeammatesReplyIsHeldUntilThePersonSaysJoin() throws {
        let (store, reply) = try replied(grantedBy: "alice@example.com")
        #expect(store.accept(reply, at: Date()) == nil, "a teammate's runners were handed over unasked")
        guard case .confirmingJoin(let held) = store.phase else {
            Issue.record("not asking: \(store.phase)")
            return
        }
        #expect(held.grantor == "alice@example.com")
        #expect(store.join()?.runners.map(\.label) == ["build-box"])
        // Said yes once; there is nothing left to say yes to.
        #expect(store.join() == nil)
    }

    @Test func cancellingAddsNothing() throws {
        let (store, reply) = try replied(grantedBy: "alice@example.com")
        #expect(store.accept(reply, at: Date()) == nil)
        store.cancelJoin()
        #expect(store.join() == nil, "Cancel left the runners to be joined")
        if case .confirmingJoin = store.phase { Issue.record("still asking after Cancel") }
        // The reply was consumed, so the same one cannot be taken again.
        #expect(store.accept(reply, at: Date()) == nil)
        if case .confirmingJoin = store.phase { Issue.record("asked again for a reply already taken") }
    }

    @Test func yourOwnDevicesReplyJoinsWithoutAsking() throws {
        let (store, reply) = try replied(grantedBy: nil)
        #expect(store.accept(reply, at: Date())?.runners.map(\.label) == ["build-box"])
        if case .confirmingJoin = store.phase { Issue.record("asked about your own runners") }
    }

    @Test func aBlankGrantorIsNotAnotherAccount() {
        var manifest = CeremonyManifest(
            v: 1, ceremony: "c", account: "a", channel: "ch", target: "t", runners: [])
        manifest.granted_by = "  "
        #expect(manifest.grantor == nil)
        manifest.granted_by = " alice@example.com "
        #expect(manifest.grantor == "alice@example.com")
    }

    @Test func theWordsAreThePhonesAndSayWhatTheCodeSays() {
        #expect(
            TeammateJoinCopy.title(runners: [Self.runner("build-box")]) == "Join Your Teammate’s Runner “build-box”?")
        #expect(
            TeammateJoinCopy.title(runners: [Self.runner("a"), Self.runner("b")]) == "Join Your Teammate’s Runners?")
        #expect(TeammateJoinCopy.detail(from: "alice@example.com").hasPrefix("This code says alice@example.com added"))
    }

    @Test func aLongLabelIsCutInTheMiddleOnOneLine() {
        let long = "Platform Builds, EU West (Shared) and Staging Runner 7"
        let cut = TeammateJoinCopy.shortened(long)
        #expect(cut.count == TeammateJoinCopy.labelLimit)
        #expect(cut.hasPrefix("Platform") && cut.hasSuffix("Runner 7") && cut.contains("…"))
        #expect(TeammateJoinCopy.shortened("build-box") == "build-box")
        #expect(TeammateJoinCopy.title(runners: [Self.runner(long)]).contains(cut))
    }
}
