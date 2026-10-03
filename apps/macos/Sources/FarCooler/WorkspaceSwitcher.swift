import AgentKit
import AppKit
import SwiftUI

/// The workspace switcher in the title bar (ov-86): the workspace you're
/// in and its repository, "Billing · shop ⌄", opening a native menu of
/// every workspace grouped by repository, each with its waiting count as
/// the item's badge and its ⌘-number as its key equivalent, then each
/// repository's own actions, the runners' state, and the places and the
/// things to add (`WorkspaceSwitcherMenu.entries`).
///
/// An `NSMenu`, not a popover of buttons (review M3): arrow keys, type to
/// select, Return and VoiceOver come with it. ⌘0 (Workspace ▸ Switch
/// Workspace…) opens it from the keyboard (`openRequest`).
struct WorkspaceSwitcherButton: NSViewRepresentable {
    let title: String
    let repository: String
    let entries: [SwitcherEntry]
    /// Bumped to open the menu from the keyboard.
    let openRequest: Int
    let perform: (SwitcherCommand) -> Void

    /// The bezel (ov-91): borderless until the pointer is over it, then the
    /// standard recessed highlight, and the pressed one while clicked.
    static func configure(_ button: NSButton) {
        button.isBordered = true
        button.bezelStyle = .recessed
        button.showsBorderOnlyWhileMouseInside = true
    }

    func makeCoordinator() -> Coordinator { Coordinator() }

