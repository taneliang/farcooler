#!/usr/bin/env swift
// The public half of a Sparkle signing key, derived from the private half.
//
// This exists to answer one question CI could not otherwise ask: is the key we
// are about to SIGN with actually the pair of the key every installed app was
// built to TRUST?
//
// Those two values travel by completely different routes. The public half is
// committed in `apps/macos/sparkle-public-keys.txt` and stamped into the bundle
// at build time; the private half lives in a Keychain, was exported by hand, and
// reached CI as a GitHub secret. Nothing has ever compared them. If they are not
// a pair, everything looks perfect — CI signs, the appcast publishes, the feed
// serves — and every client silently refuses the update, because a signature
// that does not verify is indistinguishable to Sparkle from one that was forged.
// The failure appears on other people's machines and nowhere else.
//
// Sparkle's `generate_keys -x` exports the raw 32-byte ed25519 SEED, base64
// encoded, and its own help says the seed "can be used to create the
// private/public keypair with other tools that support EdDSA signing". CryptoKit
// is such a tool and ships with macOS, so this needs no dependency at all.
//
//   echo "$CANARY_SPARKLE_KEY" | swift scripts/sparkle-public-key.swift
//
// Prints the base64 public key, or exits non-zero saying why.
import CryptoKit
import Foundation

func fail(_ message: String) -> Never {
    FileHandle.standardError.write(Data((message + "\n").utf8))
    exit(1)
}

// Read from standard input rather than an argument, so the key never appears in
// a process listing or a shell history — and so a workflow can pipe a secret
// straight in without writing it to disk first.
let raw = FileHandle.standardInput.readDataToEndOfFile()
guard let text = String(data: raw, encoding: .utf8) else {
    fail("the private key on stdin is not text")
}

let encoded = text.trimmingCharacters(in: .whitespacesAndNewlines)
guard !encoded.isEmpty else {
    fail("no private key on stdin")
}
guard let seed = Data(base64Encoded: encoded) else {
    fail("the private key is not valid base64")
}
// 32 bytes is the seed format `generate_keys -x` writes today. An older Sparkle
// key is 64 bytes (seed and public half concatenated); say so plainly rather
// than deriving something wrong from the first half of one.
guard seed.count == 32 else {
    fail("expected a 32-byte ed25519 seed, got \(seed.count) bytes — an older 64-byte key needs converting first")
}

guard let key = try? Curve25519.Signing.PrivateKey(rawRepresentation: seed) else {
    fail("those 32 bytes are not a usable ed25519 seed")
}
print(key.publicKey.rawRepresentation.base64EncodedString())
