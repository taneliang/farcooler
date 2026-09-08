import Foundation

/// Whether this device can offer a tunnel key at all, and the one sentence a
/// person is owed when it cannot.
///
/// # Why this exists
///
/// A device with no tunnel key enrolls perfectly well. `CeremonyStore.showOffer`
/// sends an offer with no node key in it, `enrollment.rs` sees an empty one and
/// declines to route a tunnel, `Enrollment.token(in:)` reads the empty
/// `conn_blob` as "no tunnel" and keeps the address, and the runner that comes
/// out is a working, direct runner. **Every one of those four is right**, and
/// none of them is changed by anything here.
///
/// What none of them does is SAY so. The owner removed a healthy Mac runner,
/// added it again, got a direct runner again, and there was no screen anywhere
/// in the product — and no line in a shipping build's logs — that named the
/// reason: this phone's mint answered an error and the answer was dropped on
/// the floor. `grep -c -- "--node-key" ~/.ssh/authorized_keys` was `0` across
/// every enrolled device and nothing had ever reported it.
///
/// So this is a FACT, not a failure. Nothing here refuses anything, nothing
/// here retries, and nothing here offers a fix — there is no fix a phone can
/// perform, which is exactly why inventing a button would be worse than saying
/// nothing.
///
/// # Why it is in AgentKit
///
/// The same reason ``RunnerRefusal`` and ``CeremonyReach`` are: the iOS target
/// has no unit-test target and CI's iOS job stops at `build-for-testing`, so a
/// rule written into a `View` body is a rule with nothing on it. `swift test
/// --package-path apps/shared/AgentKit` runs on every push. The word-reading,
/// the sentences and the decision about whether there is anything to show are
/// all here; the phone's screen only draws the string this hands it.
///
/// # Why it is `internal`
///
/// `RendezvousSection` is `public` and is drawn on macOS as well as iOS, and
/// the Mac reads `Account.derpMap` nowhere at all — every reader is in
/// `apps/ios/`. This notice sits in the same neighborhood and would inherit
/// exactly that trap if it were shareable. It is not: the phone compiles
/// AgentKit's sources into its own module and sees `internal`, while
/// `apps/macos` imports AgentKit as a real module and cannot. The Mac has no
/// `NodeIdentity` either — its node key belongs to `RunnerStore` in
/// `crates/cli/src/runner_pipe.rs`, on the Rust side — so there is no value
/// here it could honestly construct. Being unable to draw this on the Mac is
/// the point, and it is enforced by the access level rather than by a comment.
enum NodeKeyStatus: Equatable, Sendable {
    /// This device holds a pair and offers its public half. Nothing to say.
    case held
    /// Minting refused, carrying the stable word it refused with.
    case notMinted(RunnerTrouble.TunnelWord)
    /// Minting produced a pair and the Keychain would not keep it.
    ///
    /// A genuinely different fault from ``notMinted``, and worse.
    /// `NodeIdentity.mintIfNeeded` hands the pair back whether or not the write
    /// landed, so an offer built after this DOES carry a public half: the
    /// runner is granted a tunnel, its allowlist gets a line, and the reach is
    /// `tailcat`. Only the dial fails, because `storedPrivateKey` reads the
    /// Keychain and finds nothing. Worse still, the next mint has nothing to
    /// re-read, so a second runner is granted a DIFFERENT key. Folding this in
    /// with "no key was ever made" would send whoever is diagnosing it to the
    /// tunnel archive, which is not where the fault is.
    ///
    /// The payload is the `OSStatus` the Keychain answered, which is also what
    /// `NodeIdentity.write(_:)` records under `nodeKeychainWriteStatus` and
    /// what `TunnelE2EHarness` prints. `Int32` rather than `OSStatus` so this
    /// file needs no `Security` import and AgentKit stays portable.
    case notStored(status: Int32)

    /// What a mint, plus whatever the platform's key store said about keeping
    /// it, comes to.
    ///
    /// The three-way decision, here rather than in `NodeIdentity` for the
    /// reason everything else in this file is here: `apps/ios` has no unit-test
    /// target. `NodeIdentity.status()` keeps only the two effects — reading the
    /// Keychain and writing it — and `store` is that write, handed in as a
    /// closure so this stays a rule and not a second Keychain client.
    ///
    /// `store` is called ONLY for a pair, and exactly once. A refused mint has
    /// nothing to store, and calling it anyway would delete and re-add a
    /// Keychain item on every settings screen for no reason.
    ///
    /// **The pair is passed to `store` and goes nowhere else.** No branch here
    /// puts either half into a value that has a sentence — see
    /// `noKeyMaterialCanReachASentence`, which is the guard on that.
    static func after(_ mint: NodeKeyMint, store: (String, String) -> Int32) -> NodeKeyStatus {
        switch mint {
        case .refused(let word):
            return .notMinted(word)
        case .pair(let priv, let pub):
            let status = store(priv, pub)
            return status == kept ? .held : .notStored(status: status)
        }
    }

    /// `errSecSuccess`, spelled as its value so this file needs no `Security`
    /// import and AgentKit compiles for every platform its targets use.
    private static let kept: Int32 = 0

