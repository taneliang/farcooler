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
/// A SwiftUI button in the toolbar's leading group, beside the sidebar
/// button (ov-105): the toolbar draws its glass, its hover and its press,
/// as it does for every other item. The menu is still an `NSMenu`, not a
/// popover of buttons (review M3): arrow keys, type to select, Return and
/// VoiceOver come with it. ⌘0 (Workspace ▸ Switch Workspace…) opens it
/// from the keyboard (`openRequest`).
struct WorkspaceSwitcherButton: View {
    let title: String
    let repository: String
    let entries: [SwitcherEntry]
    /// Bumped to open the menu from the keyboard.
    let openRequest: Int
    let perform: (SwitcherCommand) -> Void

    /// Where the menu drops from, and what it sends.
    @State private var coordinator = Coordinator()

    /// What the toolbar measures it by: a change in what it says
    /// (`LeadingToolbar`).
    var measuredIdentity: String { title + "\u{1}" + repository }

    /// What VoiceOver reads: the workspace, then its repository.
    static func accessibilityLabel(title: String, repository: String) -> String {
        repository.isEmpty ? "Workspace: \(title)" : "Workspace: \(title), \(repository)"
    }

    var body: some View {
        Button {
            open()
        } label: {
            HStack(spacing: 5) {
                HStack(spacing: 0) {
                    Text(title)
                    if !repository.isEmpty { Text(" · \(repository)").foregroundStyle(.secondary) }
                }
                .lineLimit(1)
                Image(systemName: "chevron.down")
                    .imageScale(.small)
                    .font(.caption.weight(.semibold))
                    .foregroundStyle(.secondary)
            }
            // Its whole width: a toolbar short of room moves it to the
            // overflow menu rather than squeezing it to "Main · ov…" (ov-177).
            // What it's measured at is `LeadingToolbar`'s concern.
            .fixedSize()
            .background(MenuAnchor(coordinator: coordinator))
        }
        .help("Switch Workspace (⌘0)")
        .accessibilityLabel(Self.accessibilityLabel(title: title, repository: repository))
        .accessibilityIdentifier("workspace-switcher")
        .onChange(of: openRequest) { _, _ in
            // After the keystroke's own event is done, or the menu would
            // open inside its handling.
            DispatchQueue.main.async { open() }
        }
    }

    private func open() {
        coordinator.entries = entries
        coordinator.perform = perform
        coordinator.open()
    }

    /// The view under the label, for the menu to drop from. Flipped, so the
    /// menu's origin is measured down from its top as a button's is.
    private struct MenuAnchor: NSViewRepresentable {
        let coordinator: Coordinator

        final class Anchor: NSView {
            override var isFlipped: Bool { true }
        }

        func makeNSView(context: Context) -> NSView {
            let view = Anchor()
            coordinator.anchor = view
            return view
        }

        func updateNSView(_ view: NSView, context: Context) { coordinator.anchor = view }
    }

    @MainActor
    final class Coordinator: NSObject {
        var entries: [SwitcherEntry] = []
        var perform: (SwitcherCommand) -> Void = { _ in }
        weak var anchor: NSView?

