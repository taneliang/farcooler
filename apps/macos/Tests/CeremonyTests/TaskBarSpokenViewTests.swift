import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// The task bar's tooltips and VoiceOver strings, read off the real view
/// (ov-603). `TaskBarSpokenTests` checks the strings as statics, which can't
/// tell that a button stopped using them; here the hosted bar reports what each
/// control draws through `.spoken`, so a button that drops the modifier or
/// passes a different string fails.
@MainActor
struct TaskBarSpokenViewTests {
    /// What the bar's controls report once the preference has settled.
    static func spoken<V: View>(_ view: V) async -> [SpokenEntry] {
        var got: [SpokenEntry] = []
        let size = CGSize(width: 900, height: 200)
        let host = NSHostingView(rootView: view.frame(width: size.width, height: size.height).onPreferenceChange(SpokenStrings.self) { got = $0 })
        host.frame = CGRect(origin: .zero, size: size)
        let window = NSWindow(contentRect: host.frame, styleMask: [.borderless], backing: .buffered, defer: true)
        window.contentView = host
        // Until the report has held the same through a further step.
        var seen: [SpokenEntry] = [], steady = 0
        await Settle.until {
            host.layoutSubtreeIfNeeded()
            steady = !got.isEmpty && got == seen ? steady + 1 : 0
            seen = got
            return steady >= 3
        }
        return got
    }

    private func entry(_ entries: [SpokenEntry], _ label: String) throws -> SpokenEntry {
        try #require(entries.first { $0.label == label }, "no control says “\(label)”; it says \(entries.map(\.label))")
    }

    @Test("In every form, the New Terminal and Open Worktree buttons and the agent picker say their label, value and tooltip")
    func controlsSpeak() async throws {
        let name = Terminal.name(of: "claude")
        for form in [TaskTabBar.Form.full, .compact, .tight] {
            let entries = await Self.spoken(TaskViewFormTests.bar(agents: 2).bar(form))
            #expect(entries.count == 3, "\(form): \(entries.map(\.label))")

            let terminal = try entry(entries, "Open Terminal in This Task’s Worktree")
            let note = form == .full ? "" : "2 terminals open"
            #expect(terminal.value == note, "\(form)")
            #expect(
                terminal.help == "Open a terminal in this task’s worktree." + (form == .full ? "" : " 2 terminals open."),
                "\(form)")

            let open = try entry(entries, "Open Worktree")
            // New Terminal is on the bar, so it carries the count; Open Worktree doesn't.
            #expect(open.value == "", "\(form)")
            #expect(open.help == "Show this task’s worktree full size, with all its layouts", "\(form)")

            let picker = try entry(entries, "Choose Agent")
            #expect(picker.value == name, "\(form)")
            #expect(picker.help == "Choose which agent to show. Showing \(name).", "\(form)")
        }
    }

    @Test("With one agent there's no picker to speak, and the buttons still do")
    func onePickerFewer() async throws {
        let entries = await Self.spoken(TaskViewFormTests.bar(agents: 1).bar(.tight))
        #expect(entries.map(\.label) == ["Open Terminal in This Task’s Worktree", "Open Worktree"])
    }
}
