import AppKit
import SwiftUI

/// Back and Forward in the title bar (ov-214, slice 3), as Xcode keeps
/// them beside its scheme: through where this window has been, the same
/// steps ⌃⌘← and ⌃⌘→ take. Each is dimmed with nowhere to go, as its menu
/// item is (`MainWindowFocus`).
///
/// A click steps; a press and hold, or a right-click, lists the places this
/// window has been, one list on both buttons with the one it's at checked, as
/// Firefox's session history is (ov-248). Choosing one goes there in one move.
///
/// Drawn only where the status area keeps room for its medium form
/// (`TitleStatusRoom.layout`); narrower, the keys and the Workspace menu
/// still go back and forward.
struct BackForwardControl: View {
    let canGoBack: Bool
    let canGoForward: Bool
    /// The window's history, for each button's long press.
    var rows: [PlaceRow] = []
    let back: () -> Void
    let forward: () -> Void
    /// A row of the list chosen.
    var go: (NavigationHistory.Spot) -> Void = { _ in }

    static let backHelp = "Go back (⌃⌘←)"
    static let forwardHelp = "Go forward (⌃⌘→)"

    var body: some View {
        // Two buttons in one item, not a `ControlGroup`: in a toolbar with a
        // center item, a ControlGroup inserted after the window is on screen
        // is split into two items and laid out at the trailing end (integ-8;
        // the lane's probe). One item keeps its place after the switcher.
        HStack(spacing: 0) {
            button("Back", symbol: "chevron.left", action: back)
                .disabled(!canGoBack)
                .help(Self.backHelp)
                .accessibilityIdentifier("toolbar-back")
            button("Forward", symbol: "chevron.right", action: forward)
                .disabled(!canGoForward)
                .help(Self.forwardHelp)
                .accessibilityIdentifier("toolbar-forward")
        }
        .labelStyle(.iconOnly)
        .background(Anchor.Mark())
    }

    /// A button that steps on a click and opens the history on a press and
    /// hold.
    private func button(_ title: String, symbol: String, action: @escaping () -> Void) -> some View {
        Menu {
            if rows.count > 1 { HistoryMenuItems(rows: rows, go: go) }
        } label: {
            Label(title, systemImage: symbol)
        } primaryAction: {
            action()
        }
        .menuIndicator(.hidden)
        .contextMenu {
            if rows.count > 1 { HistoryMenuItems(rows: rows, go: go) }
        }
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

/// Back and Forward from the mouse and the trackpad (ov-214): a mouse's
/// side buttons (4 back, 5 forward) and the swipe between pages, as in
/// Safari and Xcode. Worked out from the event alone, so a test pins which
/// is which; the window takes them only for itself.
enum BackForwardGesture {
    enum Direction: Equatable { case back, forward }

    /// The direction an event asks for, or nil for any other event. AppKit
    /// numbers mouse buttons from 0, so button 4 is 3.
    static func direction(type: NSEvent.EventType, button: Int = 0, deltaX: CGFloat = 0) -> Direction? {
        switch type {
        case .otherMouseDown:
            switch button {
            case 3: return .back
            case 4: return .forward
            default: return nil
            }
        case .swipe:
            // A swipe's deltaX is positive going back, as WebKit reads it.
            if deltaX > 0 { return .back }
            if deltaX < 0 { return .forward }
            return nil
        default:
            return nil
        }
    }

    static func direction(of event: NSEvent) -> Direction? {
        direction(
            type: event.type, button: event.type == .otherMouseDown ? event.buttonNumber : 0,
            deltaX: event.type == .swipe ? event.deltaX : 0)
    }
}
