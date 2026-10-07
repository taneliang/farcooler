import Foundation

/// A draft the runner holds behind a dialog (ov-385).
///
/// Ask the Orchestrator, Reverse and Discuss paste a draft into a terminal
/// orchestrator's box with no Enter. When a dialog is up there (a permission,
/// a question, a menu), a runner with the `draft_hold` capability keeps the
/// draft and pastes it once the dialog closes, instead of refusing it. The
/// screen that sent it says so, offers Withdraw while it waits, and says
/// "Sent" once it's in. It reads how the hold is doing from the terminal's
/// `draftHold`, which every fleet read and terminal event carries.
///
/// The phones and the Mac read the same JSON: the FFI's `{"held": …}` and a
/// fleet row's `draftHold`, and the Mac's CLI prints the same shape. Android
/// says the same sentences from `model/HeldDraft.kt`.
public struct DraftHold: Hashable, Sendable, Decodable {
    public enum State: String, Hashable, Sendable {
        case waiting, sent, withdrawn, expired
    }

    /// Names the hold to `terminal.draft_withdraw`, and tells this screen's
    /// hold from a newer one.
    public let id: String
    public let state: State
    /// Unix milliseconds: when it gives up if it's still waiting.
    public let expiresMs: Int64

    public init(id: String, state: State, expiresMs: Int64 = 0) {
        self.id = id
        self.state = state
        self.expiresMs = expiresMs
    }

    private enum Keys: String, CodingKey { case id, state, expiresMs }

    /// A fleet row's `draftHold`. A state this build doesn't know reads as
    /// ended, never as waiting, so nothing waits on it forever.
    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: Keys.self)
        id = try c.decode(String.self, forKey: .id)
        state = (try c.decodeIfPresent(String.self, forKey: .state)).flatMap(State.init(rawValue:)) ?? .expired
        expiresMs = try c.decodeIfPresent(Int64.self, forKey: .expiresMs) ?? 0
    }

    /// From `{"id", "state", "expiresMs", …}`, as `init(from:)` reads it.
    public init?(json: Any?) {
        guard let object = json as? [String: Any], let id = object["id"] as? String, !id.isEmpty else {
            return nil
        }
        self.id = id
        self.state = (object["state"] as? String).flatMap(State.init(rawValue:)) ?? .expired
        self.expiresMs = (object["expiresMs"] as? NSNumber)?.int64Value ?? 0
    }
}

/// What the screen that sent a held draft says about it, from the hold on
/// its terminal as the runner last reported it.
public enum HeldDraft {
    /// How a held draft is doing.
    public enum Status: Equatable, Sendable {
        /// Behind the dialog still.
        case waiting
        /// In the orchestrator's box, unsent.
        case sent
        /// Not pasted within half an hour.
        case expired
        /// The runner no longer has it: it restarted.
        case lost
        /// Withdrawn, or replaced by a newer draft: nothing to say.
        case gone
    }

    /// `tracked`, the hold this screen sent, as `current` (the terminal's hold
    /// now) says it is. `seen` says the screen has read its hold on the
    /// terminal before, so a terminal with none means the runner lost it,
    /// rather than a read from before the hold.
    public static func status(of tracked: String, current: DraftHold?, seen: Bool) -> Status {
        guard let current else { return seen ? .lost : .waiting }
        guard current.id == tracked else { return .gone }
        switch current.state {
        case .waiting: return .waiting
        case .sent: return .sent
        case .withdrawn: return .gone
        case .expired: return .expired
        }
    }

    /// The banner's first line, or nil when it has nothing to say.
    public static func title(_ status: Status) -> String? {
        switch status {
        case .waiting: "Waiting for the dialog to close"
        case .sent: "Sent"
        case .expired, .lost: "Not sent"
        case .gone: nil
        }
    }

    /// The banner's second line: what happens next, or why it didn't.
    public static func detail(_ status: Status) -> String? {
        switch status {
        case .waiting: "Your draft goes into the orchestrator’s box when the dialog in its pane closes."
        case .sent: "Your draft is in the orchestrator’s box. Finish it there and press Return."
        case .expired: "The dialog stayed open for half an hour. Answer it, then try again."
        case .lost: "The runner restarted before the dialog closed. Try again."
        case .gone: nil
        }
    }

    /// The one action while it waits.
    public static let withdraw = "Withdraw"

    /// What `terminal.draft_prompt` answered, read: `{"held": <hold>}` is a
    /// draft held behind a dialog, and anything else is one pasted.
    public static func result(of answer: Data) -> AskAboutTask.DraftResult {
        let object = try? JSONSerialization.jsonObject(with: answer)
        if let hold = DraftHold(json: (object as? [String: Any])?["held"]) { return .held(hold) }
        return .pasted
    }

    /// What a screen watching a pane remembers: the hold it saw waiting, so it
    /// can say how that one ended. `observe` each hold the pane reports;
    /// `status` is what to show, nil for nothing.
    public struct Watch: Equatable, Sendable {
        public private(set) var tracked: String?

        public init() {}

        public mutating func observe(_ hold: DraftHold?) {
            if let hold, hold.state == .waiting { tracked = hold.id }
        }

        public func status(_ current: DraftHold?) -> Status? {
            guard let tracked else { return nil }
            let status = HeldDraft.status(of: tracked, current: current, seen: true)
            return status == .gone ? nil : status
        }

        /// The person closed what it said.
        public mutating func dismiss() { tracked = nil }
    }
}
