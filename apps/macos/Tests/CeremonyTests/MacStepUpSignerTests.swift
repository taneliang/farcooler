import CryptoKit
import Foundation
import Testing

@testable import Far_Cooler

/// B2 of ov-974's second review: the Mac's grant key is made in the app for
/// each signature and kept nowhere, so no file another process writes can
/// stand in a key that signs without the person. Making a key asks nothing;
/// only signing does, so this never prompts. Skipped where the Mac has no
/// Secure Enclave (a CI runner).
struct MacStepUpSignerTests {
    @Test("Each key is new, and nothing is read from or left on disk")
    func freshKeys() throws {
        guard SecureEnclave.isAvailable else { return }
        let support = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
        let before = Set((try? FileManager.default.subpathsOfDirectory(atPath: support.path))?.filter { $0.hasSuffix(".enclave") } ?? [])
        let signer = MacStepUpSigner()
        #expect(signer.publicKey == nil, "no key until one is made: none is loaded")
        let first = try signer.makeKey()
        let second = try signer.makeKey()
        #expect(first != second, "a new key each time")
        #expect(signer.publicKey == second)
        #expect(MacStepUpSigner().publicKey == nil, "another signer sees no key: nothing was kept")
        let after = Set((try? FileManager.default.subpathsOfDirectory(atPath: support.path))?.filter { $0.hasSuffix(".enclave") } ?? [])
        #expect(after == before, "no key file written")
    }

    @Test("The prompt names the runner and the card")
    func reason() {
        let said = MacCutoverGrant.reason(card: "ov-974")
        #expect(said.contains("this Mac") && said.contains("ov-974"), "\(said)")
    }
}
