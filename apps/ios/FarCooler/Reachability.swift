import Network

/// The moment when waiting out a backoff is the wrong thing to do.
///
/// The Mac's `Reachability`, on a phone, minus the half that does not apply: a
/// laptop's lid is `NSWorkspace.didWakeNotification`, and a phone's equivalent
/// is the scene becoming active, which SwiftUI already delivers to the view
/// that needs it (see `RootView`). What is left is the network, and it matters
/// more here than it does on a desk — a phone changes networks by being
/// carried through a door.
///
/// **A LIST of subscribers, not one slot.** It held a single closure, which was
/// exactly right while the app talked to one runner and exactly wrong the
/// moment it talked to several: the second connection to call `start` took the
/// slot, and the first went on waiting out a backoff timer through the very
/// network change that would have fixed it. Several runners recovering from one
/// network event is the ordinary case on a phone — walking through a door
/// reconnects all of them — and it is the case a single slot cannot serve.
///
/// The list itself is `KeyedCallbacks`, which lives in AgentKit so that the
/// half of this with no radio in it can be tested — see that file's header. What
/// stays here is the part that needs a phone: an `NWPathMonitor`, and the rule
/// that only the transition INTO reachable is news.
///
/// **Both other platforms keep ONE callback here, and that is not drift to be
/// swept.** The Mac's `FleetStore.init` assigns `onShouldRetry` once and fans
/// out with `reconnectAll()`; Android constructs `Reachability(application) {
/// fleet.reconnectAll() }` and does the same. A list is what iOS needs while it
/// has no fleet store — every connection subscribes for itself, because there
/// is nothing above them to do it on their behalf. When iOS gains one, the
/// right move is to follow the other two: the store becomes the single
/// subscriber and the connections stop registering at all. A list with one
/// entry costs nothing and is not the reason to keep several.
///
/// A subscriber still does not learn WHY now is a better moment than the one
/// its timer picked, only that it is. That part of the original design is
/// unchanged.
@MainActor
final class Reachability {
    static let shared = Reachability()

    private let subscribers = KeyedCallbacks()

    private let monitor = NWPathMonitor()
    private var wasSatisfied = true

    /// Be told when the path goes from unsatisfied to satisfied.
    ///
    /// The key is the subscriber's own identity — a runner's id — and the same
    /// key twice replaces rather than accumulating.
    func onShouldRetry(_ key: String, _ handler: @escaping () -> Void) {
        subscribers.add(key, handler)
    }

    /// Stop being told. A connection that has been torn down must not be
    /// woken by a door it is no longer behind.
    func stopWatching(_ key: String) {
        subscribers.remove(key)
    }

    private init() {
        monitor.pathUpdateHandler = { [weak self] path in
            let satisfied = path.status == .satisfied
            Task { @MainActor in
                guard let self else { return }
                defer { self.wasSatisfied = satisfied }
                // Only the transition INTO reachable. A path that was already
                // satisfied and stayed that way is not news, and a phone
                // hands out plenty of those — every cell handoff is one.
                guard satisfied, !self.wasSatisfied else { return }
                self.subscribers.fire()
            }
        }
        monitor.start(queue: .main)
    }
}
