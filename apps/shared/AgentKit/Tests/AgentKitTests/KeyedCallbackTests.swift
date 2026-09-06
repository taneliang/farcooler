import Foundation
import Testing

@testable import AgentKit

/// The rule a single closure slot could not keep: everybody waiting on one
/// event is told about it.
@MainActor
struct KeyedCallbackTests {
    /// Boxed rather than a local `var` captured by the closures, so what the
    /// assertions read is what the handlers wrote.
    private final class Counter {
        var fired: [String] = []
    }

    /// The defect this type replaced, stated directly: with a slot, the second
    /// registration silenced the first and only "b" would appear here.
    @Test func everySubscriberIsTold() {
        let seen = Counter()
        let callbacks = KeyedCallbacks()
        callbacks.add("a") { seen.fired.append("a") }
        callbacks.add("b") { seen.fired.append("b") }
        callbacks.add("c") { seen.fired.append("c") }
        callbacks.fire()
        #expect(seen.fired.sorted() == ["a", "b", "c"])
    }

    /// A connection that reconnects re-registers under its own key, and must
    /// not end up being told twice about one door.
    @Test func theSameKeyTwiceReplaces() {
        let seen = Counter()
        let callbacks = KeyedCallbacks()
        callbacks.add("runner") { seen.fired.append("old") }
        callbacks.add("runner") { seen.fired.append("new") }
        callbacks.fire()
        #expect(seen.fired == ["new"])
        #expect(callbacks.count == 1)
    }

    /// A torn-down connection must not be woken by a door it is no longer
    /// behind.
    @Test func aRemovedSubscriberIsNotTold() {
        let seen = Counter()
        let callbacks = KeyedCallbacks()
        callbacks.add("gone") { seen.fired.append("gone") }
        callbacks.add("here") { seen.fired.append("here") }
        callbacks.remove("gone")
        callbacks.fire()
        #expect(seen.fired == ["here"])
    }

    /// Reconciling a fleet is the ordinary thing for one of these handlers to
    /// do, and a reconcile adds and removes subscribers. Every subscriber that
    /// was listening when the event arrived is still told.
    ///
    /// This pins the OBSERVABLE rule and not the line that implements it.
    /// `fire`'s `Array(…)` cannot be broken by deleting it — `Dictionary` is a
    /// value type and the loop already walks a copy — so a test written
    /// against that line would be a test that cannot fail. What can be broken
    /// is the rule: a subscriber added mid-dispatch being told about an event
    /// that predates it, or one of the two that were listening being skipped.
    @Test func aHandlerMayReconcileTheFleetWhileTheEventIsBeingDelivered() {
        let seen = Counter()
        let callbacks = KeyedCallbacks()
        callbacks.add("reconciler") {
            seen.fired.append("reconciler")
            callbacks.remove("doomed")
            callbacks.add("fresh") { seen.fired.append("fresh") }
        }
        callbacks.add("doomed") { seen.fired.append("doomed") }
        callbacks.add("bystander") { seen.fired.append("bystander") }
        callbacks.fire()
        // Both of the two that were listening at the moment of the event ran,
        // whichever order the reconciler happened to be walked in.
        #expect(seen.fired.contains("reconciler"))
        #expect(seen.fired.contains("bystander"))
        // The one added mid-dispatch is not told about an event that predates
        // it. Its own first door is the next one.
        #expect(!seen.fired.contains("fresh"))
    }

    /// Removing a key nothing registered is what a reconcile does to a
    /// connection torn down before it ever subscribed.
    @Test func removingAKeyThatIsNotThereIsFine() {
        let callbacks = KeyedCallbacks()
        callbacks.remove("never")
        #expect(callbacks.count == 0)
        callbacks.fire()
    }
}
