import AgentKit
import CFarCoolerClient
import Foundation
import Testing

@testable import Far_Cooler

/// This Mac adding another of its own account's devices vouches for it in the
/// directory (ov-550): once the person confirms, it certifies the Key A from
/// the code it scanned. A teammate's device, a refused tap and a ceremony that
/// went no further certify nothing.
@MainActor
struct DeviceCertifyCeremonyTests {
    static let row = CeremonyStore.RunnerRow(
        runner: CeremonyRunner(
            id: "alice@build-box", label: "build-box", alias: "build-box", user: "alice",
            host_key: "SHA256:iDqoxaySm9gzxtvLrNXXpM5PimPLeBknaaNj0Rg7vz4",
            reach: .direct(host: "build-box.example.com", port: 22), pending: true),
        granted: true, note: nil)

    final class Certified: @unchecked Sendable {
        var calls: [(account: String, keyA: String)] = []
    }

    /// A store that has scanned `account`'s code for an iPhone with `keyA`,
    /// signed in here as `signedIn`, recording every certification.
    private func scanned(
        account: String = "user_alice", signedIn: String = "user_alice", keyA: String, directory: Data? = nil,
        authenticated: Bool = true
    ) async throws -> (CeremonyStore, Certified) {
        let code = try #require(CeremonyFFI.offer(name: "Alice’s iPhone", account: account, keyA: keyA, keyB: nil))
        let store = CeremonyStore()
        let certified = Certified()
        store.authenticate = { _ in authenticated }
        store.teamDirectory = { directory }
        store.certify = { certified.calls.append(($0, $1)) }
        store.scan(code, at: Date(), account: signedIn, email: "alice@example.com", runners: [Self.row])
        await store.teamCheck?.value
        return (store, certified)
    }

    private func confirm(_ store: CeremonyStore) async {
        await store.confirm(reason: "test") { runners in Enrollment.Outcome(written: Set(runners.map(\.id))) }
        await store.certification?.value
    }

    @Test func confirmingYourOwnNewDeviceCertifiesTheKeyFromItsCode() async throws {
        let phone = try DeviceKeyProofTests.generated()
        let (store, certified) = try await scanned(keyA: phone.publicKey)
        #expect(certified.calls.isEmpty, "nothing is vouched for before the tap")
        await confirm(store)
        guard case .showingManifest = store.phase else {
            Issue.record("no reply: \(store.phase)")
            return
        }
        #expect(certified.calls.map(\.account) == ["user_alice"])
        #expect(certified.calls.map(\.keyA) == [phone.publicKey])
    }

    @Test func aRefusedTapCertifiesNothing() async throws {
        let phone = try DeviceKeyProofTests.generated()
        let (store, certified) = try await scanned(keyA: phone.publicKey, authenticated: false)
        await confirm(store)
        #expect(certified.calls.isEmpty)
    }

    @Test func aTeammatesDeviceIsNeverThisMacsToVouchFor() async throws {
        let phone = try DeviceKeyProofTests.generated()
        let (store, certified) = try await scanned(
            account: "user_bob", keyA: phone.publicKey, directory: TeammateCeremonyTests.directory())
        store.setTeammate { $0.handle = "bob" }
        await confirm(store)
        guard case .showingManifest = store.phase else {
            Issue.record("the teammate's reply wasn't built: \(store.phase)")
            return
        }
        #expect(certified.calls.isEmpty)
    }

    @Test("The certificate is the core's, by this Mac's key, for the scanned key alone")
    func theCertificateIsTheCores() throws {
        let mac = try DeviceKeyProofTests.generated()
        let phone = try DeviceKeyProofTests.generated()
        let made = try #require(
            DeviceKey.certificate(privateKey: mac.privateKey, account: "user_alice", scannedKeyA: phone.publicKey))
        #expect(made.hasPrefix("-----BEGIN SSH SIGNATURE-----") && !made.contains("PRIVATE"))
        // This Mac's own key in a code is not another device.
        #expect(DeviceKey.certificate(privateKey: mac.privateKey, account: "user_alice", scannedKeyA: mac.publicKey) == nil)
        #expect(DeviceKey.certificate(privateKey: mac.publicKey, account: "user_alice", scannedKeyA: phone.publicKey) == nil)
        #expect(DeviceKey.certificate(privateKey: mac.privateKey, account: "", scannedKeyA: phone.publicKey) == nil)
    }
}
