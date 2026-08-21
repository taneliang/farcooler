import CryptoKit
import Foundation
import Network

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
            // True here, and true of every record this file makes: `pending`
            // means "this runner does not have the new device's key", and at
            // the moment a runner is resolved nothing has been written to it.
            // ``Enrollment/Outcome/granting(_:)`` clears it for the runners
            // that answered, and only for those. It was hardcoded false, which
            // made every unreachable runner announce itself as ready.
            pending: true)
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
    ///
    /// **And it starts pending like every other runner.** Reaching the local
    /// daemon over a Unix socket is not the same as having written to
    /// `~/.ssh/authorized_keys` on this Mac — that write is a `client enroll`
    /// with no `--runner`, and it fails whenever the daemon is not installed,
    /// not running, or refuses. That failure is not hypothetical: the bug
    /// ``Enrollment/arguments(key:label:clientID:scope:shell:runner:)``
    /// documents was exactly this write failing, and a hardcoded `pending:
    /// false` here is what let the phone be told its key had landed on the very
    /// Mac it was being added from.
    static func thisMac() -> CeremonyRunner {
        CeremonyRunner(
            id: "",
            label: thisMacName,
            alias: "",
            address: ProcessInfo.processInfo.hostName,
            user: NSUserName(),
            port: 22,
            host_key: localHostKey() ?? "",
            pending: true)
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

    /// Run a tool and take its stdout, with no shell anywhere in the middle.
    ///
    /// `timeout` is nil for the two tools this started with, and both earn it:
    /// `ssh -G` reads configuration files and `ssh-keygen -F` reads
    /// `known_hosts`, so neither touches the network and neither can sit there.
    /// `tailscale status` talks to a local daemon over a Unix socket, which is
    /// instant right up until that daemon is mid-upgrade or wedged — and this
    /// is called from the Add Device sheet's `prepare()`, so a tool that never
    /// returns is a sheet with a spinner and no way out. A killed tool prints
    /// nothing, which every caller here already reads as "could not say".
    fileprivate static func run(
        _ binary: String, _ arguments: [String], timeout: TimeInterval? = nil
    ) async -> String {
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
                // The lock is not paranoia about a lost signal — it is what
                // keeps `terminate()` from ever running against a process this
                // thread has already reaped. `waitUntilExit` sets `reaped`
                // while holding the lock, so the watchdog either gets there
                // first, on a process that is still alive, or finds the flag
                // set and does nothing. Foundation raises an ObjC exception
                // out of `terminate()` on a process it no longer has, and an
                // ObjC exception through Swift is a crash with a stack that
                // points at the language runtime rather than at here.
                let watch = ProcessWatch(process)
                var watchdog: DispatchWorkItem?
                if let timeout {
                    let item = DispatchWorkItem { watch.terminate() }
                    watchdog = item
                    DispatchQueue.global(qos: .userInitiated)
                        .asyncAfter(deadline: .now() + timeout, execute: item)
                }
                let data = pipe.fileHandleForReading.readDataToEndOfFile()
                process.waitUntilExit()
                watch.reaped()
                watchdog?.cancel()
                continuation.resume(returning: String(decoding: data, as: UTF8.self))
            }
        }
    }
}

/// A `Process` and the one question the watchdog is allowed to ask about it.
///
/// Lives outside ``RunnerFacts`` only because a `DispatchWorkItem` body is
/// `@Sendable` and `Process` is not: holding it behind this class is the point
/// where that is asserted, once, next to the lock that makes it true.
private final class ProcessWatch: @unchecked Sendable {
    private let lock = NSLock()
    private let process: Process
    private var isReaped = false

    init(_ process: Process) { self.process = process }

    func terminate() {
        lock.withLock {
            guard !isReaped, process.isRunning else { return }
            process.terminate()
        }
    }

    func reaped() { lock.withLock { isReaped = true } }
}

// MARK: - How far an address travels

