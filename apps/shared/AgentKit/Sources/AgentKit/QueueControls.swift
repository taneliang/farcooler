/// Whether a runner can change a queued message, and what to say when it can't.
///
/// Edit, Remove and Send Now on a queued message are three calls
/// (`terminal.agent_edit_queued`, `…_cancel_queued`, `…_steer_queued`) that a
/// runner older than the `agent_queue` capability refuses as an unknown method.
/// This is the phones' one reading of that, in AgentKit so a test can fail it
/// (ov-171): a decision made inside a `View` is one nothing reads back.
///
/// Conservative on purpose. A runner that serves the calls without advertising
/// `agent_queue` is gated too, because the alternative is a button that may
/// do nothing. A runner we haven't heard from yet is not gated: it hasn't
/// refused anything, and a refusal is shown anyway.
public enum QueueControls: Equatable, Sendable {
    case available
    case unavailable(sentence: String)

    /// What a runner that can't change the queue is told.
    public static let olderRunnerSentence =
        "This runner can’t change queued messages. Update it to edit or cancel them."

    public static func gate(_ daemon: DaemonBuild?) -> QueueControls {
        guard let daemon else { return .available }
        return daemon.can(.agentQueue) ? .available : .unavailable(sentence: olderRunnerSentence)
    }

    public var isAvailable: Bool { self == .available }

    /// The three calls, and the step each one says it couldn't finish.
    public enum Action: Sendable {
        case edit, steer, cancel

        public var failed: String {
            switch self {
            case .edit: "Couldn’t save that edit."
            case .steer: "Couldn’t send that into the running turn."
            case .cancel: "Couldn’t take that message back."
            }
        }
    }

    /// The sentence beside a queue call a runner refused anyway.
    ///
    /// The step first; the runner's own reason follows it when this build can
    /// read the word. Never empty, never the raw error.
    public static func refusal(_ action: Action, word: String?, message: String) -> String {
        RunnerRefusal.trouble(forWord: word, message: message, after: action.failed).sentence
    }
}
