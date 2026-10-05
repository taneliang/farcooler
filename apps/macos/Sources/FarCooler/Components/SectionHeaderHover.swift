import SwiftUI

// A section header's hover-only accessory (ov-177, round 2). The owner, on
// Unread's checkmark button: "it has no text. Can you make it a text button
// that is only visible when that Unread row is hovered over instead? That way
// we don't clutter the UI but also keep it understandable".
//
// A `CollapsibleSection` tells its accessory whether the pointer is over its
// header row (`sectionHeaderHovered`). An accessory that only shows on hover
// still has to be reachable without the pointer, so the section can also
// carry the same action on its header for VoiceOver (`SectionHeaderAction`).

private struct SectionHeaderHoveredKey: EnvironmentKey {
    static let defaultValue = false
}

extension EnvironmentValues {
    /// Whether the pointer is over the header row of the section this is
    /// the accessory of. Set by `CollapsibleSection` around its accessory
    /// only, never around its content.
    var sectionHeaderHovered: Bool {
        get { self[SectionHeaderHoveredKey.self] }
        set { self[SectionHeaderHoveredKey.self] = newValue }
    }
}

/// An action VoiceOver offers on a section's header (in its Actions rotor),
/// for what the header's accessory does on hover.
struct SectionHeaderAction {
    let name: String
    let perform: () -> Void
}

extension CollapsibleSection {
    /// This section, with `action` offered on its header to VoiceOver. Nil
    /// offers nothing.
    func headerAction(_ action: SectionHeaderAction?) -> Self {
        var copy = self
        copy.headerAction = action
        return copy
    }
}

/// Mark All as Read on Unread's header (ov-177): words, not a glyph, drawn
/// only while the pointer is over the header row, so the header is just its
/// title and count the rest of the time. "Mark These as Read" while the
/// navigator's filter narrows the strip, when it reads only what's listed.
///
/// Without the pointer it's the header's VoiceOver action (`headerAction`),
/// the header's context menu, and Board ▸ Mark All as Read (⇧⌘K). Each asks
/// first (`MarkReadConfirmation`, ov-210). The task
/// selected keeps its lines until the selection moves on (`HeldRead`); the
/// rest leave on the shared spring.
struct MarkAllReadButton: View {
    /// The navigator's filter narrows the strip: only what it lists is read.
    var filtering = false
    let action: () -> Void

    @Environment(\.sectionHeaderHovered) private var headerHovered
    /// The pointer is on the words themselves: they read in primary.
    @State private var hovered = false

    /// What it says it does: everything, or under a filter, what's listed.
    static func title(filtering: Bool) -> String { filtering ? "Mark These as Read" : "Mark All as Read" }

    /// Whether it's drawn: only while the pointer is over its header.
    static func isShown(headerHovered: Bool) -> Bool { headerHovered }

    var body: some View {
        if Self.isShown(headerHovered: headerHovered) {
            // The action asks first, and animates what it reads once the
            // person says yes (`MarkReadConfirmation`).
            Button(action: action) {
                Text(Self.title(filtering: filtering))
                    .font(.system(size: WorkspaceStyle.PaneText.secondary))
                    .foregroundStyle(hovered ? Color.primary : SidebarInk.secondary)
                    .lineLimit(1)
                    .fixedSize()
                    .frame(minHeight: ColumnGrid.rowHeight)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .onHover { hovered = $0 }
            // The room between it and the header's count.
            .padding(.trailing, SidebarGrid.gap)
            // ⇧⌘K is the menu's, which reads the whole board.
            .help(filtering ? Self.title(filtering: true) : "Mark All as Read (⇧⌘K)")
            .identified("board-mark-all-read")
        }
    }
}
