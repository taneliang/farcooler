import CFarCoolerClient
import Foundation

/// Key B: the ordinary SSH key a Mac uses for everything that is not Far Cooler.
///
/// **A key can prove who is holding it, or it can open a shell. Not both.**
/// Key A carries `command="farcoolerd --stdio --client … --scope …"`, which is
/// what makes a device's identity server-asserted — and a forced command means
/// sshd runs that program and nothing else, so Zed's `ssh://` asks for a shell
/// and gets the daemon. Take the forced command away and there is nowhere left
/// to put the client id. Hence two keys, and hence this file.
///
/// Key B is yours in a way Key A never is: a plain line in `authorized_keys`, a
/// plain file in `~/.ssh`, and an entry in `~/.ssh/config` so Zed, git and
/// Terminal find it without knowing Far Cooler exists. Delivering that is most
/// of the value of onboarding a Mac.
enum ShellKey {
    /// Where a generated Key B lands. The user's own `~/.ssh`, deliberately —
    /// Key A hides in the channel's key directory because nothing but Far
    /// Cooler may use it, and Key B is the opposite of that.
    static var directory: URL {
        URL(fileURLWithPath: NSHomeDirectory()).appendingPathComponent(".ssh")
    }

    /// The default name for a new key: `farcooler-<device>`.
    ///
    /// Slugged rather than used as typed. A device name is user-supplied and
    /// arrives in a scanned code — spaces, slashes and `..` all become a file
    /// in the wrong place or a name `~/.ssh/config` cannot quote.
    static func defaultName(for device: String) -> String {
        let allowed = CharacterSet(charactersIn: "abcdefghijklmnopqrstuvwxyz0123456789-")
        let slug = device.lowercased()
            .replacingOccurrences(of: " ", with: "-")
            .unicodeScalars
            .filter { allowed.contains($0) }
            .map(String.init)
            .joined()
            .trimmingCharacters(in: CharacterSet(charactersIn: "-"))
        return slug.isEmpty ? "farcooler-device" : "farcooler-\(slug)"
    }

    /// Generate a key pair and write it where ssh will find it.
    ///
    /// The pair comes from `farcooler_client_generate_key`, which is the same
    /// `ssh-key` this project uses everywhere else. Far Cooler implements no
    /// cryptography of its own, and writing an OpenSSH private-key encoder a
    /// second time in Swift to save a function call would be a poor trade.
    ///
    /// `O_EXCL`, so an existing file at that name is never written through: the
    /// one at `~/.ssh/id_ed25519` may be the key you log in to everything with.
    static func generate(named name: String) throws -> (path: URL, publicKey: String) {
        var buffer = [UInt8](repeating: 0, count: 4096)
        let written = name.withCString { comment in
            buffer.withUnsafeMutableBufferPointer {
                farcooler_client_generate_key(comment, $0.baseAddress, $0.count)
            }
        }
        guard written > 0, written <= buffer.count,
            let object = try? JSONSerialization.jsonObject(with: Data(buffer[0..<written]))
                as? [String: Any],
            let privateKey = object["private_key"] as? String,
            let publicKey = object["public_key"] as? String
        else { throw ShellKeyError.couldNotGenerate }

        try FileManager.default.createDirectory(
            at: directory, withIntermediateDirectories: true,
            attributes: [.posixPermissions: 0o700])

        let path = directory.appendingPathComponent(name)
        try write(privateKey, to: path, mode: 0o600)
        // The public half beside it, because that is where `ssh-add`,
        // `ssh-keygen -y` and every person who has ever used ssh expect it.
        try write(publicKey + "\n", to: path.appendingPathExtension("pub"), mode: 0o644)
        return (path, publicKey)
    }

