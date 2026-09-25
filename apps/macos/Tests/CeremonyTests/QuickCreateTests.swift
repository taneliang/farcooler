import AppKit
import Foundation
import SwiftUI
import Testing

@testable import Far_Cooler

/// What ⏎ and ⌥⏎ do in the ⌘N panel: the first starts the task and closes,
/// the second starts it and stays open for the next one — and neither lets go
/// of the draft until the task exists.
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
        /// What the start answers: nil is "started", a sentence is a failure.
        var failure: String?
        /// Run while the start is in flight, before it answers.
        var during: (@MainActor () -> Void)?
    }

    private func panel(_ outcome: Outcome, submission: TaskSubmission = TaskSubmission())
        -> QuickCreate
    {
        let repository = Repository(
            id: "r1", short: "r1", displayName: "overnight", remote: "", repositoryRootId: "root")
        return QuickCreate(
            projects: [(host: "", repository: repository)],
            project: .constant("r1"),
            onSubmit: { request in
                outcome.started.append(request.description)
                outcome.names.append(request.name)
                outcome.during?()
                if let failure = outcome.failure { return .failed(failure) }
                return .started(name: request.name)
            },
            onResume: {},
            onClose: { outcome.closed += 1 },
            namer: TaskNamer(model: nil),
            submission: submission)
    }

    private var draft: String? { UserDefaults.standard.string(forKey: "tasks.draft") }

    private func withDraft(_ text: String, _ body: () async -> Void) async {
        let defaults = UserDefaults.standard
        let saved = defaults.string(forKey: "tasks.draft")
        defaults.set(text, forKey: "tasks.draft")
        await body()
        if let saved { defaults.set(saved, forKey: "tasks.draft") } else {
            defaults.removeObject(forKey: "tasks.draft")
        }
    }

    @Test func returnStartsTheTaskAndClosesThePanel() async {
        let outcome = Outcome()
        await withDraft("Fix the flaky reconnect test") {
            await panel(outcome).submit(keepOpen: false)
            #expect(draft == "")
        }
        #expect(outcome.started == ["Fix the flaky reconnect test"])
        // The short name, not the description: the directory and the sidebar
        // row are this, and the agent still gets every word above.
        #expect(outcome.names == ["fix-flaky-reconnect-test"])
        #expect(outcome.closed == 1)
    }

    @Test func optionReturnStartsTheTaskAndKeepsThePanelOpen() async {
        let outcome = Outcome()
        await withDraft("Fix the flaky reconnect test") {
            await panel(outcome).submit(keepOpen: true)
            #expect(draft == "")
        }
        #expect(outcome.started.count == 1)
        #expect(outcome.closed == 0)
    }

    @Test func aStartThatFailsKeepsTheDraftAndThePanelAndSaysWhy() async {
        let outcome = Outcome()
        outcome.failure = "Can’t reach this runner right now, so the task wasn’t started."
        let submission = TaskSubmission()
        await withDraft("Fix the flaky reconnect test") {
            await panel(outcome, submission: submission).submit(keepOpen: false)
            #expect(draft == "Fix the flaky reconnect test", "the draft survives a failed start")
        }
        #expect(outcome.closed == 0, "the panel stays to say why")
        #expect(submission.failure == outcome.failure)
        #expect(!submission.starting)
    }

    /// ⏎, Esc while it starts, ⌘N again: the first start finishing must not
    /// close the panel the person has just opened to write the next task.
    @Test func aStartThatFinishesLateDoesNotCloseAPanelOpenedSince() async {
        let outcome = Outcome()
        let submission = TaskSubmission()
        outcome.during = { submission.opened() }
        await withDraft("Fix the flaky reconnect test") {
            await panel(outcome, submission: submission).submit(keepOpen: false)
        }
        #expect(outcome.started.count == 1)
        #expect(outcome.closed == 0, "a later opening is not this start's to close")
    }

    @Test func aDraftThatCannotStartNeitherStartsNorCloses() async {
        let outcome = Outcome()
        for text in ["", "?!…", String(repeating: "x", count: TaskPrompt.maxBytes + 1)] {
            await withDraft(text) { await panel(outcome).submit(keepOpen: false) }
        }
        #expect(outcome.started.isEmpty)
        #expect(outcome.closed == 0, "the panel stays for the person to finish typing")
    }

    // MARK: - The keys

    private func returnKey(_ flags: NSEvent.ModifierFlags) throws -> NSEvent {
        try #require(
            NSEvent.keyEvent(
                with: .keyDown, location: .zero, modifierFlags: flags, timestamp: 0,
                windowNumber: 0, context: nil, characters: "\r",
                charactersIgnoringModifiers: "\r", isARepeat: false, keyCode: 36))
    }

    /// Option is what separates the two.
    @Test func optionIsWhatTheComposerReportsAsKeepOpen() throws {
        let view = SubmittingTextView()
        var seen: [Bool] = []
        view.onSubmit = { seen.append($0) }
        for flags: NSEvent.ModifierFlags in [[], [.option]] {
            view.keyDown(with: try returnKey(flags))
        }
        #expect(seen == [false, true])
    }

    /// Return while an input method is composing commits the candidate; it is
    /// not a send.
    @Test func returnDuringInputMethodCompositionIsNotASubmit() throws {
        let view = SubmittingTextView()
        var seen: [Bool] = []
        view.onSubmit = { seen.append($0) }
        view.setMarkedText(
            "にほ", selectedRange: NSRange(location: 2, length: 0),
            replacementRange: NSRange(location: NSNotFound, length: 0))
        #expect(view.hasMarkedText())
        view.keyDown(with: try returnKey([]))
        #expect(seen.isEmpty)
    }

    /// The whole path, through a real window: ⌥⏎ in the panel's own text view
    /// reaches `submit(keepOpen: true)`.
    @Test func optionReturnInThePanelsOwnFieldKeepsItOpen() async throws {
        let outcome = Outcome()
        await withDraft("Fix the flaky reconnect test") {
            let host = NSHostingView(rootView: panel(outcome))
            let window = NSWindow(
                contentRect: NSRect(x: 0, y: 0, width: 600, height: 200),
                styleMask: [.titled], backing: .buffered, defer: false)
            // Owned by this test, not by AppKit: a programmatic window left
            // to release itself on close is released twice.
            window.isReleasedWhenClosed = false
            window.contentView = host
            host.layoutSubtreeIfNeeded()
            guard let field = Self.find(SubmittingTextView.self, in: host) else {
                Issue.record("the panel has no composer")
                return
            }
            field.keyDown(with: try! returnKey([.option]))
            for _ in 0..<100 where outcome.started.isEmpty {
                try? await Task.sleep(for: .milliseconds(10))
            }
            window.close()
        }
        #expect(outcome.started == ["Fix the flaky reconnect test"])
        #expect(outcome.closed == 0)
    }

    private static func find<T: NSView>(_ type: T.Type, in view: NSView) -> T? {
        if let match = view as? T { return match }
        for sub in view.subviews { if let found = find(type, in: sub) { return found } }
        return nil
    }
}
