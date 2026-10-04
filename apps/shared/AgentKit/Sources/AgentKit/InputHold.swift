import Foundation

/// What became of one `terminal.write`, in the three ways that matter to typed
/// input (ov-238).
public enum WriteOutcome: Equatable, Sendable {
    /// The runner answered that it wrote the bytes.
    case written
    /// The bytes provably never reached the runner, so sending them again can't
    /// type them twice.
    case neverSent(InputHold.Reason)
    /// They may have arrived. A deadline "cannot unsend" a key already on the
    /// wire (`crates/client/src/deadlines.rs`), and a link that drops mid-call
    /// may have carried it, so these are never sent again.
    case maybeSent

    /// The outcome of a failed call, from what its answer line said.
    ///
    /// `word` is `RunnerRefusal.word(inAnswerLine:)`, `disconnected` the line's
    /// flag and `notSent` its `not_sent`, which only a call with no session to
    /// go on carries. A runner's own refusal means it wrote nothing. Everything
    /// else is doubt, and doubt is not resent.
    public static func failed(word: String?, disconnected: Bool, notSent: Bool) -> WriteOutcome {
        if notSent { return .neverSent(.disconnected) }
        if disconnected || word == nil || word == RunnerRefusal.timedOutWord { return .maybeSent }
        return .neverSent(.refused)
    }
}

/// Typed input a terminal could not send, held until the person decides, and
/// the order it all goes in (ov-238).
///
/// A terminal write used to be `try?`: a keystroke the runner never answered was
/// gone, and the screen looked the same as when it worked. The first fix held
/// every failure and flushed it on the next keystroke, which could type a
/// command twice (a timed-out key may have arrived) and typed minutes-old input
/// without anyone choosing to. These are the rules instead:
///
/// - **Only input that provably never left the phone is held**, and it goes out
///   again only when the person taps Try Again. Discard drops it. A new key
///   never flushes it.
/// - **A timed-out or dropped write is not held and not resent.** The line says
///   some typing may not have arrived, and that is all.
/// - **Order is kept.** Writes go one at a time: keys typed while one is in
///   flight, or while input is held, queue behind it, so a later key can't
///   overtake an earlier failed one. Held input is cleared only once an answer
///   confirms it was written.
/// - **It is bounded.** Held input is capped at `cap` bytes; past it the earliest
///   are kept and the line says so. It is dropped when the pane goes away.
///
/// In AgentKit rather than in `TerminalSession` because `apps/ios` has no unit
/// tests CI runs, and each rule above is a place a plausible edit goes wrong.
@MainActor
public final class InputHold: ObservableObject {
    /// Why input is held.
    public enum Reason: Equatable, Sendable {
        case disconnected
        case refused
    }

    /// The most input kept for Try Again.
    public static let cap = 4096

    /// Never-sent input, oldest first, waiting for Try Again or Discard.
    @Published public private(set) var held: [UInt8] = []
    @Published public private(set) var reason: Reason = .refused
    /// Whether input past `cap` was dropped.
    @Published public private(set) var truncated = false
    /// Some typing may not have reached the runner, and will not be resent.
    @Published public private(set) var maybeLost = false

    private var pending: [UInt8] = []
    private var sending = false
    private var epoch = 0

    public init() {}

    /// Whether Try Again and Discard are offered.
    public var isHolding: Bool { !held.isEmpty }

    /// Type `bytes`. Behind anything held or in flight, else sent now.
    public func type(_ bytes: [UInt8], send: ([UInt8]) async -> WriteOutcome) async {
        guard !bytes.isEmpty else { return }
        if !held.isEmpty && !sending {
            absorb(bytes)
            return
        }
        pending += bytes
        if sending { return }
        await drain(send)
    }

    /// Try Again: send what is held, then what was typed behind it.
    public func retry(send: ([UInt8]) async -> WriteOutcome) async {
        guard !held.isEmpty, !sending else { return }
        sending = true
        let mine = epoch
        let outcome = await send(held)
        sending = false
        guard mine == epoch else { return }
        switch outcome {
        case .written:
            clearHeld()
        case .neverSent(let why):
            reason = why
            absorb(pending)
            pending = []
            return
        case .maybeSent:
            clearHeld()
            maybeLost = true
        }
        await drain(send)
    }

    /// Discard: drop what is held.
    public func discard() {
        clearHeld()
        pending = []
    }

    /// Dismiss the "may not have reached" line.
    public func dismissMaybeLost() { maybeLost = false }

    /// The pane closed or went away: nothing held or queued is sent, ever.
    public func paneClosed() {
        epoch += 1
        sending = false
        clearHeld()
        pending = []
        maybeLost = false
    }

    /// The one line a terminal shows, or nil.
    public var sentence: String? {
        if !held.isEmpty {
            let why = reason == .disconnected
                ? "Far Cooler lost the connection before your typing reached the runner."
                : "The runner didn’t take your typing."
            return truncated ? why + " Only the first 4 KB is kept." : why
        }
        return maybeLost ? "Some typing may not have reached the runner." : nil
    }

    public static let retryTitle = "Try Again"
    public static let discardTitle = "Discard"
    public static let dismissTitle = "OK"

    private func drain(_ send: ([UInt8]) async -> WriteOutcome) async {
        sending = true
        let mine = epoch
        while !pending.isEmpty && held.isEmpty {
            let batch = pending
            pending = []
            let outcome = await send(batch)
            guard mine == epoch else { return }
            switch outcome {
            case .written:
                maybeLost = false
            case .neverSent(let why):
                reason = why
                held = []
                truncated = false
                absorb(batch)
                absorb(pending)
                pending = []
            case .maybeSent:
                maybeLost = true
            }
        }
        sending = false
    }

    private func absorb(_ bytes: [UInt8]) {
        let room = max(0, Self.cap - held.count)
        if bytes.count > room { truncated = true }
        held += bytes.prefix(room)
    }

    private func clearHeld() {
        held = []
        truncated = false
    }
}
