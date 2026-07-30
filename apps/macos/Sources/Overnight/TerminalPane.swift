import SwiftUI

/// The terminal detail pane: a header of facts, the live surface, a hint bar.
/// The terminal.
///
/// No header bar and no footer hint. Both were chrome explaining what the
/// sidebar already shows and what a terminal obviously is — a title repeating
/// the selected row, a status repeating its dot, and a permanent sentence about
/// how typing works. The content is a terminal; the window title says which
/// one; everything else was in the way of it.
struct TerminalPane: View {
    @ObservedObject private var preferences = Preferences.shared

    let terminal: Terminal
    let workspace: Workspace
    let binary: String?
    let environment: [String: String]
    let onGeometry: (Int, Int) async -> Void
    let onAction: (TerminalAction) -> Void

    private var isLive: Bool {
        let kind = StateKind.parse(terminal.state)
        return kind == .running || kind == .starting
    }

    var body: some View {
        Group {
            if isLive {
                TerminalSurface(
                    terminal: terminal.short,
                    binary: binary,
                    environment: environment,
                    onResize: onGeometry,
                    fontRevision: preferences.revision
                )
                .id(terminal.id)
            } else {
                inactive
            }
        }
        // The window has rounded corners; an NSView filling the pane does not.
        // Without this the terminal ran square past the window's bottom-right
        // curve while the sidebar beside it curved away, and the two met at a
        // corner that belonged to neither.
        .clipShape(
            UnevenRoundedRectangle(
                bottomLeadingRadius: 0,
                bottomTrailingRadius: 10,
                topTrailingRadius: 0))
        // The window's own title bar, which macOS already draws. Free, native,
        // and it costs the content no vertical space.
        .navigationTitle(workspace.windowTitle)
        .navigationSubtitle(workspace.windowSubtitle)
    }

    /// The only state left that needs saying out loud.
    ///
    /// Everything else removes itself: a terminal is its process, and when that
    /// exits there is nothing to look at. `lost` is the exception, because it
    /// is the one case Overnight cannot explain and will not pretend to.
    private var inactive: some View {
        VStack(spacing: 12) {
            Spacer()
            StatusGlyph(status: terminal.status, size: 14)
            Text(terminal.status.label).font(.title3.weight(.medium))
            Text(
                "This terminal was expected to be running, but no live pane proves it. "
                + "Overnight will not guess between an exit it never saw and a session it "
                + "cannot reach."
            )
            .font(.callout)
            .foregroundStyle(.secondary)
            .multilineTextAlignment(.center)
            .frame(maxWidth: 420)

            Button("Dismiss") { onAction(.dismissLost) }
                .padding(.top, 4)
            Spacer()
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(Color(nsColor: .textBackgroundColor))
        .navigationTitle(workspace.windowTitle)
        .navigationSubtitle(workspace.windowSubtitle)
    }
}
