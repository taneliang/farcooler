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
/// Deliberately one callback rather than a notification per connection: a
/// connection does not need to know why now is a better moment than the one
/// its timer picked, only that it is.
@MainActor
final class Reachability {
    static let shared = Reachability()

    /// Called when the path goes from unsatisfied to satisfied.
    var onShouldRetry: (() -> Void)?

    private let monitor = NWPathMonitor()
    private var wasSatisfied = true

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
                self.onShouldRetry?()
            }
        }
        monitor.start(queue: .main)
    }
}
