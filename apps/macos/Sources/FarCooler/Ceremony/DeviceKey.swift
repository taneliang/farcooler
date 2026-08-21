import AgentKit
import CFarCoolerClient
import Foundation

/// Key A: the key Far Cooler uses to be Far Cooler, and nothing else uses at all.
///
/// A line enrolled for Key A carries
/// `restrict,command="farcoolerd --stdio --client … --scope …"`. The client id
/// is written into `authorized_keys` by whoever enrolled the key, so the
/// connecting device never sends it and cannot change it — which is what makes
/// writer leases, per-client idempotency and "close this device's sessions"
/// mean anything. The price is that sshd runs that program and only that
/// program, so this key cannot open a shell. See ``ShellKey`` for the one that
/// can.
///
/// **This key is never in `~/.ssh/config`.** Far Cooler passes it with `-i` on
/// the command line, so deleting Far Cooler's block from that file takes Zed's
/// access away and leaves Far Cooler's untouched. `SshConfig.assertNotKeyA`
/// keeps that true.
enum DeviceKey {
    /// One key per `(device, account)` pair.
    ///
    /// Not one fixed `keys/device`: two accounts on one Mac hold two keys that
    /// are never interchanged, and a single file would have the second account
    /// overwrite the first. The account id is the relay's opaque identifier —
    /// never a runner label, which is mutable, user-supplied, and full of `/`
    /// and `..`.
    ///
    /// **A config directory, not a runtime one.** A runtime directory is
    /// cleared on logout on some platforms, which would delete a private key
    /// whose public half is enrolled on every runner — a Mac that silently
    /// stops reaching anything and cannot revoke itself either.
    ///
    /// ## This path is duplicated, and should not stay that way
    ///
    /// The design says the CLI and the app resolve it through **one shared
    /// function**, because the CLI is what passes it to `ssh -i` and the two
    /// disagreeing means an app that enrolled a key the CLI never offers. That
    /// function does not exist yet — the CLI has no notion of Key A at all —
    /// so this is the Swift half of a rule that currently has one half. It
    /// wants an entry point beside the ceremony's four:
    ///
    /// ```c
    /// /* Where this device's Far Cooler key for `account` lives. */
    /// size_t farcooler_client_key_path(const char *account,
    ///                                  uint8_t *out, size_t capacity);
    /// ```
    static var directory: URL {
        URL(fileURLWithPath: NSHomeDirectory())
            .appendingPathComponent(".config")
            .appendingPathComponent("farcooler")
            .appendingPathComponent("keys")
            // Channel-scoped for the same reason the daemon's runtime directory
            // is: stable, preview, canary and local share nothing, and a canary
            // enrolling over the release build's key would be a fleet-wide
            // surprise.
            .appendingPathComponent(AppVersion.channel)
    }

    /// The private key for an account, generating one on first use.
    ///
    /// `0700` on the directory and `0600` on the file, with `O_EXCL` so an
    /// existing key is never written through. A private key whose public half
    /// is already enrolled everywhere is not a file to overwrite on a retry.
    static func privateKey(for account: String) throws -> (path: URL, publicKey: String) {
        guard !account.isEmpty, account.allSatisfy(isSafe) else { throw DeviceKeyError.noAccount }
        let path = directory.appendingPathComponent(account)

        if let existing = try? String(contentsOf: path, encoding: .utf8), !existing.isEmpty {
            guard let publicKey = publicKey(of: existing) else { throw DeviceKeyError.unreadable }
            return (path, publicKey)
        }

        var buffer = [UInt8](repeating: 0, count: 4096)
        let comment = "farcooler-\(thisMacName)"
        let written = comment.withCString { comment in
            buffer.withUnsafeMutableBufferPointer {
                farcooler_client_generate_key(comment, $0.baseAddress, $0.count)
            }
        }
        guard written > 0, written <= buffer.count,
            let object = try? JSONSerialization.jsonObject(with: Data(buffer[0..<written]))
                as? [String: Any],
            let priv = object["private_key"] as? String,
            let pub = object["public_key"] as? String
        else { throw DeviceKeyError.unreadable }

        try FileManager.default.createDirectory(
            at: directory, withIntermediateDirectories: true,
            attributes: [.posixPermissions: 0o700])
        let descriptor = open(path.path, O_WRONLY | O_CREAT | O_EXCL | O_NOFOLLOW, mode_t(0o600))
        guard descriptor >= 0 else { throw DeviceKeyError.unreadable }
        defer { close(descriptor) }
        let bytes = Array(priv.utf8)
        guard
            bytes.withUnsafeBufferPointer({ Foundation.write(descriptor, $0.baseAddress, $0.count) })
                == bytes.count
        else { throw DeviceKeyError.unreadable }
        return (path, pub)
    }

