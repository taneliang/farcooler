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
/// (``refusals``). A refusal comes after the runner proved its host key
/// against the one this device pinned, so it's the runner's own answer, not
/// a stranger's.
///
/// **What it tells a stranger: nothing.** The runner says nothing new. sshd
/// refuses a revoked key in exactly the words it refuses any key it has
/// never seen, so nobody dialing a runner can learn from it whether a key
/// was ever there. The device draws its conclusion from what it remembers.
///
/// **Revoked against transient.** A runner that never let this device in is a
/// stranger's refusal: ``Verdict/stranger``, and nothing changes from before.
/// A runner that did, and refuses ``settlesAfter`` dials in a row with no
/// success in between, has taken the access away: ``Verdict/revoked``. Fewer
/// than that is ``Verdict/passing``: a refusal that can pass (an
/// `authorized_keys` being rewritten, a home directory whose permissions
/// sshd's StrictModes refused for a moment), so it's retried on the usual
/// backoff, whose first three rungs are about two, four and eight seconds.
/// Any dial that gets in starts the count again. A failure that isn't a
/// refusal says nothing about the key and leaves the count as it is.
///
/// A device that's added back is let in on the next slow check, which
/// clears all of this; a revoked verdict is never final.
public struct KeyRefusals: Sendable, Equatable {
    /// How many refused dials in a row settle it.
    public static let settlesAfter = 3

    /// What a refused dial means, given this device's history with the runner.
    public enum Verdict: Sendable, Equatable {
        /// The runner never let this device in: it isn't authorized yet.
        case stranger
        /// It did, and has refused fewer than ``settlesAfter`` dials since:
        /// retried as a failure that can pass.
        case passing
        /// It did, and has refused ``settlesAfter`` dials in a row: this
        /// device's access was taken away.
        case revoked
    }

    /// Whether the runner has let this device in before.
    public private(set) var admitted: Bool
    /// Refused dials since the last one that got in.
    public private(set) var refusals = 0

    public init(admitted: Bool) {
        self.admitted = admitted
    }

    /// A dial got in: the runner accepts this device.
    public mutating func letIn() {
        admitted = true
        refusals = 0
    }

    /// A dial was refused: what that means now.
    public mutating func refused() -> Verdict {
        guard admitted else { return .stranger }
        refusals += 1
        return verdict
    }

    /// What the refusals so far mean, without counting another.
    public var verdict: Verdict {
        guard admitted else { return .stranger }
        return refusals >= Self.settlesAfter ? .revoked : .passing
    }

    /// Whether ``letIn()`` was ever called for `runner`, from `defaults`.
    /// Kept across launches, because a device removed while the app was
    /// closed first dials the runner after its next launch.
    public static func remembered(_ runner: String, in defaults: UserDefaults = .standard) -> KeyRefusals {
        KeyRefusals(admitted: defaults.bool(forKey: key(runner)))
    }

    /// ``letIn()``, and remembered for `runner` in `defaults`.
    public mutating func letIn(_ runner: String, in defaults: UserDefaults = .standard) {
        letIn()
        defaults.set(true, forKey: Self.key(runner))
    }

    static func key(_ runner: String) -> String { "keyRefusals.admitted.\(runner)" }

    /// Whether `stderr` from a command the Mac ran over `ssh` is sshd
    /// refusing this device's key: OpenSSH's "Permission denied (publickey)"
    /// or "(publickey,password)", the methods sshd offers in its order. The
    /// words are OpenSSH's own and have been for decades. A refusal that
    /// offers no public key method is not about this device's key.
    public static func isKeyRefusal(_ stderr: String) -> Bool {
        stderr.lowercased().split(separator: "\n").contains { line in
            guard let open = line.range(of: "permission denied (") else { return false }
            let methods = line[open.upperBound...].prefix { $0 != ")" }
            return methods.split(separator: ",").contains("publickey")
        }
    }
}
