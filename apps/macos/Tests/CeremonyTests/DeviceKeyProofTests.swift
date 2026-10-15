import AgentKit
import CFarCoolerClient
import Foundation
import Testing

@testable import Far_Cooler

/// This Mac's Key A, proven to the team directory on sign-in (ov-549): what the
/// Mac hands the core, and what it reads back. The statement and its rules are
/// the core's (`crates/client/src/device_keys.rs`); these never touch the real
/// key folder, only a key made here in memory.
@MainActor
struct DeviceKeyProofTests {
    static let challenge = String(repeating: "0f", count: 32)

    /// A fresh key pair, as `farcooler_client_generate_key` makes one.
    static func generated() throws -> (privateKey: String, publicKey: String) {
        var buffer = [UInt8](repeating: 0, count: 4096)
        let written = "farcooler-Test Mac".withCString { comment in
            buffer.withUnsafeMutableBufferPointer { farcooler_client_generate_key(comment, $0.baseAddress, $0.count) }
        }
        let object = try #require(
            try JSONSerialization.jsonObject(with: Data(buffer[0..<written])) as? [String: String])
        return (try #require(object["private_key"]), try #require(object["public_key"]))
    }

    @Test("The key this Mac enrolls proves itself, and only its public half leaves")
    func theKeyProvesItself() throws {
        let key = try Self.generated()
        let proof = try #require(DeviceKey.proof(privateKey: key.privateKey, account: "user_alice", challenge: Self.challenge))
        #expect(proof.keyA == key.publicKey.replacingOccurrences(of: " farcooler-Test Mac", with: ""))
        #expect(proof.proofA.hasPrefix("-----BEGIN SSH SIGNATURE-----"))
        #expect(!proof.proofA.contains("PRIVATE") && !proof.keyA.contains("Test Mac"))
    }

    @Test("A refusal from the core is no proof, so nothing is sent")
    func aRefusalIsNoProof() throws {
        let key = try Self.generated()
        #expect(DeviceKey.proof(privateKey: key.privateKey, account: "user_alice", challenge: "not hex") == nil)
        #expect(DeviceKey.proof(privateKey: key.privateKey, account: "", challenge: Self.challenge) == nil)
        #expect(DeviceKey.proof(privateKey: key.publicKey, account: "user_alice", challenge: Self.challenge) == nil)
    }
}
