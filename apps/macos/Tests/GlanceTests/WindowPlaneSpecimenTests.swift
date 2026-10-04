import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// The window plane (ov-220): the navigator and the gutters on the plane, the
/// work as opaque paper beside it. A rendering, written by `VisualSpecimen`.
@MainActor
struct WindowPlaneSpecimenTests {
    @Test("Write the window plane sheet")
    func writeSheet() throws {
        try VisualSpecimen.shoot("window-plane", size: CGSize(width: 900, height: 520), Self.window())
    }

    private static func window() -> some View {
        @State var width = 280.0
        return WorkspaceView(
            opened: "task" as String?, hasConversation: true, cell: 8, focused: false,
            navigatorWidth: Binding(get: { 280 }, set: { _ in }),
            conversation: { Text("Orchestrator").frame(maxWidth: .infinity, maxHeight: .infinity) },
            navigator: {
                VStack(alignment: .leading, spacing: 8) {
                    Text("Board").font(.headline)
                    ForEach(["ov-220 Frosted window plane", "ov-221 Pane cards", "ov-222 Navigator rows"], id: \.self) {
                        Text($0).font(.callout)
                    }
                    Spacer()
                }
                .padding(12)
                .frame(maxWidth: .infinity, alignment: .leading)
            },
            breadcrumb: { _ in
                Text("Main  /  ov-220 Frosted window plane").font(.system(size: 11)).foregroundStyle(.secondary)
                    .columnHeader()
            },
            detail: { _, _ in
                Text("The task document is paper: opaque, so text reads over any wallpaper.")
                    .padding(16)
                    .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
                    .background(WorkspaceStyle.document)
            })
    }
}
