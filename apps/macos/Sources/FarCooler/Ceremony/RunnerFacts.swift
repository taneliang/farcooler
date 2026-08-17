import CryptoKit
import Foundation

/// Turning a runner this Mac reaches into the record another device can reach
/// it by.
///
/// A runner is stored here as one string — `you@box`, or an `ssh_config` alias
/// somebody typed. That is enough for this Mac, which hands it to `ssh` and
/// lets ssh resolve it. It is not enough for a phone: the phone has no
/// `~/.ssh/config`, no known_hosts, and no `ssh` binary. So the reply carries
/// the resolved address, user and port, and the host key to pin.
///
/// **Resolved by asking ssh, never by parsing the string.** `you@box` is easy;
/// a `Host box` block with `HostName box.tail-1234.ts.net`, `User deploy`, a
/// `Port 2222` and a `ProxyJump` is what people actually have, and a regular
/// expression over the target would hand the phone an address that does not
/// exist.
enum RunnerFacts {
    /// What another device needs to reach `target`, or nil if ssh could not say.
    ///
    /// The host key comes out of THIS Mac's `known_hosts` rather than from a
    /// fresh `ssh-keyscan`: a keyscan trusts whatever answers right now, which
    /// is the unknown-host prompt with the human removed. What travels in the
    /// reply is the key this Mac has already been using, so an interception
    /// becomes a refusal on the new device instead of a silent success.
    static func facts(for target: String, label: String) async -> CeremonyRunner? {
        let resolved = await resolve(target)
        guard let address = resolved["hostname"], !address.isEmpty else { return nil }
        let user = resolved["user"] ?? NSUserName()
        let port = Int(resolved["port"] ?? "22") ?? 22

        return CeremonyRunner(
            // The target string, which is what this Mac calls this runner
            // everywhere else — its settings row, its reachability state, its
            // editor target. A runner has no id of its own on this Mac to use
            // instead, and inventing a UUID here would make one that nothing
            // else on this Mac agrees with.
            id: target,
            label: label.isEmpty ? target : label,
            // Filled in by `SshConfig.aliases`, which is the only thing that can
            // choose it: the answer depends on what is already in the file.
            alias: "",
            address: address,
            user: user,
            port: port,
            host_key: await hostKey(for: address) ?? "",
            pending: false)
    }

    /// This Mac, as something another device can reach.
    ///
    /// The empty target is a real runner in this app — the local daemon, over a
    /// Unix socket, with no key and no SSH. That is exactly why it needs
    /// resolving differently: there is no ssh configuration describing this Mac
    /// to itself, so the address is its own network name and the host key is
    /// the one sshd will present, read from `/etc/ssh`.
    ///
    /// A phone reaching it still needs Remote Login turned on, which macOS
    /// keeps off. That is the one screen this app cannot do for somebody, and
    /// ``RemoteLoginView`` is it.
    static func thisMac() -> CeremonyRunner {
        CeremonyRunner(
            id: "",
            label: thisMacName,
            alias: "",
            address: ProcessInfo.processInfo.hostName,
            user: NSUserName(),
            port: 22,
            host_key: localHostKey() ?? "",
            pending: false)
    }

    /// The fingerprint of the key this Mac's sshd presents.
    ///
    /// Read from the public half in `/etc/ssh`, which is world-readable — the
    /// private half is not, and is not wanted. Ed25519 first, since that is
    /// what a current macOS presents and what the client pins.
    private static func localHostKey() -> String? {
        let candidates = [
            "/etc/ssh/ssh_host_ed25519_key.pub",
            "/etc/ssh/ssh_host_ecdsa_key.pub",
            "/etc/ssh/ssh_host_rsa_key.pub",
        ]
        guard let path = candidates.first(where: { FileManager.default.fileExists(atPath: $0) })
        else { return nil }
        guard let line = try? String(contentsOfFile: path, encoding: .utf8) else { return nil }
        return fingerprint(ofOpenSSHKey: line)
    }

    /// `ssh-ed25519 AAAA… comment` → `SHA256:…`, as `ssh-keygen -lf` prints it.
    ///
    /// The digest of the base64 blob, which is what the format is — not an
    /// implementation of anything. Far Cooler's own keys are fingerprinted in
    /// Rust by `ssh-key`, and everything that decides anything uses that;
    /// this is for a label and for a key file `ssh-keygen` is not being run
    /// against.
    static func fingerprint(ofOpenSSHKey line: String) -> String? {
        let fields = line.split(separator: " ", omittingEmptySubsequences: true)
        guard fields.count >= 2, let blob = Data(base64Encoded: String(fields[1])) else {
            return nil
        }
        let digest = SHA256.hash(data: blob)
        // Base64, with the padding stripped, which is how OpenSSH prints it.
        let encoded = Data(digest).base64EncodedString().replacingOccurrences(of: "=", with: "")
        return "SHA256:\(encoded)"
    }

    /// `ssh -G` — what ssh itself would use for this target, after every
    /// `Host`, `Match` and `Include` in the configuration.
    private static func resolve(_ target: String) async -> [String: String] {
        // `--`, then the destination. ssh reads a leading dash as a flag
        // wherever it appears, so a target of `-oProxyCommand=…` is command
        // execution on THIS Mac. Every target is typed by a human today, which
        // is why it has never mattered; onboarding has them arrive in a scanned
        // manifest.
        let output = await run("/usr/bin/ssh", ["-G", "--", target])
        var facts: [String: String] = [:]
        for line in output.components(separatedBy: .newlines) {
            let parts = line.split(separator: " ", maxSplits: 1, omittingEmptySubsequences: true)
            guard parts.count == 2 else { continue }
            facts[parts[0].lowercased()] = String(parts[1])
        }
        return facts
    }

    /// The `SHA256:…` fingerprint this Mac already trusts for that address.
    private static func hostKey(for address: String) async -> String? {
        let output = await run("/usr/bin/ssh-keygen", ["-l", "-F", address])
        // `# Host box found: line 3` then `256 SHA256:… box (ED25519)`. Ed25519
        // first when the file holds several: it is what every runner this
        // project installs presents.
        let lines = output.components(separatedBy: .newlines).filter { !$0.hasPrefix("#") }
        let preferred = lines.first { $0.contains("(ED25519)") } ?? lines.first
        return preferred?
            .split(separator: " ")
            .first { $0.hasPrefix("SHA256:") }
            .map(String.init)
    }

    private static func run(_ binary: String, _ arguments: [String]) async -> String {
        await withCheckedContinuation { continuation in
            DispatchQueue.global(qos: .userInitiated).async {
                let process = Process()
                process.executableURL = URL(fileURLWithPath: binary)
                process.arguments = arguments
                let pipe = Pipe()
                process.standardOutput = pipe
                // Swallowed on purpose: `ssh-keygen -F` on a host it does not
                // know says so on stderr and exits non-zero, which is an
                // ordinary answer here and not something to show anybody.
                process.standardError = FileHandle.nullDevice
                do { try process.run() } catch {
                    continuation.resume(returning: "")
                    return
                }
                let data = pipe.fileHandleForReading.readDataToEndOfFile()
                process.waitUntilExit()
                continuation.resume(returning: String(decoding: data, as: UTF8.self))
            }
        }
    }
}
