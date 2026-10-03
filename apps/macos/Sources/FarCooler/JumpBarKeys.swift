import Foundation

// The jump bar from the keyboard (ov-192), as values, so `JumpBarKeysTests`
// pins the routing: ⌘L lands on the last segment; ← and → move between
// segments; ↓, Space or Return opens one's menu; ↑ and ↓ move in it; typing
// filters it; Return jumps; ← and → in an open menu go to the neighbor's
// menu, as Xcode's do; Esc clears the filter, then closes the menu, then
// leaves the bar.

/// Where the keyboard is in the jump bar.
struct JumpBarFocus: Equatable {
    /// The segment the keyboard is on, or nil when it's not in the bar.
    var segment: Int?
    /// Whether that segment's menu is open.
    var open = false
    /// What's been typed into the open menu.
    var query = ""
    /// The highlighted item, by id.
    var highlighted: String?

    static let away = JumpBarFocus()

    var isActive: Bool { segment != nil }
}

/// A key the jump bar takes.
enum JumpKey: Equatable {
    case left, right, up, down, space, `return`, escape, delete
    case character(Character)
}

/// What a key asks the window to do.
enum JumpEffect: Equatable {
    case none
    /// Go there; the bar lets the keyboard go.
    case jump(JumpTarget)
    /// The keyboard leaves the bar, back to what's opened.
    case leave
}

enum JumpBarKeys {
    /// ⌘L: the bar takes the keyboard on its last segment, the level you're
    /// at, menu closed. Nil with no segments.
    static func focus(segments: Int) -> JumpBarFocus? {
        segments > 0 ? JumpBarFocus(segment: segments - 1) : nil
    }

    /// A click on segment `index`: its menu, open, on the item you're at.
    static func open(_ index: Int, menus: [JumpMenu]) -> JumpBarFocus {
        guard menus.indices.contains(index) else { return .away }
        return JumpBarFocus(segment: index, open: true, highlighted: entry(menus[index]))
    }

    /// `key` in `state`, over the segments' `menus` (one each). Returns what
    /// to do; `state` is where the keyboard is after it.
    static func handle(_ key: JumpKey, state: inout JumpBarFocus, menus: [JumpMenu]) -> JumpEffect {
        guard let at = state.segment, menus.indices.contains(at) else {
            state = .away
            return .none
        }
        if !state.open {
            switch key {
            case .left: state.segment = max(at - 1, 0)
            case .right: state.segment = min(at + 1, menus.count - 1)
            case .down, .space, .return: state = open(at, menus: menus)
            case .character(let c):
                // Typing on a segment opens its menu, filtered.
                state = open(at, menus: menus)
                type(c, state: &state, menu: menus[at])
            case .escape:
                state = .away
                return .leave
            case .up, .delete: break
            }
            return .none
        }
        let shown = menus[at].filtered(state.query).items
        switch key {
        case .up, .down:
            let index = shown.firstIndex { $0.id == state.highlighted }
            let next: Int? = {
                guard !shown.isEmpty else { return nil }
                guard let index else { return key == .down ? 0 : shown.count - 1 }
                return key == .down ? min(index + 1, shown.count - 1) : max(index - 1, 0)
            }()
            state.highlighted = next.map { shown[$0].id }
        case .left, .right:
            let next = key == .left ? at - 1 : at + 1
            if menus.indices.contains(next) { state = open(next, menus: menus) }
        case .return, .space:
            if key == .space, !state.query.isEmpty {
                type(" ", state: &state, menu: menus[at])
                return .none
            }
            guard let item = shown.first(where: { $0.id == state.highlighted }) else { return .none }
            state = .away
            return .jump(item.target)
        case .character(let c):
            type(c, state: &state, menu: menus[at])
        case .delete:
            guard !state.query.isEmpty else { break }
            state.query.removeLast()
            settle(&state, menu: menus[at])
        case .escape:
            if !state.query.isEmpty {
                state.query = ""
                settle(&state, menu: menus[at])
            } else {
                state.open = false
                state.highlighted = nil
            }
        }
        return .none
    }

    /// The item a menu opens on: where you are, else its first.
    private static func entry(_ menu: JumpMenu) -> String? { (menu.current ?? menu.items.first)?.id }

    private static func type(_ c: Character, state: inout JumpBarFocus, menu: JumpMenu) {
        state.query.append(c)
        settle(&state, menu: menu)
    }

    /// After the filter changed: the highlight goes to the item the query
    /// means (`JumpMenu.best`: a key it names before one it begins), so
    /// Return opens it whatever's listed above; with the filter cleared, it
    /// stays where it was, or goes back to where you are.
    private static func settle(_ state: inout JumpBarFocus, menu: JumpMenu) {
        if state.query.trimmingCharacters(in: .whitespaces).isEmpty {
            if !menu.items.contains(where: { $0.id == state.highlighted }) { state.highlighted = entry(menu) }
        } else {
            state.highlighted = menu.best(state.query)?.id
        }
    }
}
