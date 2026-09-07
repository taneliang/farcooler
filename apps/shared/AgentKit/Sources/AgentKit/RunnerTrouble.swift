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
/// that crosses the FFI boundary on the connect path — the core hands back
/// Rust's `Display` output as a string, with nothing beside it. The substrings
/// are the ones in `crates/client/src/ssh.rs` and `session.rs`; each is a
/// distinctive phrase
/// from the middle of its message rather than a prefix, so wrapping the error in
/// more context does not stop it matching.
///
/// **One of them is not a phrase, and must not be read as one.**
/// `SshError::Tunnel` renders as `cannot open the tunnel: <word>`, and the word
/// is `farcooler_tailcat::TunnelError::code` — a stable machine word that
/// already crosses the FFI on purpose, wrapped in a sentence for a log. So the
/// tunnel failures are classified by READING THAT WORD (``TunnelWord``), never
/// by matching the English around it. That prose can be reworded; the word is
/// the thing whose own doc promises it will not be.
///
/// **The FFI is deliberately not widened to carry a code beside the message
/// here.** `push_call` already does exactly that for a call on a live session,
/// and `ClientCore.CoreError` states why the connect path is the exception: a
/// connect failure genuinely arrives as prose, because two of these have
/// nothing but the message to show — a changed host key, whose text carries the
/// two fingerprints being compared and must not be paraphrased, and the
/// undiagnosed failure, where the core's account is the only account there is.
/// A code cannot replace those. What a code can do is stop a code being
/// recovered from the sentence printed around it, and that is the whole of the
/// change this seam needed.
enum RunnerTrouble: Equatable {
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
    /// The tunnel never opened, named by the stable word the core sent.
    /// `SshError::Tunnel` — see ``TunnelWord``.
    ///
    /// Its own kind rather than ``other``, and this is the bug that made it
    /// one: `tunnel_error` renders everything but a refused port as
    /// `cannot open the tunnel: <word>`, which matches no phrase below, so a
    /// revoked device fell through to ``other`` and read
    /// `cannot open the tunnel: no_answer` off its own screen — a raw machine
    /// word in front of a person, in the one situation where a clear sentence
    /// matters most.
    case tunnelFailed(TunnelWord)
    case other

    /// The four stable words `farcooler_tailcat::TunnelError::code` sends, and
    /// nothing else.
    ///
    /// These are not English and they are not a message: they are the machine
    /// words the tunnel crate documents as "the stable word that crosses the
    /// FFI. The apps own the sentence a person reads." This type is this app
    /// owning them. `crates/cli/src/runner_pipe.rs`'s `sentence` is the same
    /// table for the CLI, and the two say the same things in the two registers
    /// their readers are in — one dialect, two readers.
    ///
    /// A word this build has never seen becomes ``unspecified`` rather than
    /// nothing, so a fifth word added in Rust reaches a screen as a sentence
    /// somebody wrote instead of as itself. That is the same rule
    /// `farcooler_core::error::word_for` follows for a code it has not seen.
    enum TunnelWord: String, Sendable, Equatable, Hashable, CaseIterable {
        /// Tailcat ignores a client it does not recognize SILENTLY, so a
        /// device removed from a runner's allowlist gets no refusal — it gets
        /// a timeout. This is that timeout, and it is also what a runner that
        /// is simply asleep looks like. The sentence has to say both.
        case noAnswer = "no_answer"
        /// The rendezvous service that introduces this device to the runner
        /// could not be reached. That is this device's own network, not the
        /// runner's.
        case rendezvous = "derp"
        /// This build links no tunnel at all. On iOS that is the Simulator:
        /// `build-ios-frameworks.sh` links the archive into the device slice
        /// only. A device build always has one.
        case notInThisBuild = "no_tailcat"
        /// Deliberately generic upstream — a malformed token, a dead sshd
        /// whose errno differs by platform, and `EMFILE` all wear it — so the
        /// sentence claims nothing about which. Also where an unrecognized
        /// word lands.
        case unspecified = "io"

        /// What `SshError::Tunnel`'s `Display` puts in front of the word.
        ///
        /// The one string in this file that has to match Rust exactly.
        /// `crates/client/src/ssh.rs`'s
        /// `the_tunnel_message_carries_the_word_the_apps_read` is the other
        /// half of that pair, and it names this file — because a reword on
        /// either side is silent everywhere else.
        static let marker = "cannot open the tunnel: "

