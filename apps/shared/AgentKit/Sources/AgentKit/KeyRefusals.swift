import Foundation

/// Whether a runner that refuses this device's key has taken its access away
/// (ov-818), or only refused it for a moment.
///
/// Since ov-783, removing a member, or taking a device off a member's list,
/// revokes the device's key lines from the runner's `authorized_keys`. Its
/// next connection is refused by sshd: "Permission denied (publickey)" from
/// OpenSSH on the Mac, `key_rejected` from the phones' core. Before this, the
/// Mac read that as a runner it couldn't reach, retried it on the fast
/// schedule and left the team's last-read work on screen.
///
/// **What it reads.** Only this device's own history with the runner: whether
/// the runner has let this device in before (``admitted``, kept across
/// launches), and how many dials in a row it has refused since
/// (``refusals``).
///
/// **Whose refusal it is.** On the phones, the core checks the host key
/// against the one this device pinned before it authenticates, so a refusal
/// is the runner's own. **The Mac pins nothing for a runner at its address**
/// (`SshConfig` pins only Tailcat's), and the CLI's ssh uses the owner's own
/// config and agent, so "Permission denied (publickey)" there can just as well
/// be an agent that wouldn't sign (a locked 1Password, a Touch ID prompt
/// nobody answered), a passphrase key with no agent, a refusing ProxyJump
/// bastion, an impostor let through by `StrictHostKeyChecking no`, or a
/// stray `~/.ssh/id_*` offered because the agent wasn't there. So the Mac
/// settles only on a refusal ``isProvenKeyRefusal(verbose:key:)`` reads off a
/// verbose probe it makes for the purpose (`DaemonClient.removal(after:)`),
/// and only of ``key``: the key the runner said this Mac came in with.
///
/// **What it tells a stranger: nothing.** The runner says nothing new. sshd
/// refuses a revoked key in exactly the words it refuses any key it has
/// never seen, so nobody dialing a runner can learn from it whether a key
/// was ever there. The device draws its conclusion from what it remembers.
///
/// **Revoked against transient.** A runner that never let this device in is a
/// stranger's refusal: ``Verdict/stranger``, and nothing changes from before.
/// A runner that did, and refuses ``settlesAfter`` dials in a row with no
/// success in between, spread over ``settlesOver`` at least, has taken the
/// access away: ``Verdict/revoked``. Short of that is ``Verdict/passing``: a
/// refusal that can pass (an `authorized_keys` being rewritten, a home
/// directory whose permissions sshd's StrictModes refused for a moment), so
/// it's retried on the usual backoff, whose rungs are about two, four and
/// eight seconds: the fourth refusal, some fourteen seconds after the first,
/// is usually the one that settles it. The time floor is for the Mac, where
/// any of some thirty reads can dial, so three refusals can land within a
/// second of each other.
/// Any dial that gets in starts the count again. A failure that isn't a
/// refusal says nothing about the key and leaves the count as it is.
///
/// A device that's added back is let in on the next slow check, which
/// clears all of this; a revoked verdict is never final.
public struct KeyRefusals: Sendable, Equatable {
    /// How many refused dials in a row settle it.
    public static let settlesAfter = 3
    /// How long, at least, from the first refusal in a row to the one that
    /// settles it.
    public static let settlesOver: TimeInterval = 10

    /// What a refused dial means, given this device's history with the runner.
    public enum Verdict: Sendable, Equatable {
        /// The runner never let this device in: it isn't authorized yet.
        case stranger
        /// It did, and has refused fewer than ``settlesAfter`` dials since,
        /// or over less than ``settlesOver``: retried as a failure that can
        /// pass.
        case passing
        /// It did, and has refused ``settlesAfter`` dials in a row over
        /// ``settlesOver`` or more: this device's access was taken away.
        case revoked
    }

    /// Whether the runner has let this device in before.
    public private(set) var admitted: Bool
    /// Refused dials since the last one that got in.
    public private(set) var refusals = 0
    /// When the first of ``refusals`` came.
    public private(set) var firstRefusal: Date?
    /// When the latest did.
    public private(set) var lastRefusal: Date?

