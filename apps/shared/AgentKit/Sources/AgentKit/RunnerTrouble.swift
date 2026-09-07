import Foundation

/// What a failure to reach a runner MEANS, as opposed to what it says, and the
/// one thing worth doing about it.
///
/// `Connection.Failure` is a typealias for this, so there is one vocabulary and
/// not two. It moved out of `Connection` for the reason `AgentFailure` lives
/// here: the runner sends a machine word, the app owns the sentence, and the
/// sentence has to be somewhere a test can read it back — the iOS UI suite is
/// compiled by CI and never executed, so copy kept inside a `View` is copy that
/// nothing checks.
///
/// **Two screens draw this, which is why it is a table rather than a `switch`
/// in one of them.** `FleetView`'s full-screen failure phase is one runner and
/// nothing else; `RunnerStatusRow` is one runner among several. Two screens
/// offering different next moves about the same failure is the drift this
/// codebase keeps finding, and the only structural cure is that neither of them
/// decides.
///
/// A failure screen that offers the same button for every failure is a failure
/// screen that is wrong most of the time: "Try Again" fixes a host that was
/// asleep and fixes nothing at all about a key this device was never authorized
/// with, or a host key that changed underneath us. Each of these has exactly one
/// useful next move and they are not the same move.
///
/// Read off the message rather than a typed error because the message is all
/// that crosses the FFI boundary — the core hands back Rust's `Display` output
/// as a string and there is no code to switch on. The substrings are the ones in
/// `crates/client/src/ssh.rs` and `session.rs`; each is a distinctive phrase
/// from the middle of its message rather than a prefix, so wrapping the error in
/// more context does not stop it matching.
enum RunnerTrouble {
    /// The host answered but does not know this device's key.
    /// `SshError::AuthRejected` — fixed by authorizing, not by retrying.
    case keyRejected
    /// The key presented is not the one we pinned. `SshError::HostKeyChanged`.
    /// Retrying is guaranteed to fail, and offering it would suggest this is
    /// a glitch rather than a decision someone has to make.
    case hostKeyChanged
    /// Nothing answered: wrong address, host asleep, off the network.
    /// `SshError::Connect` — the one case where retrying is the right move.
    case unreachable
    /// SSH worked; Far Cooler is not installed over there.
    /// `SessionError::DaemonMissing`.
    case daemonMissing
    /// This device has no usable key, so no host will ever accept it.
    case noIdentity
    /// This runner is reached through the tunnel and this device holds no
    /// node key. Its own kind rather than ``noIdentity``, because the
    /// remedy is different: an SSH key this app can generate for itself,
    /// and a node key it cannot — the public half is a line in a runner's
    /// allowlist, written by the device that granted the runner, so the
    /// way to a working one is the ceremony again.
    case noNodeKey
    /// The user was shown a fingerprint and did not say yes. Not a fault at
    /// all — a decision that has been deferred — and the way back is the
    /// same screen again, not a retry that pretends something broke.
    case keyNotTrusted
    /// The user stopped waiting. Also not a fault, and it must not be
    /// headlined as one.
    case stopped
    case other

    init(message: String) {
        if message.contains("rejected this key") { self = .keyRejected }
        else if message.contains("is not the one Far Cooler has recorded") {
            self = .hostKeyChanged
        } else if message.contains("cannot reach") { self = .unreachable }
        else if message.contains("did not answer") { self = .daemonMissing }
        else if message.contains("no SSH key") { self = .noIdentity }
        else if message.contains("no tunnel key") { self = .noNodeKey }
        else if message.contains("has not been trusted") { self = .keyNotTrusted }
        else if message.contains("Stopped waiting") { self = .stopped }
        else { self = .other }
    }

    /// The one action that fits what happened.
    ///
    /// A value rather than a view, because the two screens that draw it build
    /// different controls from the same decision: the full-screen phase makes
    /// the first three a `NavigationLink` into `AuthorizeView` and the row
    /// hands them back to whoever placed it. What must not differ is WHICH of
    /// these a given failure gets.
    enum NextMove: Equatable {
        /// The public key, and the one line to paste on the machine.
        case authorizeThisDevice
        case reviewTheNewKey
        /// Straight back to the fingerprint. Deliberately not "Try Again":
        /// nothing failed, the question is simply still open.
        case showTheKeyAgain
        /// The same road `authorizeThisDevice` takes, because this device asks
        /// to be added again and the offer it shows carries a node key a runner
        /// can admit.
        case addThisDeviceAgain
        case tryAgain

        /// Title case, as every button in this app is.
        var label: String {
            switch self {
            case .authorizeThisDevice: return "Authorize This Device"
            case .reviewTheNewKey: return "Review the New Key"
            case .showTheKeyAgain: return "Show the Key Again"
            case .addThisDeviceAgain: return "Add This Device Again"
            case .tryAgain: return "Try Again"
            }
        }
    }

    var nextMove: NextMove {
        switch self {
        case .keyRejected: return .authorizeThisDevice
        case .hostKeyChanged: return .reviewTheNewKey
        case .keyNotTrusted: return .showTheKeyAgain
        // Deliberately not "Try Again": the dial would use the key that is
        // missing, so the button could only fail, every time, forever.
        case .noNodeKey: return .addThisDeviceAgain
        case .unreachable, .daemonMissing, .noIdentity, .stopped, .other: return .tryAgain
        }
    }

    /// Whether "Try Again" belongs BELOW the primary action as a second
    /// option. False where retrying is already the primary action (it would
    /// then appear twice) and false where it cannot work at all.
    var worthRetryingAsAlternative: Bool {
        switch self {
        case .keyRejected: return true
        case .hostKeyChanged, .keyNotTrusted: return false
        case .unreachable, .daemonMissing, .noIdentity, .noNodeKey, .stopped, .other:
            return false
        }
    }