        /// The word inside a tunnel failure's message, or nil if this is not
        /// one.
        ///
        /// Looked for anywhere in the message rather than at the front, for
        /// the reason every phrase below is: wrapping the error in more
        /// context must not stop it matching. The word runs to the first
        /// space or the end, so trailing context does not become part of it.
        static func inside(_ message: String) -> TunnelWord? {
            guard let start = message.range(of: marker)?.upperBound else { return nil }
            let word = message[start...].prefix { !$0.isWhitespace }
            // Never nil past this point: an unknown word is a sentence this
            // app wrote, never the word itself on a screen.
            return TunnelWord(rawValue: String(word)) ?? .unspecified
        }
    }

    /// The sentences the APP writes, as opposed to the ones the core sends.
    ///
    /// Four of the ten kinds above are diagnosed by matching a phrase in a
    /// message this app composed itself, which is a round trip with a seam in
    /// the middle: reword the sentence in `Connection` and the classifier
    /// quietly stops matching it, so a decision the user made turns into
    /// `.other` — "Can't Connect", the core's own words, and "Try Again" for a
    /// question nobody answered.
    ///
    /// Nothing about that failure is visible in a diff or a build. So the
    /// sentence and the phrase that recognizes it live in one file, and
    /// `RunnerTroubleTests` reads each one back through `init(message:)`. That
    /// test is the only enforcement this rule has, and it is the reason these
    /// are here rather than beside the code that raises them.
    enum Said {
        /// The user was shown a fingerprint and backed out of the question.
        /// Matches on "has not been trusted".
        static func declined(runner: String) -> String {
            "The key \(runner) presented has not been trusted on this device. "
                + "Far Cooler won’t connect until it is."
        }

        /// The user stopped waiting out a dial. Matches on "Stopped waiting".
        static func stoppedWaiting(for runner: String) -> String {
            "Stopped waiting for \(runner). It may be asleep or off the network."
        }

        /// No SSH key, and none could be made. Matches on "no SSH key".
        static let noIdentity = "This device has no SSH key and one could not be generated."

        /// A tunneled runner and no node key. Matches on "no tunnel key".
        ///
        /// **The dial does not mint one.** A tunneled runner was granted against
        /// ONE public half, which is now a line in that runner's allowlist;
        /// minting a fresh pair would produce a key nobody has authorized, and
        /// tailcat ignores a client it does not recognize without answering — so
        /// the symptom would be a spinner, then a timeout, and nothing anywhere
        /// saying why.
        static let noNodeKey =
            "This runner is reached through the tunnel, and this device has no tunnel key. "
            + "Add this device again to get one."
    }

    init(message: String) {
        // First, and by the machine word rather than by a phrase. See the
        // header: the word is what the core promises to keep stable, and the
        // sentence printed around it is not.
        if let word = TunnelWord.inside(message) { self = .tunnelFailed(word) }
        else if message.contains("rejected this key") { self = .keyRejected }
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

        /// What tapping this move has to DO, in order, and all of it.
        ///
        /// The label is the intent; this is the mechanism. Here rather than in
        /// a `switch` inside a view for the table's own reason — two screens
        /// draw these buttons and neither of them may decide — and for a
        /// second reason this file did not have before: **one of these answers
        /// is not the obvious one, and the obvious one is a button that does
        /// nothing.**
        ///
        /// Forgetting a runner's pinned key is not, by itself, a reconnect. It
        /// works out to one only because clearing the fingerprint CHANGES the
        /// runner, and `FleetMembership.plan` rebuilds a connection whose
        /// details no longer match the ones it was dialed with. So:
        ///
        /// - `reviewTheNewKey` follows a host key that changed underneath us. A
        ///   fingerprint really is pinned there, clearing it is a change, and
        ///   the rebuild fires on its own.
        /// - `showTheKeyAgain` follows a fingerprint question nobody answered.
        ///   Nothing was ever pinned, so the fingerprint is ALREADY nil,
        ///   clearing it changes nothing at all, `plan` files the runner under
        ///   `kept` and leaves it completely alone. Without an explicit dial
        ///   this move is a button that cannot work, which is the one thing
        ///   `nextMove`'s own doc says a failure screen must never offer.
        var acts: [Act] {
            switch self {
            case .authorizeThisDevice, .addThisDeviceAgain: return [.offerThisDevicesKey]
            case .reviewTheNewKey: return [.forgetThePinnedKey]
            case .showTheKeyAgain: return [.forgetThePinnedKey, .dialAgain]
            case .tryAgain: return [.dialAgain]
            }
        }
    }

