import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// The navigator's rows (ov-222): the filter, the orchestrator's row, task rows
/// as they are selected, keyed, highlighted and plain, a History row and the
/// layout pills, on the plane. A rendering, written by `VisualSpecimen`.
@MainActor
struct NavigatorSpecimenTests {
    @Test("Write the navigator sheet")
    func writeSheet() throws {
        try VisualSpecimen.shoot("navigator", size: CGSize(width: 340, height: 560), Specimen())
    }

    private struct Specimen: View {
        @FocusState private var focused: Bool
        @State private var text = ""

        var body: some View {
            VStack(alignment: .leading, spacing: 10) {
                NavigatorFilterField(text: $text, focused: $focused, onLeave: {})
                    .padding(.horizontal, NavigatorGrid.edge)
                OrchestratorRowView(
                    model: NavigatorOrchestrator(state: .needsYou, agent: "claude", status: .blocked, nowDoing: "Reading the board"),
                    inProgress: 2, selected: false, keyed: false
                )
                .padding(.horizontal, NavigatorGrid.edge)
                CompactTaskRow(key: "ov-220", title: "Mac: window plane and sidebar go frosted", selected: true, keyed: true) {
                    Text("In Progress").font(.caption).foregroundStyle(.secondary)
                }
                .padding(.horizontal, NavigatorGrid.edge)
                CompactTaskRow(key: "ov-221", title: "Mac: pane cards and headers adopt tokens", selected: true, keyed: false) {
                    Text("In Progress").font(.caption).foregroundStyle(.secondary)
                }
                .padding(.horizontal, NavigatorGrid.edge)
                CompactTaskRow(key: "ov-222", title: "Mac: navigator rows share one selection", highlighted: true) {
                    Text("Backlog").font(.caption).foregroundStyle(.secondary)
                }
                .padding(.horizontal, NavigatorGrid.edge)
                CompactTaskRow(key: "ov-223", title: "Mac: task view and agent chat adopt tokens", emphasized: true) {
                    Text("Needs Decision").font(.caption).foregroundStyle(.secondary)
                }
                .padding(.horizontal, NavigatorGrid.edge)
                HistoryRow(status: .done, total: 94, action: {})
                    .padding(.horizontal, NavigatorGrid.edge)
                GroupBar(
                    groups: ["a", "b", "c"].map {
                        PaneGroup(id: $0, name: $0, active: $0 == "a", columns: 80, rows: 24, layout: "x", panes: [])
                    },
                    showing: "a", onSelect: { _ in })
                    .padding(.horizontal, NavigatorGrid.edge)
                Spacer()
            }
            .padding(.top, 12)
            .environment(\.taskKeyWidth, 52)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background { WindowPlane() }
        }
    }
}