    /// Whether to offer correcting this runner's details.
    ///
    /// Every failure but one. A device with no key of its own has no runner to
    /// blame and nothing in a runner editor would change the answer.
    var offersEditingTheRunner: Bool { self != .noIdentity }

    /// Whether the raw text from the core goes on screen under the sentence.
    ///
    /// Only where the app has no diagnosis of its own — the same scoping the
    /// Mac's `ChangesPane` uses, and for the same reason: a transcript under a
    /// sentence that already names the cause and the fix is noise. Nothing is
    /// discarded. For a runner nobody can reach, that text is the only
    /// diagnosis that exists and somebody debugging one needs it; it just goes
    /// where output goes rather than where prose does.
    var showsTheRunnersOwnWords: Bool { self == .other }

    /// Whether this is the one failure that is genuinely alarming.
    ///
    /// Every kind used to be red, `daemonMissing` included — which is not a
    /// failure at all but a runner nobody has run `host install` on — and so
    /// were `keyNotTrusted` and `stopped`, both of which this file calls not a
    /// fault. Red on a step somebody simply has not taken yet shouts about the
    /// wrong thing, and a color spent on everything says nothing about the one
    /// case that warrants it: a host key that changed underneath us.
    var isAlarming: Bool { self == .hostKeyChanged }

    var symbol: String {
        switch self {
        case .keyRejected, .noIdentity, .noNodeKey: return "key.slash"
        case .hostKeyChanged: return "exclamationmark.shield"
        case .keyNotTrusted: return "key"
        case .unreachable: return "network.slash"
        case .daemonMissing: return "square.and.arrow.down"
        case .stopped: return "clock"
        case .other: return "exclamationmark.triangle"
        }
    }

    /// What a sentence about this runner needs to name it.
    ///
    /// Three strings rather than the runner itself, because `Runner` and
    /// `Reach` are declared in the iOS target and this package cannot see
    /// them. `port` nil means the tunnel, which is the whole of the
    /// distinction the copy below draws: a tunnel is reached by token, so
    /// there is no port to name and no address to have got wrong.
    struct Words {
        /// `Runner.named` — an address for a direct runner, the label for a
        /// tunneled one.
        var name: String
        /// `Reach.detail(user:)` — the second line under a runner's name.
        var reachDetail: String
        /// The port a direct runner answers on. Nil for a tunneled runner.
        var port: Int?

        init(name: String, reachDetail: String, port: Int?) {
            self.name = name
            self.reachDetail = reachDetail
            self.port = port
        }
    }

    func headline(_ words: Words) -> String {
        switch self {
        case .keyRejected: return "Not Authorized Yet"
        case .hostKeyChanged: return "This Host’s Key Changed"
        case .unreachable: return "Can’t Reach \(words.name)"
        case .daemonMissing: return "Far Cooler Isn’t Installed"
        case .noIdentity: return "This Device Has No Key"
        case .noNodeKey: return "This Device Has No Tunnel Key"
        case .keyNotTrusted: return "Key Not Trusted"
        case .stopped: return "Stopped Waiting"
        case .other: return "Can’t Connect"
        }
    }

    /// The sentence under the headline.
    ///
    /// Ours wherever we know what happened, the core's own text only where we
    /// do not. The raw string crossing up from Rust is written for whoever is
    /// reading a log — lowercase, and ending in things like "(os error 61)" —
    /// and putting that in front of someone who just wants their runner back is
    /// asking them to translate. Two cases keep it deliberately: the changed
    /// host key, whose message carries the two fingerprints being compared and
    /// must not be paraphrased, and the unclassified failure, where the core's
    /// account is the only account there is.
    ///
    /// Deliberately does not repeat the headline. The address is already up
    /// there in most of these, and "Not Authorized Yet" over "…doesn't have
    /// this device's key yet" said "yet" twice in two lines.
    func detail(message: String, words: Words) -> String {
        switch self {
        case .keyRejected:
            return "\(words.reachDetail) hasn’t been given this device’s key."
        case .unreachable:
            guard let port = words.port else {
                // No port to name and no address to have got wrong: a tunnel is
                // reached by token, so the two things a person could check are
                // whether the runner is awake and whether it is on the tunnel.
                return "The tunnel didn’t reach it. The runner may be asleep, or off the tunnel."
            }
            return
                "Nothing answered on port \(port). The runner may be asleep, "
                + "or the address may be wrong."
        case .daemonMissing:
            return "SSH connected, but the Far Cooler daemon didn’t answer. Install it there."
        // Sentences somebody wrote, each naming both what happened and what to
        // do about it — three of them in `Connection`, `hostKeyChanged` in
        // `crates/client/src/ssh.rs`. They are the core's words only in the
        // sense that the core is where they are stored.
        case .hostKeyChanged, .noIdentity, .noNodeKey, .keyNotTrusted, .stopped:
            return message
        // The undiagnosed arm, and the only one where `message` is whatever
        // came back rather than something written to be read. Those words go
        // into a `DetailBox` instead of standing here as the app's own account
        // of the runner.
        //
        // No cause named, deliberately: from this side the cause is unknowable,
        // and a guess sends somebody to loosen an sshd setting that was never
        // the problem. Nor any retry promised — whether one is under way is the
        // connection's business, and the button below is the only offer a
        // screen makes.
        case .other:
            return "The attempt to reach it didn’t finish."
        }
    }
}