    /// The public half, derived every time and never stored.
    ///
    /// Two copies of one identity diverge — the iOS app learned this the hard
    /// way, showing a key for someone to authorize while authenticating with a
    /// different one — so there is one file and everything else is derived
    /// from it.
    static func publicKey(of privateKey: String) -> String? {
        var buffer = [UInt8](repeating: 0, count: 2048)
        let written = privateKey.withCString { text in
            buffer.withUnsafeMutableBufferPointer {
                farcooler_client_public_key(text, $0.baseAddress, $0.count)
            }
        }
        guard written > 0, written <= buffer.count else { return nil }
        return String(decoding: buffer[0..<written], as: UTF8.self)
    }

    /// The client id a device is enrolled under, derived from its Key A.
    ///
    /// Any device's Key A, not only this Mac's: the ceremony hands this Mac a
    /// scanned code carrying the key of the device being added, and the id that
    /// goes into that device's line comes from that key.
    ///
    /// **Derived, not invented.** `farcooler_client_client_id` is
    /// `crates/client/src/ceremony.rs`'s `client_id` — the one place the format
    /// is decided, so the Mac, iOS and Android spell one device one way. That
    /// matters twice. The daemon's "this device is already enrolled" arm
    /// compares client ids, so an id invented per platform defeats it; and the
    /// id has to be STABLE, or a device re-running the ceremony against a runner
    /// it is already on enrolls a second line naming one device and nothing can
    /// afterwards say which session arrived on which key. This screen used to
    /// mint a `UUID()`, which is the per-run version of exactly that: every run
    /// a new device, as far as every fence was concerned.
    ///
    /// Same buffer contract as ``publicKey(of:)`` above, because it is the same
    /// kind of call: raw text rather than JSON, the byte count returned, nothing
    /// written when the buffer is short. `CeremonyFFI`'s `spill` belongs to the
    /// ceremony's four JSON entry points and is the wrong shape for this one.
    /// The id is `farcooler-` and twelve hex characters, so 128 bytes cannot be
    /// short — and a buffer that somehow were short answers nil rather than a
    /// truncated id, which is an id no revoke would match.
    ///
    /// Nil when the text is not a public key, and the caller must carry the nil
    /// rather than substitute anything. A device with no readable key has no id;
    /// enrolling it under a made-up one would put a line into somebody's
    /// `authorized_keys` that no `client revoke` can name.
    static func clientID(of publicKey: String) -> String? {
        var buffer = [UInt8](repeating: 0, count: 128)
        let written = publicKey.withCString { text in
            buffer.withUnsafeMutableBufferPointer {
                farcooler_client_client_id(text, $0.baseAddress, $0.count)
            }
        }
        guard written > 0, written <= buffer.count else { return nil }
        return String(decoding: buffer[0..<written], as: UTF8.self)
    }

    /// A relay account id, as a filename. Anything else is refused rather than
    /// sanitized: an id this does not recognize is not an id, and writing a key
    /// under a guessed name is how one ends up orphaned.
    private static func isSafe(_ character: Character) -> Bool {
        character.isASCII && (character.isLetter || character.isNumber || character == "_")
    }
}

/// What this Mac calls itself, for the code it shows and the key it writes.
///
/// The sharing name — "MacBook Air" — rather than the hostname, because it is
/// what the person adding this Mac will read on the other device's screen and
/// what they set in System Settings. `.local` is stripped from the fallback for
/// the same reason: nobody thinks of their Mac as `macbook-air.local`.
var thisMacName: String {
    let name = ProcessInfo.processInfo.hostName
    let trimmed = name.hasSuffix(".local") ? String(name.dropLast(6)) : name
    return trimmed.isEmpty ? "This Mac" : trimmed
}

enum DeviceKeyError: LocalizedError {
    case noAccount
    case unreadable

    var errorDescription: String? {
        switch self {
        case .noAccount:
            return "Sign in before adding this Mac to an account."
        case .unreadable:
            return "Far Cooler couldn’t read this Mac’s key."
        }
    }
}