    /// The mechanisms a move is built out of.
    ///
    /// Three, and deliberately no more: a screen that needed a fourth would be
    /// a screen deciding something, which is what this file is for.
    enum Act: Sendable, Equatable, Hashable {
        /// Show this device's public key and the line to paste on the machine.
        case offerThisDevicesKey
        /// Clear the pinned fingerprint, so the next dial asks about the key
        /// instead of refusing it.
        case forgetThePinnedKey
        /// Dial this runner again, from the beginning.
        case dialAgain
    }

    var nextMove: NextMove {
        switch self {
        case .keyRejected: return .authorizeThisDevice
        case .hostKeyChanged: return .reviewTheNewKey
        case .keyNotTrusted: return .showTheKeyAgain
        // Deliberately not "Try Again": the dial would use the key that is
        // missing, so the button could only fail, every time, forever.
        case .noNodeKey: return .addThisDeviceAgain
        // Including every tunnel word. A tunnel that did not open is the
        // tunnel's version of a runner nobody could reach, and dialing again
        // is what fixes a runner that was asleep. `notInThisBuild` is the
        // exception on paper — no dial changes which archive a build links —
        // but it is the Simulator's answer and never a shipped device's, so
        // the alternative would be a button nobody will ever tap either way.
        // What it must not do is retry on a SCHEDULE; see `retry`.
        case .unreachable, .daemonMissing, .noIdentity, .stopped, .other, .tunnelFailed:
            return .tryAgain
        }
    }

    /// Whether to dial again without being asked, and how soon.
    ///
    /// Here rather than in `Connection`'s reconnect for this file's own reason,
    /// and it took a tunnel failure to make the reason bite: the schedule is a
    /// decision about what a failure MEANS, the iOS UI suite is compiled by CI
    /// and never executed, and a `switch` in the app target is a decision
    /// nothing reads back. `RunnerTroubleTests` reads this one.
    enum Retry: Sendable, Equatable, Hashable {
        /// Nothing is scheduled. The failure needs a person — a key to
        /// authorize, a fingerprint to answer, a build that has a tunnel in it
        /// — and a spinner returning every thirty seconds says the opposite.
        case never
        /// Five minutes, at the same rung. No amount of retrying installs a
        /// daemon or wakes a build.
        case afterAWhile
        /// The exponential schedule, one rung up. For the failures that are
        /// genuinely transient often enough to be worth chasing.
        case onTheBackoff
    }

    var retry: Retry {
        switch self {
        case .keyRejected, .hostKeyChanged, .noIdentity, .noNodeKey, .keyNotTrusted:
            return .never
        // A dial cannot put the Go archive into a build that was linked
        // without one, so the schedule would be a timeout every thirty
        // seconds, forever, for an answer that cannot change.
        case .tunnelFailed(.notInThisBuild):
            return .never
        case .daemonMissing:
            return .afterAWhile
        case .unreachable, .stopped, .other, .tunnelFailed:
            return .onTheBackoff
        }
    }

