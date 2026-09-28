import Foundation

/// Which blocked panes the phone reads a permission off, one fleet poll at a
/// time, so the lock screen card can offer Allow and Deny.
///
/// **Why this exists.** The card draws its buttons from `GlancePermissions`,
/// and only something holding a connection can write that file. Before this,
/// two things did: `AgentStream`, which runs only while a chat pane is on
/// screen, and the watch asking `WatchLinkHost` what an agent is waiting on.
/// A claude TUI pane is never a chat pane, so without a watch its ask (a
/// `Permission` the daemon records while it holds claude's `PermissionRequest`
/// hook) reached the card with no buttons at all.
///
/// The fleet poll is the one place the app learns that any pane is blocked, so
/// this rides it: every pane the poll says is blocked is read once per poll,
/// and a pane that stops being blocked has its record cleared.
///
/// **Every poll, not only on the edge into blocked.** The daemon's `blocked`
/// comes off the pane's screen, and claude draws its dialog before the hook's
/// ask reaches the ring, so the first read can find nothing yet. And claude can
/// answer one ask and raise the next between two polls, which the fleet shows
/// as blocked throughout. A poll is every 3 s, or every 15 s while the runner's
/// event channel is live (sooner when fleet news arrives), and only while the
/// app is active: the lock screen gets buttons for an ask the app saw before
/// the phone locked. A read is a delta from a cached cursor
/// (see `WatchLinkHost.replay`), so reading again costs one short round trip.
///
/// **Cleared on the edge out.** The daemon's own `Resolved` retires an ask that
/// ended while the pane stayed blocked; a pane that stopped being blocked
/// (claude's dialog answered at the keyboard, the turn moved on) is cleared
/// here, so the card is not left holding buttons for a question that is over.
///
/// One per connection. Pure bookkeeping, so `swift test` can reach it.
public struct BlockedAskLookups: Sendable, Equatable {
    /// The panes the last poll said were blocked.
    public private(set) var blocked: Set<String> = []

    /// Panes with a read still running, which the next poll does not start a
    /// second read for.
    public private(set) var reading: Set<String> = []

    /// Panes with a read out that started in a blocked spell which has since
    /// ended. What those reads find is never filed, even if the pane is
    /// blocked again by the time they finish.
    public private(set) var stale: Set<String> = []

    public init() {}

    /// What one poll asks for.
    public struct Step: Sendable, Equatable {
        /// Panes to read now, in a stable order.
        public let read: [String]
        /// Panes that stopped being blocked, whose record to clear.
        public let clear: [String]
    }

    /// Take one poll's blocked panes, and say what to read and what to clear.
    ///
    /// Each pane in `read` is marked as being read until `finished` is called
    /// for it.
    public mutating func poll(blocked now: Set<String>) -> Step {
        let clear = blocked.subtracting(now).sorted()
        stale.formUnion(reading.intersection(clear))
        let read = now.subtracting(reading).sorted()
        blocked = now
        reading.formUnion(read)
        return Step(read: read, clear: clear)
    }

    /// The panes whose asks the poll reads: the blocked TUI panes.
    ///
    /// Not a chat pane. Its ask comes from ACP, which never sends `Resolved`,
    /// so a read that lands after an answer in the chat but before the fleet
    /// leaves blocked would file the answered ask again, and a lock screen tap
    /// would answer it a second time. `AgentStream` files a chat pane's asks
    /// itself, and knows when it answered them.
    static func asking(_ terminals: [Terminal]) -> Set<String> {
        Set(terminals.filter { $0.agent == .blocked && !$0.isAgentPane }.map(\.id))
    }

    /// A read has finished, however it went. Returns whether what it found may
    /// still be filed.
    ///
    /// False when a poll that landed while the read was running said the pane
    /// is no longer blocked, even if a later poll said it is blocked again.
    /// That poll has already cleared its record, and filing a read that
    /// started before it would put the buttons back for an ask that is over.
    public mutating func finished(_ terminal: String) -> Bool {
        reading.remove(terminal)
        let spellEnded = stale.remove(terminal) != nil
        return !spellEnded && blocked.contains(terminal)
    }
}
