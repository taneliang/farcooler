import SwiftUI

/// The sidebar's one column system.
///
/// This exists because the previous arrangement had four independent sources of
/// horizontal inset — the search field's own 14, the header's title rail, an 8
/// on the scroll content, and whatever a `Menu` decided to add — and every
/// attempt to line two things up meant guessing which of the four applied to
/// which view. Four rounds of that produced four wrong answers.
///
/// The rule now: **nothing in the sidebar sets its own horizontal padding.**
/// Every row is handed the same band by `SidebarRow`, and indentation inside
/// that band is expressed in columns, not in numbers a caller invents.
enum SidebarGrid {
    /// The band's inset from the window edge. One number, one place.
    static let edge: CGFloat = 14

    /// The disclosure chevron's column, and therefore one indent level.
    static let gutter: CGFloat = 18

    /// The status dot's column in a terminal row.
    static let marker: CGFloat = 8

    /// Space between a marker and the text it belongs to.
    static let gap: CGFloat = 8

    /// How far a row's selection highlight sits inside the band.
    ///
    /// The highlight is narrower than the band so a selected row reads as a
    /// pill inside the column rather than a stripe across the whole sidebar.
    /// Its content then sits at `edge`, like everything else.
    static let highlightInset: CGFloat = 8

    /// A square tap target for an icon control, so every one of them occupies
    /// the same box whatever glyph is inside it.
    static let control: CGFloat = 20
}

/// One row of the sidebar, in the band every other row uses.
///
/// `indent` is in LEVELS, not points: 0 heads a section, 1 is a worktree, 2 is a
/// terminal under it. A caller that wants to nudge something by three points is
/// a caller about to break the column, which is exactly how this got out of
/// alignment before.
struct SidebarRow<Content: View>: View {
    var indent: Int = 0
    @ViewBuilder var content: Content

    var body: some View {
        content
            .padding(.leading, SidebarGrid.edge + CGFloat(indent) * SidebarGrid.gutter)
            .padding(.trailing, SidebarGrid.edge)
            .frame(maxWidth: .infinity, alignment: .leading)
    }
}

/// An icon button that opens a menu, with geometry this file controls.
///
/// Not a bare `Menu`. `.menuStyle(.borderlessButton)` insets its own label by an
/// amount that is invisible, asymmetric, and — on the leading side — clamps how
/// far a negative padding may claw back, so two `Menu`s given identical padding
/// in different containers still landed in different columns. A `Button` has no
/// such chrome, so a fixed square frame here means the glyph is where the frame
/// is, every time.
///
/// The menu itself is a real `NSMenu`, popped at the button: a popover of
/// buttons would have been easier and would not have looked like the rest of
/// macOS.
struct SidebarMenuButton: View {
    let systemImage: String
    let help: String
    let items: [SidebarMenuItem]

    /// A real `NSView` sitting exactly where the button is, so the menu can be
    /// popped in the button's own coordinate system.
    @State private var anchor = MenuAnchor()

    var body: some View {
        Button {
            present()
        } label: {
            Image(systemName: systemImage)
                .font(.system(size: 12, weight: .semibold))
                .frame(width: SidebarGrid.control, height: SidebarGrid.control)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .help(help)
        .background(MenuAnchorView(anchor: anchor))
    }

    private func present() {
        let menu = NSMenu()
        for item in items {
            if item.isSeparator {
                menu.addItem(.separator())
                continue
            }
            let entry = NSMenuItem(title: item.title, action: #selector(Invoker.fire), keyEquivalent: "")
            let invoker = Invoker(item.action)
            entry.target = invoker
            entry.representedObject = invoker
            menu.addItem(entry)
        }
        // Below the control, aligned to its leading edge — expressed in the
        // control's own bounds, which is the only frame of reference here that
        // cannot be misread.
        if let view = anchor.view, view.window != nil {
            menu.popUp(positioning: nil, at: NSPoint(x: 0, y: view.bounds.maxY + 4), in: view)
        } else {
            menu.popUp(positioning: nil, at: NSEvent.mouseLocation, in: nil)
        }
    }

    /// `NSMenuItem` wants a target and a selector, and a SwiftUI closure is
    /// neither — so one tiny object bridges them, kept alive by the item that
    /// points at it.
    private final class Invoker: NSObject {
        private let action: () -> Void
        init(_ action: @escaping () -> Void) { self.action = action }
        @objc func fire() { action() }
    }
}

/// Somewhere to hang the button's `NSView` between drawing it and using it.
///
/// A class, not a captured frame: a `CGRect` from a `GeometryReader` is measured
/// in SwiftUI's coordinates — origin top-left, y growing downward — and
/// `NSView.convert(_:from: nil)` reads whatever it is handed as AppKit window
/// coordinates, origin bottom-left, y growing upward. The two are mirror images,
/// so a project header near the bottom of the sidebar opened its menu near the
/// top of the window, and only a control at the exact vertical middle would have
/// looked right. Popping the menu inside the view means there is no axis left to
/// get backwards.
final class MenuAnchor {
    fileprivate(set) weak var view: NSView?
}

private struct MenuAnchorView: NSViewRepresentable {
    let anchor: MenuAnchor

    func makeNSView(context: Context) -> NSView {
        let view = FlippedAnchorView()
        anchor.view = view
        return view
    }

    func updateNSView(_ nsView: NSView, context: Context) {
        anchor.view = nsView
    }

    /// Flipped so `bounds.maxY` means the bottom edge, as it reads. Invisible to
    /// the mouse, because it sits on top of the button it is measuring.
    private final class FlippedAnchorView: NSView {
        override var isFlipped: Bool { true }
        override func hitTest(_ point: NSPoint) -> NSView? { nil }
    }
}

struct SidebarMenuItem {
    let title: String
    let action: () -> Void
    var isSeparator = false

    /// A computed property, not a stored one: a struct holding a closure is
    /// not `Sendable`, and a `static let` of one is a concurrency error under
    /// Swift 6 even though nothing here is shared.
    static var separator: SidebarMenuItem {
        SidebarMenuItem(title: "", action: {}, isSeparator: true)
    }
}