        func open() {
            guard let anchor else { return }
            let menu = NSMenu()
            menu.autoenablesItems = false
            for entry in entries { add(entry, to: menu) }
            menu.popUp(positioning: nil, at: NSPoint(x: 0, y: anchor.bounds.height + 6), in: anchor)
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
            case .item(let title, let symbol, let badge, let command):
                let item = item(title, command, symbol: symbol)
                if badge > 0 {
                    item.badge = NSMenuItemBadge(count: badge)
                    item.setAccessibilityLabel("\(title), \(badge)")
                }
                menu.addItem(item)
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

/// The words and tint of the title bar's Needs You item (ov-91, ov-105).
enum NeedsYouToolbar {
    /// The tooltip: what waits, in the sidebar's words ("3 things need
    /// you"), plain when nothing does. Never "Needs You (3)" (ov-101).
    static func tooltip(count: Int) -> String {
        switch count {
        case ...0: "Needs You"
        case 1: "1 thing needs you"
        default: "\(count) things need you"
        }
    }
    /// The count beside the tray, as text; nil at zero, "99+" past 99.
    static func countText(count: Int) -> String? { count <= 0 ? nil : count > 99 ? "99+" : "\(count)" }
    /// Whether the count wears the accent: the only color in the toolbar,
    /// and only while something waits. The tray itself never does.
    static func countIsAccent(count: Int) -> Bool { count > 0 }
    static func accessibilityLabel(count: Int) -> String {
        count > 0 ? "Needs You, \(count) waiting" : "Needs You, nothing waiting"
    }
}

/// Needs You in the title bar (ov-86, quieted in ov-105): a monochrome
/// tray with the count beside it as text, in the accent while anything
/// waits; just the tray, in secondary, when nothing does. No system badge.
/// The sidebar's Needs You row, for a window without the sidebar.
struct NeedsYouToolbarButton: View {
    let count: Int
    let selected: Bool
    let onSelect: () -> Void

    var body: some View {
        Button(action: onSelect) {
            HStack(spacing: 4) {
                Image(systemName: "tray")
                    .symbolVariant(selected ? .fill : .none)
                    .foregroundStyle(count > 0 ? .primary : .secondary)
                if let text = NeedsYouToolbar.countText(count: count) {
                    Text(text)
                        .monospacedDigit()
                        .foregroundStyle(NeedsYouToolbar.countIsAccent(count: count) ? Color.accentColor : Color.primary)
                }
            }
        }
        .help(NeedsYouToolbar.tooltip(count: count))
        .accessibilityLabel(NeedsYouToolbar.accessibilityLabel(count: count))
        .accessibilityIdentifier("toolbar-needs-you")
    }
}

/// The runners' trouble in the toolbar (ov-105): a warning and a few words,
/// "carl offline", in secondary, with a menu to reconnect, update and
/// manage. Drawn only while `RunnerStatusItem.label` has something to say.
/// The update asks before it acts, so it opens `DaemonUpdateCard` from
/// here rather than updating from the menu.
struct RunnerStatusMenu: View {
    let label: String
    let symbol: String
    let entries: [RunnerStatusItem.Entry]
    let updates: [DaemonUpdateTarget]
    let perform: (RunnerStatusItem.Entry) -> Void

    @State private var showingUpdate = false

    var body: some View {
        Menu {
            ForEach(Array(entries.enumerated()), id: \.offset) { _, entry in
                switch entry {
                case .separator:
                    Divider()
                case .note(let text):
                    Text(text)
                case .update:
                    Button(entry.title) { showingUpdate = true }
                default:
                    Button(entry.title) { perform(entry) }
                }
            }
        } label: {
            Label(label, systemImage: symbol)
                .labelStyle(.titleAndIcon)
                .foregroundStyle(.secondary)
        }
        .menuIndicator(.hidden)
        .help(label)
        .accessibilityIdentifier("toolbar-runner-status")
        .popover(isPresented: $showingUpdate, arrowEdge: .bottom) {
            DaemonUpdateCard(targets: updates) { showingUpdate = false }
        }
    }
}

/// Whether the toolbar shows the window's title (ov-105).
///
/// Never, today, and for a different reason at each level. Needs You and a
/// workspace itself are named by the switcher beside where the title would
/// sit, so a title there is the switcher again ("Main · overnight ⌄" then
/// "Main"). A task or a worktree opened is the last crumb of the
/// breadcrumb over the main area, which every opened place has, so a
/// title there would be that crumb again. The window keeps its title all
/// the same (`WindowTitle`), for the Window menu and Mission Control.
enum TitleBar {
    static func showsTitle(for selection: ContentView.Selection?) -> Bool {
        switch selection {
        // The switcher names it.
        case nil, .needsYou, .workspace(_, _, nil): return false
        // The breadcrumb's last crumb names it.
        case .workspace(_, _, .some), .looseWorktree: return false
        }
    }
}

/// The toolbar's leading group (ov-105): the sidebar button, then the
/// workspace switcher, after the traffic lights, as Mail and Notes keep
/// theirs. One sidebar button (ov-177), and since the old Fleet sidebar
/// went (ov-178) it's this one, for the navigator: the window has no
/// `NavigationSplitView` for the system's to toggle. Then the flexible
/// space, so this goes outermost of the window's toolbars.
struct LeadingToolbar: ToolbarContent {
    let switcher: WorkspaceSwitcherButton
    let navigator: NavigatorToggle

    var body: some ToolbarContent {
        ToolbarItem(placement: .navigation) { navigator }
        // A new label is a new view, so the toolbar measures it again
        // (ov-177, round 2). The toolbar sizes an item when the window's
        // content is installed, and a label changed after that but before
        // the window is on screen isn't measured again. A window starts at
        // "Workspaces" and, when the store has already loaded (reopened from
        // the Dock, or a second window), picks its workspace in its first
        // `.task`, in that gap. The item stayed "Workspaces" wide, 102 pt,
        // around a 114 pt "Main · overnight", clipped on both sides.
        // A change once it's on screen was always measured
        // (`SwitcherWidthTests`).
        ToolbarItem(placement: .navigation) { switcher.id(switcher.measuredIdentity) }
        // The room the title held, which pushes what follows to the
        // trailing end. `.primaryAction` items stay before it whatever
        // their order, so the window's other items are `.automatic`, and
        // this group is applied outermost so the space comes first.
        ToolbarSpacer(.flexible)
    }
}

/// The title bar's sidebar button (ov-178): the navigator put away or
/// brought back, as ⌘B does. Dimmed where there's no navigator to show:
/// Needs You, a loose worktree with no board, nothing chosen.
struct NavigatorToggle: View {
    let hidden: Bool
    let available: Bool
    let toggle: () -> Void

    var body: some View {
        Button(action: toggle) {
            Label(hidden ? "Show Sidebar" : "Hide Sidebar", systemImage: "sidebar.leading")
        }
        .labelStyle(.iconOnly)
        .disabled(!available)
        .help(hidden ? "Show the sidebar (⌘B)" : "Hide the sidebar (⌘B)")
        .accessibilityIdentifier("navigator-toggle")
    }
}

/// The toolbar's trailing end (ov-105): the runners' trouble while there
/// is any (was the runner bar), then Needs You, apart from the window's
/// own actions before them. Innermost of the window's toolbars, so it
/// comes last.
struct TrailingToolbar: ToolbarContent {
    let troubles: [RunnerStatusItem.Trouble]
    let stale: [String]
    let updates: [DaemonUpdateTarget]
    let needsYou: Int
    let needsYouSelected: Bool
    let onNeedsYou: () -> Void
    let perform: (RunnerStatusItem.Entry) -> Void

    var body: some ToolbarContent {
        ToolbarSpacer(.fixed)
        if let label = RunnerStatusItem.label(troubles: troubles, stale: stale) {
            ToolbarItem {
                RunnerStatusMenu(
                    label: label, symbol: RunnerStatusItem.symbol(troubles: troubles),
                    entries: RunnerStatusItem.entries(troubles: troubles, stale: stale), updates: updates,
                    perform: perform)
            }
        }
        ToolbarItem {
            NeedsYouToolbarButton(count: needsYou, selected: needsYouSelected, onSelect: onNeedsYou)
        }
    }
}
