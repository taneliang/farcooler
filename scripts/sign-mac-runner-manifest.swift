// Sign the manifest in OpenSSH's SSHSIG format using the existing canary seed.
// The namespace keeps this signature separate from Sparkle's raw Ed25519 signatures.
import CryptoKit
import Foundation

func field(_ data: Data) -> Data {
    var length = UInt32(data.count).bigEndian
    return withUnsafeBytes(of: &length) { Data($0) } + data
}

enum SigningError: Error {
    case invalidKey
}

func sign() throws {
    let seedText = String(decoding: FileHandle.standardInput.readDataToEndOfFile(), as: UTF8.self)
        .trimmingCharacters(in: .whitespacesAndNewlines)
    guard CommandLine.arguments.count == 3, let seed = Data(base64Encoded: seedText),
          seed.count == 32 else {
        throw SigningError.invalidKey
    }
    let key = try Curve25519.Signing.PrivateKey(rawRepresentation: seed)
    guard key.publicKey.rawRepresentation.base64EncodedString() == CommandLine.arguments[2] else {
        throw SigningError.invalidKey
    }
    let manifest = try Data(contentsOf: URL(fileURLWithPath: CommandLine.arguments[1]))
    let magic = Data("SSHSIG".utf8)
    let namespace = field(Data("runner@farcooler.com".utf8))
    let reserved = field(Data())
    let hash = field(Data("sha512".utf8))
    let algorithm = field(Data("ssh-ed25519".utf8))
    let signed = magic + namespace + reserved + hash + field(Data(SHA512.hash(data: manifest)))
    let signature = try key.signature(for: signed)
    let envelope = magic + Data([0, 0, 0, 1])
        + field(algorithm + field(key.publicKey.rawRepresentation))
        + namespace + reserved + hash + field(algorithm + field(signature))

    FileHandle.standardOutput.write(manifest)
    print("-----BEGIN SSH SIGNATURE-----")
    print(envelope.base64EncodedString(options: [.lineLength64Characters, .endLineWithLineFeed]))
    print("-----END SSH SIGNATURE-----")
}

do {
    try sign()
} catch {
    FileHandle.standardError.write(Data("Cannot sign the runner manifest. Check the manifest and the canary key pair.\n".utf8))
    exit(1)
}