    /// What Far Cooler says about it, or nil when there is nothing to say.
    ///
    /// **Nil for ``held`` is the whole decision about whether to show this**,
    /// made here rather than in a `View` body so a test can hold it. A device
    /// whose tunnel key is fine must show nothing at all: a settings screen
    /// that reports a working thing every time it is opened teaches people to
    /// scroll past the one time it reports a broken one.
    ///
    /// Each sentence is a fact and its consequence, and stops there. It does
    /// not instruct, and it does not offer a repair, which is the restraint
    /// ``RendezvousSection``'s header asks of everything in its neighborhood —
    /// "someone talked through changing this by a caller claiming to be support
    /// has been phished, not helped". The one sentence that could tempt a
    /// remedy is ``notStored``, and the remedy would be a lie: nothing on this
    /// screen can make the Keychain accept an item it refused.
    ///
    /// No machine word is ever printed and no Rust error string is ever
    /// carried. The word crosses the FFI; the sentence is written here. The
    /// only number that reaches a screen is Apple's own `OSStatus`, which is
    /// the single thing that makes a Keychain refusal searchable, and which no
    /// other build the owner can run would ever show them —
    /// `TunnelE2EHarness`, the one tool that prints it, is `#if DEBUG` and
    /// unreachable from TestFlight.
    var sentence: String? {
        switch self {
        case .held:
            nil
        case .notMinted(let word):
            Self.noKey + " " + Self.because(word)
        case .notStored(let status):
            "This device made a tunnel key and couldn’t save it, so runners you add through "
                + "the tunnel won’t connect. The Keychain wouldn’t keep it (error \(status))."
        }
    }

    /// The fact and its consequence, which is the same in every case where no
    /// key exists — one string rather than three copies that drift.
    private static let noKey =
        "This device has no tunnel key, so runners you add are reached by their address "
        + "rather than through the tunnel."

    /// The cause, in the app's own words.
    ///
    /// **Only two of the four words can arrive here, and the other two get no
    /// sentence of their own.** Minting reaches no network:
    /// `crates/tailcat/src/linked.rs` says so where it turns `EINVAL` and
    /// `ERANGE` into `Io` — "neither is a statement about the relay or a silent
    /// runner, so neither borrows `Derp`'s or `NoAnswer`'s words". Writing copy
    /// for a rendezvous failure during a mint would be copy nothing can ever
    /// show, which is the same trap ``RunnerRefusal``'s header refuses; they
    /// fall to the generic instead, alongside every word from a core newer than
    /// this build — ``NodeKeyMint/read(_:)`` has already turned one of those
    /// into `unspecified` before it reaches here, so an unknown word degrades
    /// to a sentence and never to silence.
    private static func because(_ word: RunnerTrouble.TunnelWord) -> String {
        switch word {
        case .notInThisBuild:
            "This build of Far Cooler doesn’t include the tunnel."
        case .unspecified, .rendezvous, .noAnswer:
            "Far Cooler couldn’t make one on this device."
        }
    }
}

/// What `farcooler_client_mint_node_key` answered, with the reason kept.
///
/// The reason is the whole point. `NodeIdentity.mint()` used to collapse five
/// distinct answers — an error word, a truncated buffer, a reply that is not
/// JSON, a reply missing a half, and an empty half — into one `nil`, and that
/// `nil` was then collapsed again by `mintIfNeeded`, again by `showOffer`,
/// again by the daemon and again by `Enrollment.token(in:)`. Each of those five
/// collapses is correct on its own; the first one is the only one that could
/// have kept anything, so it is the one that now does.
enum NodeKeyMint: Equatable, Sendable {
    case pair(privateKey: String, publicKey: String)
    case refused(RunnerTrouble.TunnelWord)

    /// Read one decoded mint answer.
    ///
    /// Takes the decoded object rather than the bytes because the buffer
    /// contract belongs to the call site and is the same one
    /// `farcooler_client_generate_key` uses. Nil covers every way the answer
    /// failed to become an object at all — a zero return, a return longer than
    /// the buffer, bytes that are not JSON, JSON that is not a dictionary.
    ///
    /// **A missing or unreadable answer is `unspecified`, not a fifth word.**
    /// `linked.rs` makes exactly that call on the Rust side for the buffer
    /// errors this mirrors, and `TunnelWord.unspecified` already documents
    /// itself as "where an unrecognized word lands". Inventing a word here
    /// would put a dialect in the app that the core has never heard of.
    ///
    /// The pair guard is the one `mint()` already had, and it stays for the
    /// reason `crates/client/src/ffi.rs` gives: an offer carrying an empty or
    /// half-empty node key looks filled in and grants a tunnel that admits
    /// nobody, and tailcat ignores an unrecognized client in silence.
    static func read(_ answer: [String: Any]?) -> NodeKeyMint {
        guard let answer else { return .refused(.unspecified) }
        if let priv = answer["private_key"] as? String, !priv.isEmpty,
            let pub = answer["public_key"] as? String, !pub.isEmpty
        {
            return .pair(privateKey: priv, publicKey: pub)
        }
        let word = answer["error"] as? String ?? ""
        return .refused(RunnerTrouble.TunnelWord(rawValue: word) ?? .unspecified)
    }
}
