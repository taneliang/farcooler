import AppKit
import SwiftUI

/// What a pull-down menu holds, as values (ov-319).
enum PullDownEntry {
    case item(String, action: () -> Void)
    /// A dimmed line that says something and does nothing.
    case note(String)
    /// A section's heading.
    case header(String)
    case separator
}

/// A toolbar status control whose menu hangs below it (ov-319).
///
/// A SwiftUI `Menu` in a toolbar is drawn as a pop-up button: the menu opens
/// with its selected row over the control, and covers it. A status count has
/// no selection, so it is a pull-down: a button that pops an `NSMenu` at its
/// own bottom edge, as the workspace switcher does. The control stays in
/// view above the menu.
struct PullDownMenu<Content: View>: View {
    let entries: [PullDownEntry]
    @ViewBuilder let label: () -> Content

    @State private var coordinator = PullDownCoordinator()

    var body: some View {
        Button {
            coordinator.open()
        } label: {
            label().background(Anchor(coordinator: coordinator, entries: entries))
        }
        .buttonStyle(.borderless)
        .fixedSize()
    }

    private struct Anchor: NSViewRepresentable {
        let coordinator: PullDownCoordinator
        let entries: [PullDownEntry]
        func makeNSView(context: Context) -> NSView {
            let view = PullDownAnchorView()
            view.coordinator = coordinator
            coordinator.anchor = view
            coordinator.entries = entries
            return view
        }
        func updateNSView(_ view: NSView, context: Context) {
            coordinator.anchor = view
            coordinator.entries = entries
        }
    }
}

/// Flipped, so the menu's origin is measured down from the top.
final class PullDownAnchorView: NSView {
    override var isFlipped: Bool { true }
    /// What opens the menu from here; a test opens it without a click.
    weak var coordinator: PullDownCoordinator?
}

@MainActor
final class PullDownCoordinator: NSObject {
    var entries: [PullDownEntry] = []
    weak var anchor: NSView?
    /// The last menu opened, for a test to read.
    private(set) var lastMenu: NSMenu?

    final class Box: NSObject {
        let action: () -> Void
        init(_ action: @escaping () -> Void) { self.action = action }
    }

    func open() {
        guard let anchor else { return }
        let menu = PullDownCoordinator.menu(for: entries, target: self)
        lastMenu = menu
        menu.popUp(positioning: nil, at: PullDownCoordinator.origin(anchorHeight: anchor.bounds.height), in: anchor)
    }

    @objc func pick(_ item: NSMenuItem) { (item.representedObject as? Box)?.action() }
}

extension PullDownCoordinator {
    /// The gap between the control's bottom edge and the menu's top.
    static var gap: CGFloat { 8 }

    /// Where the menu's top-left goes, in the flipped anchor: its bottom
    /// edge plus the gap, never inside it.
    static func origin(anchorHeight: CGFloat) -> NSPoint { NSPoint(x: 0, y: anchorHeight + gap) }

    /// The menu `entries` draw.
    static func menu(for entries: [PullDownEntry], target: PullDownCoordinator) -> NSMenu {
        let menu = NSMenu()
        menu.autoenablesItems = false
        for entry in entries {
            switch entry {
            case .item(let title, let action):
                let item = NSMenuItem(title: title, action: #selector(PullDownCoordinator.pick(_:)), keyEquivalent: "")
                item.target = target
                item.representedObject = PullDownCoordinator.Box(action)
                menu.addItem(item)
            case .note(let text):
                let item = NSMenuItem(title: text, action: nil, keyEquivalent: "")
                item.isEnabled = false
                menu.addItem(item)
            case .header(let title):
                menu.addItem(NSMenuItem.sectionHeader(title: title))
            case .separator:
                menu.addItem(.separator())
            }
        }
        return menu
    }
}
