import AgentKit
import AppKit
import Foundation

// Collapse All and Expand All for the tree (ov-334): the toggle beside its
// filter, ⌥-click on a disclosure, and View ▸ Collapse All, Expand All. The
// rules over the expansion are AgentKit's (`OneTreeExpansion.setAll`,
// `toggle(_:withSiblings:)`); this is what the window and the tree say to
// each other and how the control reads.

/// A fold the window asks of the tree: from a menu item, whose state lives in
/// the window and whose tree lives in its navigator. `serial` makes two asks
/// for the same thing in a row two changes.
struct TreeFoldRequest: Equatable {
    var serial = 0
    var expands = false

    mutating func collapse() {
        serial += 1
        expands = false
    }

    mutating func expand() {
        serial += 1
        expands = true
    }

    /// What a menu command asks of the tree: View ▸ Collapse All, Expand All.
    /// Any other command asks nothing.
    mutating func apply(_ command: AppCommand) {
        switch command {
        case .collapseTree: collapse()
        case .expandTree: expand()
        default: break
        }
    }
}

/// How the one toggle button reads: it offers what a click would do.
enum TreeFold {
    static func title(anyExpanded: Bool) -> String { anyExpanded ? "Collapse All" : "Expand All" }

    /// A symbol for each, from the pair SF Symbols draws for folding a whole
    /// outline.
    static func symbol(anyExpanded: Bool) -> String {
        anyExpanded ? "rectangle.compress.vertical" : "rectangle.expand.vertical"
    }

    static func help(anyExpanded: Bool) -> String {
        anyExpanded ? "Collapse every theme, card and lane in the tree" : "Expand every theme, card and lane in the tree"
    }

    /// Whether `event` is a mouse click with ⌥ held: the click's own
    /// modifiers, not the keyboard's state at the moment. VoiceOver and the
    /// keyboard reach the same toggle with no click, or with a key event, and
    /// never fold siblings (review 6).
    static func togglesSiblings(event: NSEvent?) -> Bool {
        guard let event else { return false }
        switch event.type {
        case .leftMouseDown, .leftMouseUp: return event.modifierFlags.contains(.option)
        default: return false
        }
    }

    /// `expansion` after `node`'s disclosure is used: with its siblings for
    /// an ⌥-click, else by itself.
    static func toggled(
        _ expansion: OneTreeExpansion, _ node: OneTreeNode, siblings: [OneTreeNode], event: NSEvent?
    ) -> OneTreeExpansion {
        var next = expansion
        if togglesSiblings(event: event) {
            next.toggle(node, withSiblings: siblings)
        } else {
            next.toggle(node)
        }
        return next
    }

    /// The default menu keys: ⌥⌘← and ⌥⌘→, which nothing else here holds.
    static let collapseKeys = "⌥⌘←"
    static let expandKeys = "⌥⌘→"
}
