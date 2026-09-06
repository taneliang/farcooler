import Foundation

/// Several things waiting to be told about one event, told by name.
///
/// **Written down as a type because the alternative is a slot, and a slot is
/// the bug.** `Reachability` held one closure — "a connection does not need to
/// know why now is a better moment than the one its timer picked, only that it
/// is" — and that was exactly right while the app talked to one runner. With
/// several, the last one to register took the slot and every other runner went
/// on waiting out a backoff through the very network change that would have
/// fixed it. Walking through a door recovers a phone's whole fleet at once, or
/// it recovers whichever runner happened to register last.
///
/// **Here rather than beside `Reachability` because the iOS target has no unit
/// tests, only UI tests** — `ShellNavigation` and `PaneDrafts` are in this
/// package for the same reason. What `Reachability` adds is an `NWPathMonitor`
/// and the rule that only the transition INTO reachable is news; neither of
/// those can be exercised without a radio. Everything else about it — that all
/// of them fire, that a key replaces rather than accumulates, and that a
/// handler which reconciles the fleet mid-dispatch does not corrupt the walk —
/// is this file, and this file is testable on a host with no phone attached.
/// See `KeyedCallbackTests`.
///
/// Keyed rather than token-based, because the thing that owns these lifetimes
/// is a fleet store: it brings a connection up under a runner's id and tears it
/// down under the same id, so subscribing and unsubscribing are the two halves
/// of one reconcile rather than a bookkeeping problem of their own.
///
/// `@MainActor` because every subscriber it will ever have is a view model on
/// the main actor, and because holding non-`Sendable` closures anywhere else
/// would be a promise this cannot keep.
///
/// A CLASS and not a struct, which is the one shape decision here worth a
/// sentence. A handler is allowed to add and remove subscribers — that is the
/// whole point of `fire`'s snapshot below — and doing that to a struct held as
/// somebody's stored property means writing to the very property whose read is
/// still on the stack, which is an exclusivity violation rather than a subtle
/// ordering question. Reference semantics make the mutation ordinary.
@MainActor
final class KeyedCallbacks {
    private var handlers: [String: () -> Void] = [:]

    init() {}

    /// How many are listening. For tests and for a caller that wants to skip
    /// work nobody is waiting on.
    var count: Int { handlers.count }

    /// Listen under a key. The same key twice REPLACES — a connection that
    /// reconnects is not a second subscriber, and one that accumulated would
    /// fire once per reconnect it had ever made.
    func add(_ key: String, _ handler: @escaping () -> Void) {
        handlers[key] = handler
    }

    /// Stop listening. Removing a key nothing registered is not an error:
    /// tearing down a connection that never got as far as subscribing is an
    /// ordinary thing for a reconcile to do.
    func remove(_ key: String) {
        handlers[key] = nil
    }

    /// Tell all of them, as they were when the event arrived.
    ///
    /// A handler here reconnects a runner, a reconnect can reconcile the
    /// fleet, and a reconcile adds and removes subscribers — so this loop has
    /// to have an answer for its own collection changing underneath it. The
    /// answer is the one the language already gives: `Dictionary` is a value
    /// type, so the sequence walked here is a copy and mutating `handlers`
    /// during the walk neither corrupts it nor is visible to it.
    ///
    /// **The `Array(…)` is therefore explicitness and not a fix**, and it is
    /// worth saying so out loud rather than leaving a reader to assume this
    /// line is load-bearing: `for handler in handlers.values` behaves
    /// identically. What is behavior, and what `KeyedCallbackTests` actually
    /// pins, is the consequence — every subscriber listening at the moment of
    /// the event is told about it, and one a handler adds mid-dispatch is not
    /// told about an event that predates it.
    func fire() {
        for handler in Array(handlers.values) { handler() }
    }
}
