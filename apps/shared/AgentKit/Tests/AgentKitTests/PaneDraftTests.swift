import Foundation
import Testing

@testable import AgentKit

/// The one thing a pane's teardown cannot give back.
///
/// A suite name per test's own `UserDefaults`, so these do not read or write
/// the machine's standard defaults and cannot see one another.
struct PaneDraftTests {
    private func defaults() -> UserDefaults {
        let suite = "PaneDraftTests.\(UUID().uuidString)"
        // A suite that cannot be opened is a test that would silently pass
        // against the standard defaults, which is exactly the shape of check
        // this repo keeps finding.
        guard let defaults = UserDefaults(suiteName: suite) else {
            fatalError("no defaults for suite \(suite)")
        }
        return defaults
    }

    @Test func aDraftComesBackForThePaneItWasTypedInto() {
        let store = defaults()
        PaneDraftStore.record("half a thought", forPane: "t1", in: store)
        #expect(PaneDraftStore.draft(forPane: "t1", from: store) == "half a thought")
    }

    /// The failure this whole file exists to prevent, stated as two panes: a
    /// draft must belong to the pane it was written in and to no other.
    @Test func drafsDoNotLeakBetweenPanes() {
        let store = defaults()
        PaneDraftStore.record("for one", forPane: "t1", in: store)
        PaneDraftStore.record("for two", forPane: "t2", in: store)
        #expect(PaneDraftStore.draft(forPane: "t1", from: store) == "for one")
        #expect(PaneDraftStore.draft(forPane: "t2", from: store) == "for two")
    }

    @Test func aPaneNothingWasTypedIntoHasNoDraft() {
        #expect(PaneDraftStore.draft(forPane: "never", from: defaults()) == nil)
    }

    /// Emptying the field is not "an empty draft" — it is no draft. Otherwise
    /// every pane anybody has ever sent from keeps a row forever.
    @Test func clearingTheFieldRemovesTheEntry() {
        let store = defaults()
        PaneDraftStore.record("typed", forPane: "t1", in: store)
        PaneDraftStore.record("", forPane: "t1", in: store)
        #expect(PaneDraftStore.draft(forPane: "t1", from: store) == nil)
        #expect(PaneDraftStore.read(from: store).isEmpty)
    }

    @Test func sendingClearsThatPaneAndLeavesTheOthers() {
        let store = defaults()
        PaneDraftStore.record("mine", forPane: "t1", in: store)
        PaneDraftStore.record("theirs", forPane: "t2", in: store)
        PaneDraftStore.clear(pane: "t1", in: store)
        #expect(PaneDraftStore.draft(forPane: "t1", from: store) == nil)
        #expect(PaneDraftStore.draft(forPane: "t2", from: store) == "theirs")
    }

    /// A month-old draft is forgotten, and forgotten on READ as well as on
    /// write — a phone left in a drawer prunes nothing while it is closed.
    @Test func aDraftOlderThanTheKeepWindowIsGone() {
        let store = defaults()
        let then = Date(timeIntervalSince1970: 1_000_000)
        PaneDraftStore.record("ancient", forPane: "t1", in: store, now: then)
        let later = then.addingTimeInterval(PaneDraftStore.keepFor + 1)
        #expect(PaneDraftStore.draft(forPane: "t1", from: store, now: later) == nil)
    }

    @Test func aDraftInsideTheKeepWindowSurvives() {
        let store = defaults()
        let then = Date(timeIntervalSince1970: 1_000_000)
        PaneDraftStore.record("recent", forPane: "t1", in: store, now: then)
        let later = then.addingTimeInterval(PaneDraftStore.keepFor - 1)
        #expect(PaneDraftStore.draft(forPane: "t1", from: store, now: later) == "recent")
    }

    /// Age is applied before the count, so a fleet of long-dead panes cannot
    /// crowd out the live ones.
    @Test func expiredDraftsAreDroppedBeforeTheCountIsApplied() {
        let now = Date(timeIntervalSince1970: 2_000_000)
        var all: [String: PaneDraft] = [:]
        for index in 0..<(PaneDraftStore.keepAtMost * 2) {
            all["dead\(index)"] = PaneDraft(
                text: "old", savedAt: now.addingTimeInterval(-PaneDraftStore.keepFor - 1))
        }
        all["live"] = PaneDraft(text: "new", savedAt: now)
        let pruned = PaneDraftStore.prune(all, now: now)
        #expect(pruned.count == 1)
        #expect(pruned["live"]?.text == "new")
    }

    /// The second bound: a fleet that is busy rather than old.
    @Test func theNewestDraftsSurviveTheCountCap() {
        let now = Date(timeIntervalSince1970: 2_000_000)
        var all: [String: PaneDraft] = [:]
        for index in 0..<(PaneDraftStore.keepAtMost + 10) {
            all["t\(index)"] = PaneDraft(
                text: "\(index)", savedAt: now.addingTimeInterval(TimeInterval(index)))
        }
        let pruned = PaneDraftStore.prune(all, now: now)
        #expect(pruned.count == PaneDraftStore.keepAtMost)
        // The last one written is the one most likely to be on screen.
        #expect(pruned["t\(PaneDraftStore.keepAtMost + 9)"] != nil)
        #expect(pruned["t0"] == nil)
    }

    /// Writing prunes, so the file on disk is bounded and not merely what is
    /// read back out of it.
    @Test func recordingPrunesWhatIsOnDisk() {
        let store = defaults()
        let then = Date(timeIntervalSince1970: 1_000_000)
        PaneDraftStore.record("ancient", forPane: "old", in: store, now: then)
        PaneDraftStore.record(
            "fresh", forPane: "new", in: store,
            now: then.addingTimeInterval(PaneDraftStore.keepFor + 1))
        #expect(PaneDraftStore.read(from: store).keys.sorted() == ["new"])
    }
}