extension RunnerFacts {
    /// Whether an address is one the other device can still use tomorrow, in a
    /// different building.
    ///
    /// The `address` in a ``CeremonyRunner`` is written into the new device
    /// once and never resolved again: there is no re-discovery on either side,
    /// and the repair is a person retyping it into a phone. A Mac granted over
    /// the LAN hands out `cosmo.local`, or `192.168.1.180` if the phone asked
    /// while `ssh` had a literal — correct in the room where the code was
    /// scanned, and dead in every other room. The moment the code is shown is
    /// the only moment where this Mac, the tailnet and the person are all
    /// present at once, so it is the only moment the question can be asked.
    ///
    /// **The judgement is on the string, never on a probe.** A reachability
    /// test answers "can I reach it from here", and "here" is precisely the
    /// network the address is about to stop working outside of — it would pass,
    /// every time, on exactly the addresses this exists to catch.
    enum Reach: Sendable {
        /// A public name, a public literal, or a tailnet address: it survives
        /// the walk out of the building.
        case anywhere
        /// mDNS, a bare hostname, or a private literal: this network only.
        case thisNetwork
        /// Nothing that reads as an address at all — an empty `ssh -G` answer,
        /// or `0.0.0.0`. Not a promise that it fails, a refusal to guess.
        case unknown
    }

    /// Which of the three ``Reach`` cases `address` falls into.
    ///
    /// Pure, and deliberately the only part of this that is: everything else
    /// here shells out, and the rules below are the part that has to be right
    /// on a Mac with no Tailscale, no network and no `ssh`.
    static func reach(of address: String) -> Reach {
        var text = address.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        // `[fd7a:115c:a1e0::f3b:cf77]`, and the `]:22` form. Brackets are how a
        // literal is written where a bare colon would read as a port — a URL, a
        // manifest, a `HostName` somebody pasted out of one — and they are not
        // part of the address.
        if text.hasPrefix("["), let close = text.firstIndex(of: "]") {
            text = String(text[text.index(after: text.startIndex)..<close])
        }
        // The DNS root label. `cosmo.local.` and `cosmo.local` are one name,
        // and `tailscale status` prints the dotted form (see ``Tailnet``).
        while text.hasSuffix(".") { text.removeLast() }
        // The zone, as in `fe80::1%en0`. It names an interface on THIS Mac, so
        // it could not travel even if the address it qualifies did.
        if let percent = text.firstIndex(of: "%") { text = String(text[..<percent]) }
        guard !text.isEmpty else { return .unknown }

        if let literal = IPv6Address(text) { return reach(ofIPv6: literal) }
        // Three dots, THEN `IPv4Address`. The count is not redundant: Network's
        // parser takes the short forms `inet_aton` takes, so `IPv4Address("1")`
        // succeeds as `0.0.0.1` — and a runner whose `ssh_config` alias is `1`
        // is a bare hostname, not a loopback-adjacent literal.
        if text.filter({ $0 == "." }).count == 3, let literal = IPv4Address(text) {
            return reach(ofIPv4: Array(literal.rawValue))
        }
        return reach(ofName: text)
    }

    private static func reach(ofIPv4 octets: [UInt8]) -> Reach {
        switch (octets[0], octets[1]) {
        // 0.0.0.0/8, RFC 1122's "this network". Not somewhere anything is
        // reached, so there is nothing to say about how far it goes.
        case (0, _): return .unknown
        case (10, _), (127, _), (192, 168): return .thisNetwork
        case (172, 16...31): return .thisNetwork
        // 169.254.0.0/16 — self-assigned, which on a Mac means DHCP did not
        // answer. It is the one private range that is not even stable for the
        // machine that has it.
        case (169, 254): return .thisNetwork
        // 100.64.0.0/10, RFC 6598. Shared address space on paper; in practice
        // the range Tailscale hands its nodes — this Mac's own tailnet address
        // is 100.66.207.119. It reaches from anywhere the other device is on
        // the tailnet, which is the entire reason a tailnet is worth offering.
        case (100, 64...127): return .anywhere
        default: return .anywhere
        }
    }

