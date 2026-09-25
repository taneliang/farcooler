import AppKit
import Foundation
import SwiftUI
import Testing

@testable import Far_Cooler

/// What ⏎ and ⌥⏎ do in the ⌘N panel: the first starts the task and closes,
/// the second starts it and stays open for the next one.
///
/// Serialized because the panel's draft is `@AppStorage("tasks.draft")`, one
/// key in this test process's defaults, and two tests writing it at once
/// would read each other's.
@MainActor
@Suite(.serialized)
struct QuickCreateTests {
    private final class Outcome {
        var started: [String] = []
        var names: [String] = []
        var closed = 0
    }

    private func panel(_ outcome: Outcome) -> QuickCreate {
        let repository = Repository(
            id: "r1", short: "r1", displayName: "overnight", remote: "", repositoryRootId: "root")
        return QuickCreate(
            projects: [(host: "", repository: repository)],
            project: .constant("r1"),
            onSubmit: { description, name, _, _, _ in
                outcome.started.append(description)
                outcome.names.append(name)
            },
            onResume: {},
            onClose: { outcome.closed += 1 },
            namer: TaskNamer(model: nil))
    }

    private func withDraft(_ draft: String, _ body: () -> Void) {
        let defaults = UserDefaults.standard
        let saved = defaults.string(forKey: "tasks.draft")
        defaults.set(draft, forKey: "tasks.draft")
        body()
        if let saved { defaults.set(saved, forKey: "tasks.draft") } else {
            defaults.removeObject(forKey: "tasks.draft")
        }
    }

    @Test func returnStartsTheTaskAndClosesThePanel() {
        let outcome = Outcome()
        withDraft("Fix the flaky reconnect test") {
            panel(outcome).submit(keepOpen: false)
            #expect(UserDefaults.standard.string(forKey: "tasks.draft") == "")
        }
        #expect(outcome.started == ["Fix the flaky reconnect test"])
        // The short name, not the description: the directory and the sidebar
        // row are this, and the agent still gets every word above.
        #expect(outcome.names == ["fix-flaky-reconnect-test"])
        #expect(outcome.closed == 1)
    }

    @Test func optionReturnStartsTheTaskAndKeepsThePanelOpen() {
        let outcome = Outcome()
        withDraft("Fix the flaky reconnect test") {
            panel(outcome).submit(keepOpen: true)
            #expect(UserDefaults.standard.string(forKey: "tasks.draft") == "")
        }
        #expect(outcome.started.count == 1)
        #expect(outcome.closed == 0)
    }

    @Test func aDraftThatCannotStartNeitherStartsNorCloses() {
        let outcome = Outcome()
        withDraft("") { panel(outcome).submit(keepOpen: false) }
        #expect(outcome.started.isEmpty)
        #expect(outcome.closed == 0, "the panel stays for the person to finish typing")
    }

    /// The key itself: Option is what separates the two.
    @Test func optionIsWhatTheComposerReportsAsKeepOpen() throws {
        let view = SubmittingTextView()
        var seen: [Bool] = []
        view.onSubmit = { seen.append($0) }
        for flags: NSEvent.ModifierFlags in [[], [.option]] {
            let event = try #require(
                NSEvent.keyEvent(
                    with: .keyDown, location: .zero, modifierFlags: flags, timestamp: 0,
                    windowNumber: 0, context: nil, characters: "\r",
                    charactersIgnoringModifiers: "\r", isARepeat: false, keyCode: 36))
            view.keyDown(with: event)
        }
        #expect(seen == [false, true])
    }
}
