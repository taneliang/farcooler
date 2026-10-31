import SwiftUI

/// The workspace's terminals in the toolbar (ov-867, C10f): `>_ 6`, a menu of
/// every pane this workspace has but its orchestrator, each opening its real,
/// typeable terminal in the plan's place (`go(to:)`). Drawn only with a
/// workspace on screen that has one.
struct TerminalsToolbar: ToolbarContent {
    let panes: [BoardPane]
    let open: (BoardPane) -> Void

    var body: some ToolbarContent {
        if !panes.isEmpty {
            ToolbarItem(placement: .automatic) {
                Menu {
                    let titles = BoardPane.titles(panes)
                    ForEach(panes.indices, id: \.self) { index in
                        Button(titles[index]) { open(panes[index]) }
                    }
                } label: {
                    Label(Self.label(panes.count), systemImage: "apple.terminal")
                        .labelStyle(.titleAndIcon)
                }
                .menuIndicator(.hidden)
                .help("Open one of this workspace’s terminals")
                .accessibilityLabel(Self.accessibilityLabel(panes.count))
                .accessibilityIdentifier("toolbar-terminals")
            }
        }
    }

    /// The count beside the glyph.
    static func label(_ count: Int) -> String { "\(count)" }

    /// "6 Terminals", "1 Terminal".
    static func accessibilityLabel(_ count: Int) -> String { count == 1 ? "1 Terminal" : "\(count) Terminals" }

    /// What the toolbar keeps room for: the glyph, the count and the capsule.
    static func width(_ count: Int) -> CGFloat {
        count == 0 ? 0 : 36 + TitleStatus.textWidth(label(count)) + 8
    }
}