    private static func reach(ofIPv6 address: IPv6Address) -> Reach {
        let bytes = Array(address.rawValue)
        // `::ffff:192.168.1.1` — an IPv4 address wearing a v6 coat. What
        // decides it is the v4 inside, not the prefix around it.
        if bytes.prefix(10).allSatisfy({ $0 == 0 }), bytes[10] == 0xff, bytes[11] == 0xff {
            return reach(ofIPv4: Array(bytes.suffix(4)))
        }
        // `::`, the unspecified address, and `::1`.
        if bytes.allSatisfy({ $0 == 0 }) { return .unknown }
        if bytes.prefix(15).allSatisfy({ $0 == 0 }), bytes[15] == 1 { return .thisNetwork }
        // fe80::/10, link-local: the v6 equivalent of 169.254.
        if bytes[0] == 0xfe, bytes[1] & 0xc0 == 0x80 { return .thisNetwork }
        if bytes[0] & 0xfe == 0xfc {
            // fc00::/7 is unique-local — except for fd7a:115c:a1e0::/48, which
            // is inside it and is Tailscale's. Every node in a tailnet has one
            // beside its 100.x address (`fd7a:115c:a1e0::f3b:cf77` is this
            // Mac's), and it reaches exactly as far as the 100.x one does.
            // Reading it as "your Wi-Fi only" would offer to improve on an
            // address that is already the improvement.
            let tailscale: [UInt8] = [0xfd, 0x7a, 0x11, 0x5c, 0xa1, 0xe0]
            return Array(bytes.prefix(6)) == tailscale ? .anywhere : .thisNetwork
        }
        return .anywhere
    }

    private static func reach(ofName name: String) -> Reach {
        // Tailscale MagicDNS. Checked before anything else because it is the
        // one suffix this file also HANDS OUT, and the two have to agree.
        //
        // Note that ``Tailnet`` is stricter than this when it offers one: a
        // MagicDNS name only resolves while the tailnet has MagicDNS switched
        // on, and a name already in an `ssh_config` is evidence that it does.
        if name == "ts.net" || name.hasSuffix(".ts.net") { return .anywhere }
        // Multicast DNS, RFC 6762. `cosmo.local` is not a name that is looked
        // up; it is a shout down the local link, and `ProcessInfo.hostName`
        // hands one back on almost every Mac.
        if name == "local" || name.hasSuffix(".local") { return .thisNetwork }
        // RFC 8375, which reserved this for residential networks precisely so
        // that it would never be globally resolvable. Routers hand it out.
        if name == "home.arpa" || name.hasSuffix(".home.arpa") { return .thisNetwork }
        // A single label — `cosmo`, or an `ssh_config` alias. It resolves
        // through whatever the resolver's search domains say today, which on a
        // phone in a different country is nothing.
        if !name.contains(".") { return .thisNetwork }
        return .anywhere
    }

    /// One runner's address, judged, plus what to use instead.
    struct Addressing: Equatable, Sendable {
        /// What `ssh -G` resolved the target to — the address that goes into
        /// the other device if nobody changes anything.
        let address: String
        /// How far ``address`` travels.
        let reach: Reach
        /// An address for the same runner that does travel, when ``address``
        /// does not and one was found. Nil is the ordinary answer: most people
        /// have no Tailscale, and this is the case that must cost nothing.
        let betterAddress: String?
    }

    /// What is true about the address a runner is about to be granted under.
    ///
    /// Takes the ``CeremonyRunner`` rather than the target string because
    /// ``facts(for:label:)`` has already paid for the `ssh -G`, and asking a
    /// second question by running it a second time is a second answer that can
    /// disagree with the first. Which is why this stayed synchronous and why
    /// that signature did not have to change.
    ///
    /// `tailnet` is passed in for the same reason: ``Tailnet/current()`` spawns
    /// a process, the answer is identical for every runner in the list, and
    /// ``AddDeviceView`` walks every runner this Mac has. Load it once above
    /// the loop; nil there means no Tailscale, which is not an error and not
    /// unusual.
    static func addressing(of runner: CeremonyRunner, in tailnet: Tailnet?) -> Addressing {
        let reach = reach(of: runner.address)
        guard reach != .anywhere else {
            return Addressing(address: runner.address, reach: reach, betterAddress: nil)
        }
        // The empty id is this Mac, the same convention ``thisMac()`` is built
        // on. It matters here because Self and a peer are separate objects in
        // `tailscale status`, and this Mac never appears in its own peer list.
        let candidate =
            runner.id.isEmpty
            ? tailnet?.selfAddress
            : tailnet?.address(forHostName: hostName(in: runner.id), orAddress: runner.address)
        // Offered only when it is genuinely an improvement. A tailnet whose
        // MagicDNS is off answers with a 100.x literal, and a runner already
        // addressed by its MagicDNS name would otherwise be told to replace an
        // address with itself.
        guard let candidate, candidate != runner.address,
            RunnerFacts.reach(of: candidate) == .anywhere
        else {
            return Addressing(address: runner.address, reach: reach, betterAddress: nil)
        }
        return Addressing(address: runner.address, reach: reach, betterAddress: candidate)
    }

