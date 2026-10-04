import Foundation
import SwiftUI
import Testing

@testable import Far_Cooler

/// The jump bar from the keyboard (ov-192): ⌘L onto the last segment, ← →
/// between segments, ↓ opens a menu on where you are, typing filters, Return
/// jumps, ← → in a menu go to the neighbor's, Esc steps back out.
struct JumpBarKeysTests {
    private typealias Selection = ContentView.Selection

    private static func place(_ name: String) -> Selection { .workspace(host: "", workspace: name, focus: nil) }

    private static func item(_ id: String, current: Bool = false) -> JumpItem {
        JumpItem(id: id, title: id, current: current, target: .go(place(id)))
    }

    /// Workspaces (Billing checked), then tasks (bil-2 checked), then
    /// worktrees.
    private static let menus = [
        JumpMenu([JumpSection(title: "shop", items: [item("Billing", current: true), item("Search")])]),
        JumpMenu([
            JumpSection(title: "In Progress", items: [item("bil-1 pdf"), item("bil-2 tax", current: true)]),
            JumpSection(title: "Done", items: [item("bil-3 rounding")]),
        ]),
        JumpMenu([JumpSection(title: "Worktrees", items: [item("pdf"), item("scratch")])]),
    ]

    private static func press(_ keys: [JumpKey], from state: JumpBarFocus) -> (JumpBarFocus, [JumpEffect]) {
        var state = state
        let effects = keys.map { JumpBarKeys.handle($0, state: &state, menus: menus) }
        return (state, effects)
    }

    @Test("⌘L lands on the last segment, its menu closed; nothing with no segments")
    func focus() {
        #expect(JumpBarKeys.focus(segments: 3) == JumpBarFocus(segment: 2))
        #expect(JumpBarKeys.focus(segments: 0) == nil)
    }

    @Test("← and → move between segments, held at either end")
    func betweenSegments() {
        let (left, _) = Self.press([.left, .left, .left], from: JumpBarFocus(segment: 2))
        #expect(left == JumpBarFocus(segment: 0))
        let (right, _) = Self.press([.right, .right], from: JumpBarFocus(segment: 1))
        #expect(right == JumpBarFocus(segment: 2))
    }

    @Test("↓ opens the menu on where you are; ↑ ↓ move in it; Return jumps and lets go")
    func openAndJump() {
        let (open, _) = Self.press([.down], from: JumpBarFocus(segment: 1))
        #expect(open == JumpBarFocus(segment: 1, open: true, highlighted: "bil-2 tax"))
        let (moved, effects) = Self.press([.down, .down, .up, .return], from: open)
        #expect(effects.last == .jump(.go(Self.place("bil-2 tax"))))
        #expect(moved == .away)
        // A menu with nothing checked opens on its first.
        let (first, _) = Self.press([.space], from: JumpBarFocus(segment: 2))
        #expect(first.highlighted == "pdf")
    }

    @Test("Typing filters: the highlight goes to the first match, Delete widens, Esc clears then closes")
    func typing() {
        let open = JumpBarFocus(segment: 1, open: true, highlighted: "bil-2 tax")
        let (typed, _) = Self.press([.character("r"), .character("o")], from: open)
        #expect(typed.query == "ro" && typed.highlighted == "bil-3 rounding")
        let (wider, _) = Self.press([.delete], from: typed)
        #expect(wider.query == "r" && wider.highlighted == "bil-3 rounding")
        let (cleared, _) = Self.press([.escape], from: typed)
        #expect(cleared.query.isEmpty && cleared.open && cleared.highlighted == "bil-3 rounding")
        let (closed, closing) = Self.press([.escape], from: cleared)
        #expect(closed == JumpBarFocus(segment: 1) && closing == [.none])
        let (left, leaving) = Self.press([.escape], from: closed)
        #expect(left == .away && leaving == [.leave])
    }

    @Test("Typing on a closed segment opens its menu, filtered; Return with no match does nothing")
    func typeToOpen() {
        let (typed, _) = Self.press([.character("s")], from: JumpBarFocus(segment: 2))
        #expect(typed == JumpBarFocus(segment: 2, open: true, query: "s", highlighted: "scratch"))
        let (none, effects) = Self.press([.character("q"), .return], from: typed)
        #expect(none.highlighted == nil && effects == [.none, .none])
    }

    @Test("Space in a filter is typed, not a jump")
    func spaceInFilter() {
        let (typed, effects) = Self.press(
            [.character("b"), .space, .character("t")], from: JumpBarFocus(segment: 1, open: true))
        #expect(typed.query == "b t" && effects == [.none, .none, .none])
        #expect(typed.highlighted == "bil-2 tax")
    }

    @Test("← and → in an open menu go to the neighbor's menu, on where you are there")
    func acrossMenus() {
        let open = JumpBarFocus(segment: 1, open: true, query: "ro", highlighted: "bil-3 rounding")
        let (left, _) = Self.press([.left], from: open)
        #expect(left == JumpBarFocus(segment: 0, open: true, highlighted: "Billing"))
        let (edge, _) = Self.press([.left], from: left)
        #expect(edge == left)
        let (right, _) = Self.press([.right, .right], from: left)
        #expect(right == JumpBarFocus(segment: 2, open: true, highlighted: "pdf"))
    }

    @Test("A click opens a segment's menu on where you are")
    func click() {
        #expect(JumpBarKeys.open(0, menus: Self.menus) == JumpBarFocus(segment: 0, open: true, highlighted: "Billing"))
        #expect(JumpBarKeys.open(5, menus: Self.menus) == .away)
    }

    @Test("Typing a key highlights that task, not the one above it whose key it begins")
    func typedKey() {
        func task(_ key: String, _ title: String) -> JumpItem {
            JumpItem(id: key, title: "\(key) \(title)", key: key, target: .go(Self.place(key)))
        }
        let menus = [JumpMenu([JumpSection(title: "Backlog", items: [
            task("lo-37", "Remove the legacy aliases"), task("lo-3", "Coordinator agreement"), task("lo-13", "Tidy"),
        ])])]
        var state = JumpBarFocus(segment: 0)
        for c in "lo-3" { _ = JumpBarKeys.handle(.character(c), state: &state, menus: menus) }
        #expect(state.highlighted == "lo-3")
        #expect(menus[0].filtered(state.query).items.first?.id == "lo-37")
        #expect(JumpBarKeys.handle(.return, state: &state, menus: menus) == .jump(.go(Self.place("lo-3"))))
    }

    @Test("Keys with ⌘ or ⌃ pass through to the menu bar; the rest are the bar's")
    func keyMapping() {
        #expect(JumpBarKeys.key(.leftArrow, characters: "", modifiers: [.command, .control]) == nil)
        #expect(JumpBarKeys.key("l", characters: "l", modifiers: .command) == nil)
        #expect(JumpBarKeys.key("a", characters: "\u{1}", modifiers: .control) == nil)
        #expect(JumpBarKeys.key(.leftArrow, characters: "", modifiers: []) == .left)
        #expect(JumpBarKeys.key(.downArrow, characters: "", modifiers: .option) == .down)
        #expect(JumpBarKeys.key(.escape, characters: "", modifiers: []) == .escape)
        #expect(JumpBarKeys.key(.delete, characters: "", modifiers: []) == .delete)
        #expect(JumpBarKeys.key("L", characters: "L", modifiers: .shift) == .character("L"))
        #expect(JumpBarKeys.key("-", characters: "-", modifiers: []) == .character("-"))
        #expect(JumpBarKeys.key(.tab, characters: "\t", modifiers: []) == nil)
    }
}
