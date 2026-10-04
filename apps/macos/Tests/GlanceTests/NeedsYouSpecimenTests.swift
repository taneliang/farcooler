import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// The Needs You page and the History page's area chips (ov-225): items as cards
/// on the plane, chips as pills. A rendering, written by `VisualSpecimen`.
@MainActor
struct NeedsYouSpecimenTests {
    @Test("Write the Needs You sheet")
    func writeSheet() throws {
        try VisualSpecimen.shoot("needs-you", size: CGSize(width: 820, height: 500), Self.page())
        try VisualSpecimen.shoot("history-chips", size: CGSize(width: 520, height: 70), Chips())
    }

    private struct Chips: View {
        @State private var chosen: String? = "Mac"
        var body: some View {
            AreaChips(areas: ["Mac", "iOS", "Android", "Daemon", "Relay"], chosen: $chosen).padding(16)
        }
    }

    private static func item(
        _ kind: NeedsYouKind, _ question: String, actions: [NeedsYouAction] = [], askID: String? = nil
    ) -> NeedsYouItem {
        NeedsYouItem(
            id: "\(kind.rawValue):\(question)", kind: kind, rank: 1, since: Date().addingTimeInterval(-540), workspaceID: "ws",
            workspaceName: "Billing", repositoryID: "r",
            task: NeedsYouTask(id: "t-9", key: "bil-9", title: "Invoice PDF export", status: "in_review"),
            terminal: NeedsYouTerminal(
                id: "t", worktreeID: "w", label: "claude", role: "agent", paneMode: "terminal", chatCapable: true),
            question: question, askID: askID, actions: actions)
    }

    private static func page() -> some View {
        NeedsYouView(
            items: [
                item(
                    .ask, "Allow touch x?",
                    actions: [
                        NeedsYouAction(id: "deny", title: "Deny", destructive: true, primary: false),
                        NeedsYouAction(id: "allow", title: "Allow", destructive: false, primary: true),
                    ], askID: "a1"),
                item(.decision, "Which store should the exports use?"),
                item(.review, "Review the Changes pane conversion"),
            ],
            olderRunners: [], canAct: { _ in true }, onOpen: { _ in }, onAnswerAsk: { _, _ in nil }, onDecide: { _, _ in true })
    }
}