    /// `you@box` → `box`; `box` → `box`.
    ///
    /// The runner id is the target string, which is what a person typed and not
    /// necessarily a hostname at all — it can be an `ssh_config` alias whose
    /// `HostName` is something else entirely. That is fine: this is one of
    /// several things ``Tailnet`` tries to match a node on, and the address
    /// `ssh -G` resolved is the one that carries weight.
    private static func hostName(in target: String) -> String {
        let afterUser = target.split(separator: "@", maxSplits: 1, omittingEmptySubsequences: false)
        return String(afterUser.last ?? "")
    }
}

// MARK: - Tailscale

/// The tailnet this Mac is on, as far as `tailscale status --json` will say.
///
/// **Why a tailnet is the answer to a dead LAN address.** A phone granted
/// `cosmo.local` loses the runner the moment it leaves the Wi-Fi, and the only
/// address that survives that walk without somebody port-forwarding a home
/// router is a tailnet one. Far Cooler does not set Tailscale up and never
/// will — but when it is already there, this Mac knows the travelling name for
/// the very runner being granted, and not offering it is the app watching a
/// person write down an address it knows is about to break.
///
/// **Absent is the normal case.** Most people have no Tailscale, so every path
/// through this — no binary, a logged-out daemon, a daemon that does not
/// answer, JSON in a shape a later version invented — returns nothing rather
/// than an error. Nothing here is allowed to become a reason a ceremony fails.
///
/// The shape below was read off `tailscale status --json` on a real tailnet
/// (client 1.102.1), and three things in that output are the reason the
/// matching is as careful as it is: `HostName` is a display name and repeats
/// (two nodes called `Carl`, another calling itself `localhost`), `DNSName`
/// need not resemble it at all (`eliang-machine` is `parasky1.tail23af.ts.net.`
/// on the wire), and `MagicDNSEnabled` can be false — which makes every
/// `.ts.net` name in the file unresolvable and the 100.x addresses the only
/// thing worth handing anybody.
struct Tailnet: Sendable {
    /// One node, reduced to the three fields that decide anything.
    struct Node: Sendable, Equatable {
        /// `Cosmo`, `E-Liang's MacBook Pro`, `localhost`. A label a person
        /// chose, unique to nothing.
        let hostName: String
        /// `cosmo.tail23af.ts.net`, with the trailing root dot already off.
        let dnsName: String
        /// `TailscaleIPs`, in the order the daemon lists them — the 100.x one
        /// first, then the `fd7a:` one.
        let addresses: [String]

        /// `cosmo`, out of `cosmo.tail23af.ts.net`.
        var dnsLabel: String { String(dnsName.prefix { $0 != "." }) }
    }

    /// Whether the tailnet resolves its own names. False makes every `DNSName`
    /// here a label and not an address.
    let magicDNSEnabled: Bool
    /// This Mac, which `tailscale status` reports separately and never among
    /// the peers.
    let selfNode: Node?
    /// Everything else on the tailnet, online or not. Offline is not a reason
    /// to withhold an address: a runner that is asleep now is still reached by
    /// that name when it wakes, and the alternative is a ceremony whose result
    /// depends on what happened to be powered on.
    let peers: [Node]

    /// The travelling address for this Mac, or nil if there is not one.
    var selfAddress: String? { selfNode.flatMap(travelingAddress(of:)) }

    /// The travelling address for a runner, matched on what little is known
    /// about it: the name in its target string, and whatever `ssh -G` resolved.
    func address(forHostName hostName: String, orAddress address: String) -> String? {
        node(forHostName: hostName, orAddress: address).flatMap(travelingAddress(of:))
    }

