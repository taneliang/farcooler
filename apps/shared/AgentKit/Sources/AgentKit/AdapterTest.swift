import Foundation

/// What the Test button in an adapter editor found, and what it says about it.
///
/// This type was declared three times — `RunnerSettingsStore.swift`,
/// `RunnerSettings.swift`, and Android's `model/RunnerSettings.kt` — and each
/// declaration had a `switch` beside it turning it into words, byte for byte
/// the same as the other two. That is the duplication `b357841` lifted for
/// `GapReason` and `f9f37eb` lifted for the notification strings: nobody sees
/// the drift, because nobody presses Test on a Mac and on a phone at the same
/// moment. It had already drifted here in behavior — see `sentence`.
///
/// The cases name a SITUATION and never carry a sentence, and that is the half
/// that keeps this from coming apart again. A call site chooses which of the
/// three ways a test failed; the words are chosen here, once. It is also what
/// stops the mistake `7661b67` found on Android, where sentences the app had
/// written itself were passed down a channel meant for a runner's output and
/// then shown as one.
public enum AdapterTestOutcome: Equatable {
    /// It started, it answered, and this is what it called itself.
    ///
    /// A name, not a sentence: `describe` in `crates/acp/src/handshake.rs`
    /// builds it from the agent's own `agentInfo`, and the two native
    /// backends build it from the version they checked — "Claude Code 2.0.1",
    /// "codex app-server 0.44". So it is joined to the sentence rather than
    /// boxed as output. A runner's *diagnosis* is what must never arrive in
    /// this app's voice; a runner's name for itself is the news.
    case worked(String)

    /// It didn't, in one of three ways.
    case failed(Reason)

    /// Which way a test failed, and what the other side said about it.
    public enum Reason: Equatable {
        /// The form could not be turned into a request, so nothing was sent.
        ///
        /// Only the Mac can reach this: it pipes the form to the CLI as JSON,
        /// and the phones hand a dictionary to the bridge, which has no
        /// encoding step that can fail.
        case formUnusable

        /// The request went out and nothing usable came back.
        ///
        /// Carries whatever the transport printed, where there is any. The Mac
        /// has the CLI's own line — an exit with output that is not JSON is
        /// usually the CLI saying it could not reach that runner — and the
        /// phones have nothing to put here, because a bridge call that throws
        /// throws a state rather than a message.
        case noAnswer(String?)

        /// The handshake ran on the runner and did not complete, in the
        /// runner's own words.
        ///
        /// Those words are the only clue about which field is wrong, which is
        /// why `adapter.test` in `crates/daemon/src/rpc.rs` passes the
        /// adapter's message through deliberately instead of replacing it with
        /// "the test failed". They are also lowercase fragments about a
        /// process — `could not find \`npx\` on this runner`, `the adapter
        /// started and then went silent`, or an agent's own JSON-RPC error
        /// message — so they are shown here as output and never as a sentence.
        case refused(String)
    }
}

extension AdapterTestOutcome {
    /// Whether to draw this as good news.
    ///
    /// A property rather than each editor pattern-matching for it, because
    /// each editor doing that is how three of them ended up with three copies
    /// of everything else on this type.
    public var succeeded: Bool {
        if case .worked = self { return true }
        return false
    }

    /// This app's own account of what happened, in one place for three
    /// platforms.
    ///
    /// The success line used to read "Starts and speaks ACP — \(reported)",
    /// and that was the defect the triplication was hiding. An adapter set to
    /// the agent's native protocol is dispatched by the daemon to `codex
    /// app-server` or the Claude CLI's stream-json handshake, where no ACP is
    /// spoken at all — and when only one of three platforms could ask for
    /// that, the identical sentence was true on two and false on the third,
    /// which is precisely the drift nobody can see from one device.
    ///
    /// It names no protocol now, and that is not a stopgap: this type is
    /// handed an outcome and never the form that produced it, so on this side
    /// the weaker sentence is the only true one. What Test proves either way
    /// is that the program starts and answers, and the protocol, where it
    /// matters, is already inside `reported`. A caller that does still hold
    /// the adapter can be more specific — `farcooler adapter test` names the
    /// backend, because it sent it.
    public var sentence: String {
        switch self {
        case .worked(let reported):
            return "Starts and answers — \(reported)"
        case .failed(.formUnusable):
            return "That adapter couldn’t be described."
        case .failed(.noAnswer):
            return "That runner couldn’t be reached."
        // The negation of the success line on purpose: "start and answer" is
        // the pair being proven, and a test fails when the pair doesn't hold —
        // whether the program was never found, never started, started and went
        // quiet, or answered with a refusal. No cause is named here because
        // this side cannot know which of those it was, and a guess sends
        // somebody to change a setting that was never the problem. The one
        // account anybody has of it is `detail`.
        case .failed(.refused):
            return "This adapter didn’t start and answer."
        }
    }

    /// The other side's own words, to be shown as output.
    ///
    /// `nil` wherever there is nothing to show, so an editor can ask before it
    /// reserves the space — an empty box under a sentence reads as output that
    /// failed to arrive. Empty counts as nothing: the daemon sends `failure`
    /// as `""` on success and a client that lost the field entirely decodes to
    /// the same thing, and neither is a transcript.
    ///
    /// `.formUnusable` has none by construction — nothing was sent, so nobody
    /// said anything — and a box under a sentence that already names its own
    /// cause is noise, the way `776d3e0` and `c42c352` both scoped theirs.
    public var detail: String? {
        switch self {
        case .failed(.noAnswer(let output)):
            return output.flatMap { $0.isEmpty ? nil : $0 }
        case .failed(.refused(let said)):
            return said.isEmpty ? nil : said
        // Listed rather than defaulted, so a case added later has to decide.
        case .worked, .failed(.formUnusable):
            return nil
        }
    }
}