    func makeNSView(context: Context) -> NSButton {
        let button = NSButton(title: "", target: context.coordinator, action: #selector(Coordinator.open(_:)))
        Self.configure(button)
        button.imagePosition = .imageTrailing
        button.image = NSImage(systemSymbolName: "chevron.down", accessibilityDescription: nil)?
            .withSymbolConfiguration(.init(pointSize: 9, weight: .semibold))
        button.setAccessibilityIdentifier("workspace-switcher")
        button.toolTip = "Switch Workspace (⌘0)"
        context.coordinator.seen = openRequest
        return button
    }

    func updateNSView(_ button: NSButton, context: Context) {
        let coordinator = context.coordinator
        coordinator.entries = entries
        coordinator.perform = perform
        let text = NSMutableAttributedString(
            string: title, attributes: [.font: NSFont.systemFont(ofSize: 13, weight: .semibold)])
        if !repository.isEmpty {
            text.append(
                NSAttributedString(
                    string: " · \(repository)",
                    attributes: [
                        .font: NSFont.systemFont(ofSize: 12), .foregroundColor: NSColor.secondaryLabelColor,
                    ]))
        }
        text.append(NSAttributedString(string: " "))
        button.attributedTitle = text
        button.setAccessibilityLabel(repository.isEmpty ? "Workspace: \(title)" : "Workspace: \(title), \(repository)")
        if openRequest != coordinator.seen {
            coordinator.seen = openRequest
            DispatchQueue.main.async { coordinator.open(button) }
        }
    }

    @MainActor
    final class Coordinator: NSObject {
        var entries: [SwitcherEntry] = []
        var perform: (SwitcherCommand) -> Void = { _ in }
        var seen = 0

        @objc func open(_ sender: NSButton) {
            let menu = NSMenu()
            menu.autoenablesItems = false
            for entry in entries { add(entry, to: menu) }
            menu.popUp(positioning: nil, at: NSPoint(x: 0, y: sender.bounds.height + 4), in: sender)
        }

        @objc func pick(_ item: NSMenuItem) {
            guard let command = (item.representedObject as? Box)?.command else { return }
            perform(command)
        }

        private final class Box: NSObject {
            let command: SwitcherCommand
            init(_ command: SwitcherCommand) { self.command = command }
        }

        private func item(_ title: String, _ command: SwitcherCommand, symbol: String? = nil) -> NSMenuItem {
            let item = NSMenuItem(title: title, action: #selector(pick(_:)), keyEquivalent: "")
            item.target = self
            item.representedObject = Box(command)
            if let symbol { item.image = NSImage(systemSymbolName: symbol, accessibilityDescription: nil) }
            return item
        }

        private func add(_ entry: SwitcherEntry, to menu: NSMenu) {
            switch entry {
            case .header(let title):
                menu.addItem(NSMenuItem.sectionHeader(title: title))
            case .workspace(let name, let waiting, let current, let number, let command):
                let item = item(name, command)
                item.state = current ? .on : .off
                if let number {
                    item.keyEquivalent = "\(number)"
                    item.keyEquivalentModifierMask = .command
                }
                // The attention dot: the count, in the menu's own badge.
                if waiting > 0 {
                    item.badge = NSMenuItemBadge(count: waiting)
                    item.image = Self.dot(.systemOrange)
                    item.setAccessibilityLabel(waiting == 1 ? "\(name), 1 waiting" : "\(name), \(waiting) waiting")
                } else {
                    item.image = Self.dot(.clear)
                }
                menu.addItem(item)
            case .item(let title, let symbol, let command):
                menu.addItem(item(title, command, symbol: symbol))
            case .status(let text, let trouble):
                let item = NSMenuItem(title: text, action: nil, keyEquivalent: "")
                item.isEnabled = false
                item.image = Self.dot(trouble ? .systemRed : .secondaryLabelColor)
                menu.addItem(item)
            case .submenu(let title, let symbol, let entries):
                let item = NSMenuItem(title: title, action: nil, keyEquivalent: "")
                item.image = NSImage(systemSymbolName: symbol, accessibilityDescription: nil)
                let sub = NSMenu(title: title)
                sub.autoenablesItems = false
                for entry in entries { add(entry, to: sub) }
                item.submenu = sub
                menu.addItem(item)
            case .separator:
                menu.addItem(.separator())
            }
        }

        /// A 7 pt dot, drawn in its color rather than as a template, so the
        /// menu keeps it amber.
        static func dot(_ color: NSColor) -> NSImage {
            let image = NSImage(size: NSSize(width: 12, height: 12), flipped: false) { rect in
                color.setFill()
                NSBezierPath(ovalIn: rect.insetBy(dx: 2.5, dy: 2.5)).fill()
                return true
            }
            image.isTemplate = false
            return image
        }
    }
}

/// The words and tint of the title bar's Needs You button (ov-91).
enum NeedsYouToolbar {
    /// The tooltip: "Needs You (N)", plain when nothing waits.
    static func tooltip(count: Int) -> String { count > 0 ? "Needs You (\(count))" : "Needs You" }
    /// Accent while something waits, secondary otherwise. Never amber.
    static func isTinted(count: Int) -> Bool { count > 0 }
    /// The badge's text; nil at zero, "99+" past 99.
    static func badge(count: Int) -> String? { count <= 0 ? nil : count > 99 ? "99+" : "\(count)" }
    static func accessibilityLabel(count: Int) -> String {
        count == 0 ? "Needs You, nothing waiting" : count == 1 ? "Needs You, 1 item" : "Needs You, \(count) items"
    }
}

/// Needs You in the title bar (ov-86, restyled in ov-91): a standard
/// toolbar button, the tray with the count as the system's badge, tinted
/// with the accent while anything waits. The sidebar's Needs You row, for
/// a window without the sidebar.
struct NeedsYouToolbarButton: View {
    let count: Int
    let selected: Bool
    let onSelect: () -> Void

    var body: some View {
        Button(action: onSelect) {
            Label("Needs You", systemImage: "tray")
                .symbolVariant(selected ? .fill : .none)
                .foregroundStyle(NeedsYouToolbar.isTinted(count: count) ? Color.accentColor : Color.secondary)
        }
        .badge(NeedsYouToolbar.badge(count: count).map { Text($0) })
        .help(NeedsYouToolbar.tooltip(count: count))
        .accessibilityLabel(NeedsYouToolbar.accessibilityLabel(count: count))
        .accessibilityIdentifier("toolbar-needs-you")
    }
}