    public init(admitted: Bool) {
        self.admitted = admitted
    }

    /// A dial got in: the runner accepts this device.
    public mutating func letIn() {
        admitted = true
        released = false
        refusals = 0
        firstRefusal = nil
        lastRefusal = nil
    }

    /// The runner said it took this device's access away (ov-820): the
    /// `released` notice, its connection's last frame before the runner
    /// closed it. Kept across launches until a dial gets in.
    ///
    /// What a device that reaches the runner only through Tailcat has to go
    /// on: after the revoke, the tunnel ignores it in silence, so it is never
    /// refused, and every dial ends `no_answer`. Read by
    /// `RunnerTrouble.reading`, which takes any failure to reach the runner
    /// as ``Verdict/revoked`` while this holds. A device offline when it was
    /// revoked never hears it, and reads a runner that isn't answering.
    public private(set) var released = false

    /// The runner said so: remembered for `runner` in `defaults`.
    public mutating func release(_ runner: String, in defaults: UserDefaults = .standard) {
        released = true
        defaults.set(true, forKey: Self.releasedKey(runner))
    }

    static func releasedKey(_ runner: String) -> String { "keyRefusals.released.\(runner)" }

    /// A dial was refused: what that means now.
    public mutating func refused(at now: Date = Date()) -> Verdict {
        guard admitted else { return .stranger }
        refusals += 1
        if firstRefusal == nil { firstRefusal = now }
        lastRefusal = now
        return verdict
    }

    /// What the refusals so far mean, without counting another.
    public var verdict: Verdict {
        guard admitted else { return .stranger }
        guard refusals >= Self.settlesAfter, let firstRefusal, let lastRefusal,
            lastRefusal.timeIntervalSince(firstRefusal) >= Self.settlesOver
        else { return .passing }
        return .revoked
    }

    /// Whether ``letIn()`` was ever called for `runner`, from `defaults`.
    /// Kept across launches, because a device removed while the app was
    /// closed first dials the runner after its next launch.
    public static func remembered(_ runner: String, in defaults: UserDefaults = .standard) -> KeyRefusals {
        var refusals = KeyRefusals(admitted: defaults.bool(forKey: key(runner)))
        refusals.key = defaults.string(forKey: keyKey(runner))
        refusals.released = defaults.bool(forKey: releasedKey(runner))
        return refusals
    }

    /// ``letIn()``, and remembered for `runner` in `defaults`.
    public mutating func letIn(_ runner: String, in defaults: UserDefaults = .standard) {
        letIn()
        defaults.set(true, forKey: Self.key(runner))
        defaults.removeObject(forKey: Self.releasedKey(runner))
    }

    static func key(_ runner: String) -> String { "keyRefusals.admitted.\(runner)" }

    /// The key the runner said this device came in with (`host.get`'s
    /// `caller_key_fingerprint`, ov-818), as `SHA256:…`; nil until a runner
    /// that says so has been read. Kept across launches.
    public private(set) var key: String?

    /// The runner named the key this device came in with: remembered for
    /// `runner` in `defaults`, and written only when it's news.
    public mutating func learn(key: String, for runner: String, in defaults: UserDefaults = .standard) {
        guard !key.isEmpty, key != self.key else { return }
        self.key = key
        defaults.set(key, forKey: Self.keyKey(runner))
    }

    static func keyKey(_ runner: String) -> String { "keyRefusals.key.\(runner)" }

    /// Whether the stderr of a read that failed shows the agent, not the
    /// runner, in trouble: it wouldn't sign, didn't answer, or wasn't there.
    /// No probe then (ov-818 review 3): a probe would only ask the agent
    /// again, and an agent that prompts (Touch ID, 1Password, a
    /// confirm-each-use key) would put its dialog in front of the person.
    public static func showsAgentTrouble(_ stderr: String) -> Bool {
        let lower = stderr.lowercased()
        return agentTrouble.contains(where: lower.contains)
    }

