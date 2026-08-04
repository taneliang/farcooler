import AppKit
import Network

/// The two moments when waiting out a backoff is the wrong thing to do.
///
/// A laptop lid closing and opening is the single most common way this feature
/// will be experienced, and without this it means sitting through a thirty
/// second wait while everything looks broken. A network that just came back is
/// the same story with a different cause.
///
/// Deliberately one callback rather than a notification per client: the clients
/// do not each need to know why, only that now is a better moment than the one
/// their timer picked.
@MainActor
final class Reachability {
    static let shared = Reachability()

    /// Called on wake, and when the path goes from unsatisfied to satisfied.
    var onShouldRetry: (() -> Void)?

    /// The same signal wake and network-regain send, for a person asking for
    /// it directly — the Settings window's own "Reconnect all".
    ///
    /// Goes through `onShouldRetry` rather than a second, parallel path to
    /// `FleetStore`, because `HostsSettings` lives in a separate `Settings`
    /// scene with no `FleetStore` of its own to call — this singleton is
    /// already the one thing both sides share.
    func retryNow() { onShouldRetry?() }

    private let monitor = NWPathMonitor()
    private var wasSatisfied = true

    private init() {
        NSWorkspace.shared.notificationCenter.addObserver(
            forName: NSWorkspace.didWakeNotification,
            object: nil,
            queue: .main
        ) { [weak self] _ in
            MainActor.assumeIsolated { self?.onShouldRetry?() }
        }

        monitor.pathUpdateHandler = { [weak self] path in
            let satisfied = path.status == .satisfied
            Task { @MainActor in
                guard let self else { return }
                defer { self.wasSatisfied = satisfied }
                // Only the transition INTO reachable. A path that was already
                // satisfied and stayed that way is not news.
                guard satisfied, !self.wasSatisfied else { return }
                self.onShouldRetry?()
            }
        }
        monitor.start(queue: .main)
    }
}
