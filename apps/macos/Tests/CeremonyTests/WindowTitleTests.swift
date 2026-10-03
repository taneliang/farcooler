import Testing

@testable import Far_Cooler

/// ov-81 P10: the window's title is the leaf of the breadcrumb.
struct WindowTitleTests {
    private func title(leaf: String?, implicit: Bool = false) -> (title: String, subtitle: String) {
        ContentView.workspaceTitle(
            workspace: "Billing", repository: "shop", host: "mini", leaf: leaf, implicit: implicit)
    }

    @Test("In the workspace itself the title is the workspace")
    func workspaceLevel() {
        let t = title(leaf: nil)
        #expect(t.title == "Billing")
        #expect(t.subtitle == "shop · mini")
    }

    @Test("A task or a worktree opened is the title, with where it is beneath")
    func leafLevels() {
        let task = title(leaf: "bil-1 Invoice PDF export")
        #expect(task.title == "bil-1 Invoice PDF export")
        #expect(task.subtitle == "Billing · shop")
        let worktree = title(leaf: "invoice-pdf")
        #expect(worktree.title == "invoice-pdf")
        #expect(worktree.subtitle == "Billing · shop")
        // A runner without workspaces has no workspace name to repeat.
        #expect(title(leaf: "invoice-pdf", implicit: true).subtitle == "shop")
    }
}

/// ov-105: the toolbar never shows the title; the switcher and the
/// breadcrumb already name every place it could.
struct TitleBarTests {
    @Test("No selection kind shows a title beside the switcher")
    func noKindShowsATitle() {
        let kinds: [ContentView.Selection?] = [
            nil, .needsYou, .workspace(host: "", workspace: "ws", focus: nil),
            .workspace(host: "", workspace: "ws", focus: .task("t1")),
            .workspace(host: "", workspace: "ws", focus: .worktree("w1", terminal: nil)),
            .looseWorktree(host: "", worktree: "w1", terminal: nil),
        ]
        for kind in kinds { #expect(!TitleBar.showsTitle(for: kind), "\(String(describing: kind))") }
    }
}