    /// The node behind a runner, in descending order of how much the evidence
    /// is worth.
    ///
    /// Order is the whole design here. A wrong match does not degrade to a
    /// missing feature — it writes ANOTHER machine's address into the device
    /// being granted, and the person finds out when a terminal opens somewhere
    /// they did not mean. So an address, which is unique on a tailnet, is
    /// believed before a name, which is not.
    private func node(forHostName hostName: String, orAddress address: String) -> Node? {
        let hostName = hostName.trimmingCharacters(in: .whitespaces).lowercased()
        var address = address.trimmingCharacters(in: .whitespaces).lowercased()
        while address.hasSuffix(".") { address.removeLast() }

        if !address.isEmpty {
            // The runner is already addressed by a tailnet literal, or by its
            // MagicDNS name. Both are exact, and both usually mean there is
            // nothing to improve — which ``RunnerFacts/addressing(of:in:)``
            // works out for itself once it has the answer.
            if let hit = peers.first(where: { $0.addresses.contains(address) }) { return hit }
            if let hit = peers.first(where: { $0.dnsName == address }) { return hit }
        }

        // `cosmo` out of the target `you@cosmo`, and `cosmo` out of the address
        // `cosmo.local` that `ssh -G` resolved it to. Either can be the one
        // that lands: an alias in `~/.ssh/config` need not resemble the machine,
        // and an address handed out by DHCP need not resemble either.
        let labels = [hostName, String(address.prefix { $0 != "." })].filter(isUsable)

        // The DNS label, which the tailnet assigned and made unique.
        for label in labels {
            if let hit = peers.first(where: { $0.dnsLabel == label }) { return hit }
        }
        // `HostName` last, and only when exactly one node claims it. Two nodes
        // called `Carl` is not a corner case somebody constructed — it is what
        // this Mac's own tailnet looks like, and picking the first of them
        // would be picking at random.
        for label in labels {
            let claimants = peers.filter { $0.hostName.lowercased() == label }
            if claimants.count == 1 { return claimants[0] }
        }
        return nil
    }

    /// Whether a label is worth matching a whole machine on.
    ///
    /// `localhost` is the one that has to be named: a node in this Mac's tailnet
    /// really does report it as its `HostName`, and `localhost` is also what
    /// somebody's runner target says when the runner is this very machine.
    /// Matching those two would grant a phone an address for a stranger's iPad.
    private func isUsable(_ label: String) -> Bool {
        !label.isEmpty && label != "localhost" && label != "local"
    }

    /// What to hand another device for this node.
    ///
    /// The MagicDNS name when the tailnet serves it, and the raw tailnet
    /// address when it does not. That branch is not defensive coding: the
    /// tailnet this was written against has `MagicDNSEnabled` false, so
    /// `cosmo.tail23af.ts.net` appears in every line of its status output and
    /// resolves for nobody. The 100.x address always works.
    private func travelingAddress(of node: Node) -> String? {
        if magicDNSEnabled, !node.dnsName.isEmpty { return node.dnsName }
        return node.addresses.first { RunnerFacts.reach(of: $0) == .anywhere }
    }
}

extension Tailnet {
    /// Where the `tailscale` command line tool is, in the order it is looked
    /// for.
    ///
    /// A fixed list, and never `$PATH`. A GUI app's `PATH` is whatever
    /// `launchd` gave it rather than whatever the person's shell has, so the
    /// lookup would miss on the Macs where it should hit — and on the ones
    /// where it hit, it would be executing the first thing named `tailscale` in
    /// a list of directories the user can write to.
    ///
    /// The third entry is the Mac app's own binary, which is not a separate
    /// CLI: `/Applications/Tailscale.app/Contents/MacOS/Tailscale status --json`
    /// answers exactly as the Homebrew binary does. It is last because the
    /// standalone tool, when both are installed, is the one the person has been
    /// using.
    static let binaries = [
        "/usr/local/bin/tailscale",
        "/opt/homebrew/bin/tailscale",
        "/Applications/Tailscale.app/Contents/MacOS/Tailscale",
    ]