    /// Every key already in `~/.ssh`, by its public half.
    ///
    /// Listed by the public file because that is the one that is safe to read
    /// and the one that says what the key is. A private key with no `.pub`
    /// beside it is skipped rather than decrypted — it may be passphrase-
    /// protected or FIDO-backed, and asking for a passphrase to populate a
    /// menu is not a thing this should do.
    static func existing() -> [ExistingKey] {
        let files =
            (try? FileManager.default.contentsOfDirectory(
                at: directory, includingPropertiesForKeys: nil)) ?? []
        return
            files
            .filter { $0.pathExtension == "pub" }
            .compactMap { pub -> ExistingKey? in
                let priv = pub.deletingPathExtension()
                guard FileManager.default.fileExists(atPath: priv.path) else { return nil }
                guard let line = try? String(contentsOf: pub, encoding: .utf8) else { return nil }
                let text = line.trimmingCharacters(in: .whitespacesAndNewlines)
                guard !text.isEmpty else { return nil }
                return ExistingKey(path: priv, publicKey: text)
            }
            .sorted { $0.name < $1.name }
    }

    private static func write(_ contents: String, to path: URL, mode: Int) throws {
        let descriptor = open(path.path, O_WRONLY | O_CREAT | O_EXCL | O_NOFOLLOW, mode_t(mode))
        guard descriptor >= 0 else { throw ShellKeyError.alreadyThere(path.lastPathComponent) }
        defer { close(descriptor) }
        let bytes = Array(contents.utf8)
        guard bytes.withUnsafeBufferPointer({ Foundation.write(descriptor, $0.baseAddress, $0.count) })
            == bytes.count
        else { throw ShellKeyError.couldNotWrite(path.lastPathComponent) }
    }
}

/// A key that was already in `~/.ssh` before Far Cooler looked.
struct ExistingKey: Identifiable, Hashable {
    let path: URL
    let publicKey: String

    var id: String { path.path }
    var name: String { path.lastPathComponent }

    /// `ssh-ed25519 AAAA… you@mac` → `you@mac`, for the menu.
    var comment: String {
        let parts = publicKey.split(separator: " ", maxSplits: 2, omittingEmptySubsequences: true)
        return parts.count == 3 ? String(parts[2]) : ""
    }
}

/// What a Mac chose about shell access, before it showed its code.
///
/// Made on the Mac being ADDED, which is the only device that can make it: the
/// private half of Key B never leaves that Mac, and neither does its `~/.ssh`.
/// The granting device sees the result — `key_b` present in the offer or not —
/// and shows it, rather than choosing it.
struct ShellKeyChoice: Equatable {
    /// Whether shell access is part of this at all. On by default: a Mac whose
    /// Zed cannot open a remote worktree got the smaller half of what
    /// onboarding is for.
    var wanted = true
    var source: Source = .new
    /// The `~/.ssh/config` block. On by default, and separable from the key
    /// because enrolling the key and telling ssh about it are two things —
    /// somebody who manages their own config can take the key and skip this.
    var addToConfig = true
    /// Editable, defaulted from the device's name.
    var name: String
    /// The existing key, when one was chosen.
    var existing: ExistingKey?

    enum Source: Equatable {
        case new
        case existing
    }

    init(deviceName: String) {
        name = ShellKey.defaultName(for: deviceName)
    }

    /// **Generating is the default because it is independently revocable, not
    /// because it is safer.**
    ///
    /// An existing key may be passphrase-protected, agent-held or FIDO-backed,
    /// and so better protected than a fresh `0600` file. What it cannot be is
    /// removed without consequence — which is what this sentence says, on the
    /// screen, at the moment the choice is made.
    var warning: String? {
        guard wanted, source == .existing, let existing else { return nil }
        return "Removing this Mac later will remove \(existing.name) from those runners. "
            + "That takes away everything this key has always reached, not only Far Cooler."
    }
}

enum ShellKeyError: LocalizedError {
    case couldNotGenerate
    case alreadyThere(String)
    case couldNotWrite(String)

    var errorDescription: String? {
        switch self {
        case .couldNotGenerate:
            return "Far Cooler couldn't make a new key."
        case .alreadyThere(let name):
            return "There's already a key called \(name) in ~/.ssh. Pick another name."
        case .couldNotWrite(let name):
            return "Far Cooler couldn't write \(name) to ~/.ssh."
        }
    }
}
