import Foundation
import Testing

@testable import AgentKit

/// The iPad's workspace (ov-348): which windows get columns, how wide each
/// is, and where a pick in the tree goes.
struct PadLayoutTests {
    static let place = PhoneWorkspace(runner: "r1", workspace: "ws-billing")

    // MARK: Which layout

    /// **Columns only on an iPad at regular width**: an iPhone in landscape
    /// is regular too, and stays a phone; a Split View's compact width is a
    /// phone; a runner without workspaces has no plan to put in a column.
    @Test func columnsAreOnlyAnIPadsAtRegularWidth() {
        #expect(PadLayout.of(isPad: true, regularWidth: true, width: 1210, implicit: false) == .threeColumns)
        #expect(PadLayout.of(isPad: false, regularWidth: true, width: 932, implicit: false) == .phone)
        #expect(PadLayout.of(isPad: true, regularWidth: false, width: 1210, implicit: false) == .phone)
        #expect(PadLayout.of(isPad: true, regularWidth: true, width: 1210, implicit: true) == .phone)
    }

    /// **The edges**: three columns from 1,000 points (a 13-inch portrait,
    /// 1,032), two from 700 (an 11-inch portrait, 834), and a phone below.
    @Test func theWidthsEdges() {
        func layout(_ width: Double) -> PadLayout {
            PadLayout.of(isPad: true, regularWidth: true, width: width, implicit: false)
        }
        #expect(layout(1032) == .threeColumns)
        #expect(layout(1000) == .threeColumns)
        #expect(layout(999) == .twoColumns)
        #expect(layout(834) == .twoColumns)
        #expect(layout(700) == .twoColumns)
        #expect(layout(699) == .phone)
    }

    /// **The chat keeps a terminal's width**: in every window that gets
    /// columns, the chat is at least 340 points, about 45 columns of text,
    /// and the columns fill the window exactly.
    @Test func theChatKeepsATerminalsWidth() {
        for width in stride(from: 700.0, through: 1400, by: 10) {
            let layout = PadLayout.of(isPad: true, regularWidth: true, width: width, implicit: false)
            let w = layout.widths(width)
            #expect(abs(w.tree + w.plan + w.chat - width) < 0.001, "\(width)")
            #expect(w.chat >= 340, "the chat is \(w.chat) at \(width)")
            #expect(w.plan >= 340, "the plan is \(w.plan) at \(width)")
            if layout == .threeColumns { #expect(w.tree >= 240, "the tree is \(w.tree) at \(width)") }
            if layout == .twoColumns { #expect(w.tree == 0) }
        }
    }

    // MARK: Picks

    /// **A pick shows in the plan column**: a theme, a lane, a page, a task
    /// and the plan itself. A worktree or terminal covers the stack as on the
    /// phone; Needs You is the stack's root; a subagent goes nowhere.
    @Test func aPickShowsInThePlanColumn() {
        let place = Self.place
        #expect(PadPick.of(.plan, in: place) == .canvas(.plan))
        #expect(PadPick.of(.theme("t1"), in: place) == .canvas(.page(.theme("t1"))))
        #expect(PadPick.of(.lane("l1"), in: place) == .canvas(.page(.lane("l1"))))
        #expect(PadPick.of(.page("p1"), in: place) == .canvas(.page(.page("p1"))))
        #expect(PadPick.of(.task("bil-9"), in: place) == .canvas(.task("bil-9")))
        #expect(PadPick.of(.needsYou, in: place) == .needsYou)
        #expect(PadPick.of(.orchestrator, in: place) == .none)
        #expect(
            PadPick.of(.worktree("w1"), in: place)
                == .open(.worktree(runner: "r1", worktree: "w1", landing: .resume)))
        #expect(
            PadPick.of(.terminal(worktree: "w1", terminal: "t9"), in: place)
                == .open(.worktree(runner: "r1", worktree: "w1", landing: .terminal("t9"))))
    }

    /// **The chosen row is the canvas's**: each canvas a pick can show maps
    /// back to the target it came from, and the board, a toolbar item, to none.
    @Test func theChosenRowIsTheCanvass() {
        for target: OneTreeTarget in [.plan, .theme("t1"), .lane("l1"), .page("p1"), .task("bil-9")] {
            guard case .canvas(let canvas) = PadPick.of(target, in: Self.place) else {
                Issue.record("\(target) isn't a canvas")
                continue
            }
            #expect(PadPick.target(of: canvas) == target)
        }
        #expect(PadPick.target(of: .board) == nil)
    }

    /// **Open rows are kept per workspace**, and another workspace's are its own.
    @Test func openRowsAreKeptPerWorkspace() throws {
        let defaults = try #require(UserDefaults(suiteName: "PadLayoutTests"))
        defaults.removePersistentDomain(forName: "PadLayoutTests")
        let theme = OneTreeNode(id: "theme:t1", kind: .theme, title: "Phones", children: [
            OneTreeNode(id: "theme:t1/task:a", kind: .task, title: "A")
        ])
        var open = PadTreeExpansion.remembered(Self.place, in: defaults)
        #expect(!open.isExpanded(theme))
        open.toggle(theme)
        PadTreeExpansion.remember(open, for: Self.place, in: defaults)
        #expect(PadTreeExpansion.remembered(Self.place, in: defaults).isExpanded(theme))
        let other = PhoneWorkspace(runner: "r1", workspace: "ws-other")
        #expect(!PadTreeExpansion.remembered(other, in: defaults).isExpanded(theme))
    }
}