    /// Ask the local Tailscale daemon what it knows, or answer nothing.
    ///
    /// Nothing, specifically, when: no binary is installed, the daemon is not
    /// running, nobody is logged in, the daemon does not answer inside the
    /// timeout, or the JSON is not what this expects. Every one of those is an
    /// ordinary Mac rather than a fault, and none of them is worth a sentence
    /// on screen.
    static func current() async -> Tailnet? {
        let installed = FileManager.default
        guard let binary = binaries.first(where: { installed.isExecutableFile(atPath: $0) })
        else { return nil }
        let output = await RunnerFacts.run(binary, ["status", "--json"], timeout: 2)
        return Tailnet(json: Data(output.utf8))
    }

    /// Read one `tailscale status --json` document.
    ///
    /// Separate from ``current()`` so that the parsing — which is the part with
    /// rules in it — is tested against a captured document rather than against
    /// whatever this Mac's tailnet happens to contain today.
    init?(json: Data) {
        guard let status = try? JSONDecoder().decode(Status.self, from: json) else { return nil }
        // `BackendState` is `Running` only once the daemon is up AND logged in;
        // a signed-out Mac says `NeedsLogin`, a stopped one `Stopped`, and both
        // still hand over a `Self` full of stale names from the last tailnet.
        // Offering one of those would be worse than offering nothing.
        guard status.backendState == "Running" else { return nil }

        magicDNSEnabled = status.currentTailnet?.magicDNSEnabled ?? false
        selfNode = status.selfNode.flatMap(Node.init(status:))
        // Dictionary order is whatever the JSON had; sorted so that a tie
        // between two nodes is decided the same way twice. The matching rules
        // are written not to have ties, and this is what makes that testable.
        peers = (status.peers ?? [:]).values
            .compactMap(Node.init(status:))
            .sorted { $0.dnsName < $1.dnsName }
    }

    /// `tailscale status --json`, reduced to the keys this reads.
    ///
    /// Every field optional, on purpose. Tailscale adds keys between releases —
    /// `PeerRelay` and `TaildropTarget` were not always there — and a decoder
    /// that treats an absent key as a failure would make a Tailscale upgrade
    /// break enrollment on a Mac that never asked for any of this. Unknown keys
    /// are ignored by `Decodable`, missing ones become nil, and a document that
    /// is the wrong shape entirely fails the `try?` in ``init(json:)``.
    fileprivate struct Status: Decodable {
        struct Node: Decodable {
            let hostName: String?
            let dnsName: String?
            let addresses: [String]?

            enum CodingKeys: String, CodingKey {
                case hostName = "HostName"
                case dnsName = "DNSName"
                case addresses = "TailscaleIPs"
            }
        }

        struct CurrentTailnet: Decodable {
            let magicDNSEnabled: Bool?

            enum CodingKeys: String, CodingKey {
                case magicDNSEnabled = "MagicDNSEnabled"
            }
        }

        let backendState: String?
        let selfNode: Node?
        let peers: [String: Node]?
        let currentTailnet: CurrentTailnet?

        enum CodingKeys: String, CodingKey {
            case backendState = "BackendState"
            // `Self` on the wire, which cannot be a property name here.
            case selfNode = "Self"
            // Singular, and a dictionary keyed by node key — `Peer`, not
            // `Peers`. The keys are of no use to anything in this file.
            case peers = "Peer"
            case currentTailnet = "CurrentTailnet"
        }
    }
}

extension Tailnet.Node {
    /// A node worth keeping, or nil.
    ///
    /// A node with neither a name nor an address is not something another
    /// device could be pointed at, whatever else the daemon knows about it.
    fileprivate init?(status: Tailnet.Status.Node) {
        var dnsName = (status.dnsName ?? "").lowercased()
        // `cosmo.tail23af.ts.net.` — the daemon prints the fully qualified
        // form, root dot and all, and everything downstream compares it against
        // addresses that do not have one.
        while dnsName.hasSuffix(".") { dnsName.removeLast() }
        let addresses = (status.addresses ?? []).map { $0.lowercased() }
        guard !dnsName.isEmpty || !addresses.isEmpty else { return nil }

        self.hostName = status.hostName ?? ""
        self.dnsName = dnsName
        self.addresses = addresses
    }
}
