import AgentKit
import Foundation
import Testing

@testable import Far_Cooler

/// The one rule `AgentComposer`'s own draft wiring adds on top of
/// `PaneDraftStore` — the store itself is AgentKit's, tested exhaustively in
/// `PaneDraftTests`, and untouched here beyond being made `public`.
///
/// **Restoring must happen only into an EMPTY field.** A pane's `terminal.id`
/// changing under a composer that still has unsent text in it — the same
/// `AgentSurface` reused across a pane switch — must not clobber what somebody
/// is mid-sentence typing with a draft saved from before. `restoredDraft`
/// carries exactly that guard, pulled out of the view's `.task(id:)` so this
/// suite can break it without standing up a window.
@MainActor
struct ComposerDraftTests {
    private func defaults() -> UserDefaults {
        let suite = "ComposerDraftTests.\(UUID().uuidString)"
        guard let defaults = UserDefaults(suiteName: suite) else {
            fatalError("no defaults for suite \(suite)")
        }
        return defaults
    }

    @Test func anEmptyFieldIsFilledFromTheStore() {
        let store = defaults()
        PaneDraftStore.record("half a thought", forPane: "t1", in: store)
        #expect(restoredDraft(intoText: "", forPane: "t1", from: store) == "half a thought")
    }

    /// The failure this exists to prevent: a draft landing on top of live
    /// typing rather than into an empty field.
    @Test func aFieldThatAlreadyHasTextIsLeftAlone() {
        let store = defaults()
        PaneDraftStore.record("saved earlier", forPane: "t1", in: store)
        #expect(restoredDraft(intoText: "still typing this", forPane: "t1", from: store) == nil)
    }

    @Test func aPaneWithNoDraftLeavesAnEmptyFieldEmpty() {
        #expect(restoredDraft(intoText: "", forPane: "never", from: defaults()) == nil)
    }

    /// The other deliberate property: keyed per pane, not shared. A composer
    /// asking for one pane's draft must never be handed another's.
    @Test func restoringOnePaneDoesNotReachAnothersDraft() {
        let store = defaults()
        PaneDraftStore.record("for one", forPane: "t1", in: store)
        PaneDraftStore.record("for two", forPane: "t2", in: store)
        #expect(restoredDraft(intoText: "", forPane: "t1", from: store) == "for one")
        #expect(restoredDraft(intoText: "", forPane: "t2", from: store) == "for two")
    }
}
