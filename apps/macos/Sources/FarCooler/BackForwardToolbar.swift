import AppKit
import SwiftUI

/// Back and Forward in the title bar (ov-214, slice 3), as Xcode keeps
/// them beside its scheme: through where this window has been, the same
/// steps ⌃⌘← and ⌃⌘→ take. Each is dimmed with nowhere to go, as its menu
/// item is (`MainWindowFocus`).
///
/// Drawn only where the status area keeps room for its medium form
/// (`TitleStatusRoom.layout`); narrower, the keys and the Workspace menu
/// still go back and forward.
struct BackForwardControl: View {
    let canGoBack: Bool
    let canGoForward: Bool
    let back: () -> Void
    let forward: () -> Void

    static let backHelp = "Go back (⌃⌘←)"
    static let forwardHelp = "Go forward (⌃⌘→)"

    var body: some View {
        ControlGroup {
            Button(action: back) { Label("Back", systemImage: "chevron.left") }
                .disabled(!canGoBack)
                .help(Self.backHelp)
                .accessibilityIdentifier("toolbar-back")
            Button(action: forward) { Label("Forward", systemImage: "chevron.right") }
                .disabled(!canGoForward)
                .help(Self.forwardHelp)
                .accessibilityIdentifier("toolbar-forward")
        }
        .labelStyle(.iconOnly)
        .background(Anchor.Mark())
    }

    /// The view behind the control, for `TitleBarHarness` to find.
    enum Anchor {
        final class View: NSView {}
        struct Mark: NSViewRepresentable {
            func makeNSView(context: Context) -> NSView { View() }
            func updateNSView(_ view: NSView, context: Context) {}
        }
    }
}
