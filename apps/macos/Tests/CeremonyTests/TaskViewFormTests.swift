import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// Which form the task header and tab bar draw at a width (ov-601): each
/// form is chosen where it is the widest that fits, in order, and the title
/// is whole in every width the last form fits.
@MainActor
struct TaskViewFormTests {
    static let pane = BoardPane(terminal: TaskViewFitTests.worktree.terminals[0], worktree: TaskViewFitTests.worktree)
    static let longTitle = "Total each invoice and write down every cent"

    static func row(_ title: String = longTitle) -> TaskRow {
        TaskRow(id: "t", key: "bil-128", title: title, status: .inProgress, statusSince: Date())
    }

    static func header(_ title: String = longTitle) -> TaskViewHeader { TaskViewHeader(row: row(title), agent: pane) }

    static func bar(agents: Int) -> TaskTabBar {
        TaskTabBar(
            tab: .agent, onChoose: { _ in }, worktree: TaskViewFitTests.worktree,
            agents: Array(repeating: pane, count: agents), chosen: pane, onNewTerminal: {})
    }

    /// The width `view` asks for with room to spare.
    static func ideal<V: View>(_ view: V) -> CGFloat {
        NSHostingView(rootView: view.fixedSize(horizontal: true, vertical: false)).fittingSize.width
    }

    /// The form a `ViewThatFits` inside `view` draws in a column `width` wide.
    /// Waits on the condition, not a delay: the run loop is spun in small
    /// steps until the preference has reported and then held the same through
    /// a further step, up to `deadline` seconds (a loaded runner is slow, not
    /// wrong), and the test fails with the width if it never does.
    static func chosen<V: View>(_ view: V, width: CGFloat) -> String {
        var form = ""
        let host = NSHostingView(
            rootView: view.frame(width: width, height: 200).onPreferenceChange(ChosenForm.self) { form = $0 })
        host.frame = CGRect(x: 0, y: 0, width: width, height: 200)
        let window = NSWindow(contentRect: host.frame, styleMask: [.borderless], backing: .buffered, defer: true)
        window.contentView = host
        let deadline = Date().addingTimeInterval(2)
        var seen = "", steady = 0
        while Date() < deadline, steady < 2 {
            host.layoutSubtreeIfNeeded()
            RunLoop.main.run(until: Date().addingTimeInterval(0.002))
            steady = !form.isEmpty && form == seen ? steady + 1 : 0
            seen = form
        }
        if steady < 2 { Issue.record("no form settled in 2 s at \(width) pt (last read: \"\(form)\")") }
        return form
    }

    /// Whether `width` is within the 2 pt AppKit and SwiftUI may disagree by
    /// of where a form starts to fit.
    static func nearEdge(_ width: CGFloat, _ edges: [CGFloat]) -> Bool { edges.contains { abs($0 - width) < 3 } }

    /// Each form's width with the header's own margins, widest first.
    static func headerIdeals(_ title: String = longTitle) -> [(form: TaskViewHeader.Form, width: CGFloat)] {
        let margin = 2 * TaskTypography.inset.leading
        return [TaskViewHeader.Form.full, .markOnly, .essentials].map {
            ($0, ideal(header(title).header($0)) + margin)
        }
    }

    @Test("The header draws the widest form that fits, and sheds the agent's words, then the status, in that order")
    func headerFormByWidth() {
        let ideals = Self.headerIdeals()
        #expect(ideals[0].width > ideals[1].width && ideals[1].width > ideals[2].width, "the forms don't shed anything: \(ideals)")
        for width in stride(from: ideals[2].width, through: ideals[0].width + 40, by: 2)
        where !Self.nearEdge(width, ideals.map(\.width)) {
            let want = ideals.first { $0.width <= width }!.form
            let got = Self.chosen(Self.header(), width: width)
            #expect(got == "header.\(want)", "at \(width) drew \(got), wanted \(want): \(ideals)")
        }
    }

    @Test("The title is whole wherever the last form fits: it asks for its natural width, not a floor")
    func titleIsWholeToTheLastForm() {
        let natural = Self.ideal(Text(Self.longTitle).font(TaskTypography.title).lineLimit(1))
        #expect(natural > TaskViewHeader.titleLeast * 1.5, "the specimen title is too short to prove it")
        // A short title and a long one differ in the header's ideal by the
        // title's width over the floor.
        let short = Self.ideal(Self.header("Hi").header(.essentials))
        let long = Self.ideal(Self.header().header(.essentials))
        #expect(long - short >= natural - TaskViewHeader.titleLeast - 1, "the title asks for \(long - short + TaskViewHeader.titleLeast) of \(natural)")
        // And below that the last form is drawn, with the title squeezed no
        // further than its floor.
        let last = Self.headerIdeals()[2].width
        #expect(Self.chosen(Self.header(), width: last - 40) == "header.essentials")
        #expect(
            TaskViewFitTests.least(Self.header()) >= TaskViewHeader.titleLeast + 150,
            "the title can be squeezed below its floor: \(TaskViewFitTests.least(Self.header()))")
    }

    @Test("The bar draws the widest form that fits, for one agent and for several", arguments: [1, 2])
    func barFormByWidth(agents: Int) {
        let ideals = [TaskTabBar.Form.full, .compact, .tight].map { ($0, Self.ideal(Self.bar(agents: agents).bar($0))) }
        #expect(ideals[0].1 > ideals[1].1 && ideals[1].1 > ideals[2].1, "a form sheds nothing: \(ideals)")
        for width in stride(from: ideals[2].1, through: ideals[0].1 + 40, by: 2)
        where !Self.nearEdge(width, ideals.map(\.1)) {
            let want = ideals.first { $0.1 <= width }!.0
            let got = Self.chosen(Self.bar(agents: agents), width: width)
            #expect(got == "bar.\(want)", "at \(width) drew \(got), wanted \(want): \(ideals)")
        }
    }
}

/// What the controls say where their words are gone (ov-601). The strings
/// are `TaskTabBar`'s own statics, which its buttons and picker use.
@MainActor
struct TaskBarSpokenTests {
    @Test("Where the count isn't drawn, the New Terminal button's tooltip and value say it, right for 0, 1 and 2")
    func countMovesToTheButton() {
        #expect(TaskTabBar.terminalNote(count: 2, form: .full) == nil, "the full bar draws it")
        for form in [TaskTabBar.Form.compact, .tight] {
            #expect(TaskTabBar.terminalNote(count: 2, form: form) == "2 terminals open")
            #expect(TaskTabBar.terminalNote(count: 1, form: form) == "1 terminal open")
            #expect(TaskTabBar.terminalNote(count: 0, form: form) == "No terminals open")
        }
        #expect(
            TaskTabBar.newTerminalHelp(count: 2, form: .tight)
                == "Open a terminal in this task’s worktree. 2 terminals open.")
        #expect(TaskTabBar.newTerminalHelp(count: 2, form: .full) == "Open a terminal in this task’s worktree.")
    }

    @Test("The picker's tooltip names the agent showing, in every form")
    func pickerNamesTheAgent() {
        #expect(TaskTabBar.pickerHelp(showing: "claude") == "Choose which agent to show. Showing claude.")
    }
}