    /// Whether the stderr of a read that failed shows the runner's host key
    /// changed or unverified: something to put to a person, never read as a
    /// release (ov-820 review S2), as `RunnerTrouble.meansTheRunnerIsGone`
    /// leaves out ``RunnerTrouble/hostKeyChanged`` on the phones.
    public static func showsHostKeyTrouble(_ stderr: String) -> Bool {
        let lower = stderr.lowercased()
        return hostKeyTrouble.contains(where: lower.contains)
    }

    static let hostKeyTrouble = ["remote host identification has changed", "host key verification failed"]

    static let agentTrouble = [
        "sign_and_send_pubkey", "agent refused operation", "communication with agent failed",
        "ssh_get_authentication_socket", "agent contains no identities",
    ]

    /// Whether a verbose (`ssh -v`) dial proves the runner itself refused
    /// `key`, the key it let this device in with (ov-818, reviews S1 and 1):
    /// the host key matched the known one under strict checking, that key was
    /// offered, and no key was accepted.
    ///
    /// Each other way to the same "Permission denied (publickey)" leaves a
    /// mark this refuses:
    /// - a key the server accepted and then never got a signature for: an
    ///   agent that refused or didn't answer (`Server accepts key`,
    ///   `sign_and_send_pubkey`), or a passphrase key with no agent;
    /// - a route through a proxy or a ProxyJump bastion, whose refusal may be
    ///   the bastion's;
    /// - a host key that didn't match, or that wasn't known at all;
    /// - an agent that wasn't there or held nothing, after which ssh offers
    ///   whatever key files it finds; and any offer that isn't `key`.
    ///
    /// The samples it's held to are real OpenSSH 10.3 output, in
    /// `test/fixtures/ssh-refusals`.
    public static func isProvenKeyRefusal(verbose: String, key: String) -> Bool {
        guard !key.isEmpty else { return false }
        let lower = verbose.lowercased()
        let disqualifying =
            agentTrouble + [
                "server accepts key", "executing proxy command", "setting implicit proxycommand",
            ] + hostKeyTrouble
        guard !disqualifying.contains(where: lower.contains) else { return false }
        let offeredTheKey = verbose.split(separator: "\n").contains {
            $0.contains("Offering public key") && $0.split(separator: " ").contains(Substring(key))
        }
        return offeredTheKey && lower.contains("is known and matches the") && isKeyRefusal(verbose)
    }

    /// When the Mac last made its verbose probe, so it makes one at most
    /// every ``probesEvery``: each is one more failed login on the runner,
    /// and OpenSSH's `PerSourcePenalties` turns a source away once 15 s of
    /// them have built up.
    public private(set) var probedAt: Date?
    public static let probesEvery: TimeInterval = 30

    /// Whether a probe may be made at `now`, and if so, that it's being made.
    public mutating func mayProbe(at now: Date) -> Bool {
        if let probedAt, now.timeIntervalSince(probedAt) < Self.probesEvery { return false }
        probedAt = now
        return true
    }

    /// Whether `stderr` from a command the Mac ran over `ssh` is sshd
    /// refusing this device's key: OpenSSH's "Permission denied (publickey)"
    /// or "(publickey,password)", the methods sshd offers in its order. The
    /// words are OpenSSH's own and have been for decades. A refusal that
    /// offers no public key method is not about this device's key.
    ///
    /// A candidate only: the agent, a passphrase or a bastion print the same
    /// line. ``isProvenKeyRefusal(verbose:)`` is what settles it.
    public static func isKeyRefusal(_ stderr: String) -> Bool {
        stderr.lowercased().split(separator: "\n").contains { line in
            guard let open = line.range(of: "permission denied (") else { return false }
            let methods = line[open.upperBound...].prefix { $0 != ")" }
            return methods.split(separator: ",").contains("publickey")
        }
    }
}
