import SwiftUI

/// Text one pane wants to put in another pane's composer.
///
/// One case today: the diff pane's outbox offers "Put in Composer" beside
/// "Send", because on a Mac the agent is a few inches away rather than a screen
/// away. That is worth a channel of its own for a reason the review queue is
/// careful about — `terminal agent-prompt` has no delivery receipt, so a send
/// can only ever be reported as "handed over", while text sitting in a composer
/// the reader is looking at needs no receipt at all. They press Return, and the
/// transcript shows what happened.
///
/// A shared object rather than a parameter, because the two panes are siblings
/// with a tmux layout between them: `ChangesPane` and `AgentSurface` are built
/// by the same `TileView`, but the diff has no reference to the chat and giving
/// it one would mean threading a callback for every pane through a view that
/// deliberately knows nothing about what its panes contain.
///
/// The text WAITS. A note put into a pane whose layout is not on screen — or
/// whose chat is currently showing its raw terminal — has no composer to land
/// in yet, so it stays here until one appears and takes it. Nothing is lost if
/// none ever does: the batch is in the outbox's receipt list with its full
/// text, which is the same place a sent batch is.
///
/// In memory only, unlike `ReviewCommentQueue`. What is here is a message the
/// reader has already decided to send, on its way across one window; the queue
/// is the thing that is written down, and it is written down BEFORE this is
/// ever reached.
@MainActor
final class ComposerHandoff: ObservableObject {
    static let shared = ComposerHandoff()

    /// Text waiting for a pane, by the terminal's short id — the same
    /// identifier `AgentStream` and `ReviewAgentTarget` use on this platform.
    @Published private(set) var waiting: [String: String] = [:]

    private init() {}

    /// Leave text for a pane. Two batches offered before either is taken are
    /// joined rather than replaced, on the same rule the composer itself
    /// follows when it receives one: nothing a person wrote is overwritten by
    /// something else they wrote.
    func offer(_ text: String, to terminal: String) {
        guard !text.isEmpty else { return }
        if let already = waiting[terminal], !already.isEmpty {
            waiting[terminal] = already + "\n\n" + text
        } else {
            waiting[terminal] = text
        }
    }

    /// Take what is waiting, once.
    func take(for terminal: String) -> String? {
        waiting.removeValue(forKey: terminal)
    }
}
