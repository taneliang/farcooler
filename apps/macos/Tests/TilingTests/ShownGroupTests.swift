import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// Which layout is on screen, and when it stops being the runner's decision.
///
/// A workspace can hold several layouts — a terminal IS a tmux window and a
/// window IS a layout — and only one of them is drawn. That used to be whichever
/// one tmux called active, which is a fact the app learns by asking: selecting a
/// terminal in another layout sends `layout focus` to the runner, and the runner
/// goes on calling the OLD layout active until it answers. Measured on a quiet
/// local runner, the app spent 123ms and 269ms of that round trip drawing
/// somebody else's terminal — live, correct, and indistinguishable from the one
/// that had been asked for.
///
/// None of that is visible from a still. A pane showing the wrong terminal looks
/// exactly like a pane showing the right one, which is why it is tested here
/// rather than looked at.
@MainActor
struct ShownGroupTests {
    private func pane(_ id: String, focused: Bool = false) -> PaneRect {
        PaneRect(
            id: id, short: String(id.prefix(8)), title: nil, left: 0, top: 0,
            columns: 80, rows: 24, focused: focused, zoomed: false)
    }

    private func group(_ id: String, active: Bool, _ terminals: [String]) -> PaneGroup {
        PaneGroup(
            id: id, name: "fish", active: active, columns: 80, rows: 24,
            layout: "\(id)-layout", panes: terminals.map { pane($0) })
    }

    private var twoLayouts: [PaneGroup] {
        [group("@1", active: true, ["alpha"]), group("@3", active: false, ["beta"])]
    }

    @Test func the_layout_asked_for_wins_over_the_one_the_runner_calls_active() {
        let shown = twoLayouts.showing("@3")
        #expect(shown?.id == "@3", "drew the runner's active layout instead of the one asked for")
    }

    @Test func the_active_layout_is_what_a_workspace_with_no_choice_falls_back_to() {
        // No terminal selected, so nothing has named a layout. The runner's
        // answer is the right one here and always was.
        #expect(twoLayouts.showing("")?.id == "@1")
        // And a layout that has since been closed must not blank the pane.
        #expect(twoLayouts.showing("@99")?.id == "@1")
    }

    @Test func a_workspace_whose_layouts_all_deny_being_active_still_draws_one() {
        let none = [group("@1", active: false, ["alpha"]), group("@3", active: false, ["beta"])]
        #expect(none.showing("@99")?.id == "@1")
        #expect([PaneGroup]().showing("@1") == nil)
    }

    /// A layout changing shape is motion worth watching; one layout replacing
    /// another is not.
    ///
    /// The panes of the layout you leave spend the spring's whole settling time
    /// fading out over the one you asked for. Measured by ablation, on the same
    /// fixture and the same script: with the animation applied across a layout
    /// change the outgoing pane's view lived a median 427ms past the incoming
    /// one's mount (406-763ms); without it, 4.9ms (2.7-66.5ms).
    @Test func a_new_layout_arrives_without_animation_and_a_reshaped_one_with_it() {
        #expect(
            TileView.arrangementMotion(from: "@1", to: "@3") == nil,
            "one layout replacing another was animated, which is the outgoing terminal left on screen for the length of the spring"
        )
        #expect(TileView.arrangementMotion(from: "@1", to: "@1") != nil)
    }
}