    /// Whether "Try Again" belongs BELOW the primary action as a second
    /// option. False where retrying is already the primary action (it would
    /// then appear twice) and false where it cannot work at all.
    var worthRetryingAsAlternative: Bool {
        switch self {
        case .keyRejected: return true
        case .hostKeyChanged, .keyNotTrusted: return false
        case .unreachable, .daemonMissing, .noIdentity, .noNodeKey, .stopped, .other,
            .tunnelFailed:
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
        // A build with no tunnel in it is not a network problem and must not
        // wear the network's mark: nothing about this device's connection
        // changes the answer.
        case .tunnelFailed(.notInThisBuild): return "exclamationmark.triangle"
        case .tunnelFailed: return "network.slash"
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
        // Named the same way ``unreachable`` names it, because it is the same
        // fact about the same runner: nothing answered. `Runner.named` is
        // what makes that a label rather than the empty address a tunneled
        // runner has.
        case .tunnelFailed(.noAnswer): return "Can’t Reach \(words.name)"
        // Not the runner's name: what could not be reached is the rendezvous,
        // and blaming the runner would send somebody to go and wake a machine
        // that was awake the whole time.
        case .tunnelFailed(.rendezvous): return "Can’t Reach the Tunnel"
        case .tunnelFailed(.notInThisBuild): return "No Tunnel in This Build"
        case .tunnelFailed(.unspecified): return "The Tunnel Didn’t Open"
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
        // The app's own sentences for the tunnel's four stable words. Never
        // `message`: `message` is `cannot open the tunnel: <word>`, and the
        // word is the thing this table exists to keep off a screen.
        //
        // `crates/cli/src/runner_pipe.rs`'s `sentence` says the same four
        // things to whoever is reading a terminal. Reword one and the other
        // is where to look.
        case .tunnelFailed(.noAnswer):
            // Both causes, because from here they are indistinguishable — a
            // revoked device is ignored silently and times out exactly as a
            // sleeping runner does — and naming only one would send half the
            // people who read this to the wrong place.
            return
                "It didn’t answer. The runner may be asleep, or this device’s access "
                + "to it may have been revoked."
        case .tunnelFailed(.rendezvous):
            return
                "The service that introduces this device to the runner didn’t answer. "
                + "Check this device’s own network."
        case .tunnelFailed(.notInThisBuild):
            return "This build of Far Cooler has no tunnel it can dial."
        // No cause named, for ``other``'s reason: `io` is deliberately generic
        // upstream, so a guess here would send somebody to fix something that
        // was never the problem.
        case .tunnelFailed(.unspecified):
            return "The tunnel couldn’t be opened."
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

/// The fingerprint question, and every answer it has.
///
/// Not a `RunnerTrouble`. Nothing failed — a runner presented a key this device
/// has never seen, which is the ordinary first contact — and it is deliberately
/// not headlined as though something had. It is in this file because of what
/// one of its answers PRODUCES: declining lands on `RunnerTrouble.keyNotTrusted`,
/// whose one useful move is back to this same question, and the two halves of
/// that loop drifting apart is the drift this file exists to stop.
///
/// **This list is a recorded ruling, and the multi-runner port lost one of
/// them.** `RunnerStatusRow` replaced a full-screen approval phase that offered
/// "Trust This Runner" AND "Not Now"; the row shipped with "Trust This Runner"
/// and "Edit…", so the only way out of a fingerprint somebody was not sure
/// about became agreeing to it or force-quitting. The row's own header asserts
/// that every distinct next move survived, which is what makes this the
/// expensive kind of defect: the code and its comment disagreed, and nothing
/// could go red about it because the row lives in the iOS target, which CI
/// compiles and never runs.
///
/// So the answers are a list here rather than three `Button`s in a view, and
/// the view iterates it. Dropping one is now an edit to this file, in front of
/// `HostKeyQuestionTests`.
enum HostKeyQuestion {
    /// In the order they are offered. Trusting first because it is the answer;
    /// the other two are the ways out, and a way out that came first would read
    /// as the recommendation.
    enum Answer: Sendable, Equatable, Hashable, CaseIterable {
        /// Record the fingerprint on screen, which is also the connect. See
        /// `RunnerStore.trust`.
        case trustThisRunner
        /// Back out without answering. **Not a synonym for "Edit…".** Somebody
        /// who does not recognize a fingerprint has nothing to correct — the
        /// address is right, that is the point — and leaving them only a
        /// destructive edit and an agreement is leaving them the agreement.
        case notNow
        /// Correct this runner's details, for the case where the address really
        /// is wrong and the key on screen belongs to somebody else's machine.
        case editTheRunner

        /// Title case, as every button in this app is.
        var label: String {
            switch self {
            case .trustThisRunner: return "Trust This Runner"
            case .notNow: return "Not Now"
            case .editTheRunner: return "Edit…"
            }
        }

        /// Whether this is the answer, as opposed to a way out of the question.
        ///
        /// One of them, and the screens draw it with the weight — the two ways
        /// out are alternatives, and giving all three the same emphasis gives a
        /// person three things to weigh when only one of them is the answer.
        var isTheAnswer: Bool { self == .trustThisRunner }
    }

    /// Backing out leaves this runner in this state.
    ///
    /// Named here so the loop is closed in one place: `Said.declined` is the
    /// sentence, `RunnerTrouble.keyNotTrusted` is what it classifies to, and
    /// `showTheKeyAgain` is the move back. `HostKeyQuestionTests` walks the
    /// whole circle, because every link in it is a string match or a table and
    /// none of them fails loudly on its own.
    static let declining = RunnerTrouble.keyNotTrusted
}
