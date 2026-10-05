import AgentKit
import SwiftUI

// The navigator's sections, each scrolling on its own (ov-244). The owner,
// 3 October: "the sidebar should have multiple independent scrollable
// sections, not just 1 big one. so that even if there are a lot of tasks, we
// can still see and open the terminals".
//
// Tasks, Terminals and Worktrees are panes, one under the next, as Xcode's
// split navigator areas are. Each pane's header stays put at its top, and its
// rows scroll under it. A rule between two panes is dragged to resize them.
// A closed section is only its header and gives its room back. Each open
// section is as tall as its rows while the room allows, and otherwise takes an
// even share of it (ov-292); the navigator itself never scrolls. What a drag chose is kept for the window
// (`ContentView`'s scene storage), as `NavigatorSplit.encode` writes it.

/// How tall each pane's rows are drawn: worked out here, as values, so
/// `NavigatorSplitTests` can pin them.
enum NavigatorSplit {
    /// One pane, as the split sees it.
    struct Pane: Equatable {
        var id: String
        /// Takes the room the others leave: Tasks.
        var fills = false
        var expanded = true
        /// How tall its rows are, all of them.
        var content: CGFloat
    }

    /// One row of a pane, near enough: a one-line row's slot, its line and
    /// `NavigatorRhythm.air` over and under it. What a pane's least height
    /// is counted in, and what the keyboard and VoiceOver resize by.
    static let row: CGFloat = ColumnGrid.rowHeight
    /// The least a pane is squeezed to while it has more than that to show:
    /// two rows.
    static let minimum: CGFloat = 2 * row
    /// The least Tasks is squeezed to: four rows.
    static let fillMinimum: CGFloat = 4 * row
    /// The share of the room a pane takes by itself, while Tasks wants more
    /// than it has, unless a drag chose otherwise.
    static let capShare: CGFloat = 1.0 / 3
    /// The tallest height the window keeps for a pane: past any screen.
    static let tallest: CGFloat = 10_000

    /// The least `pane` is drawn at, open: `minimum`, or all of its rows
    /// if it has fewer. Tasks' is `fillMinimum`, while it has more rows
    /// than its share of `room`; else all of them.
    static func floor(_ pane: Pane, fill: Bool = false, wantsMore: Bool = true) -> CGFloat {
        guard pane.expanded else { return 0 }
        if fill { return wantsMore ? min(pane.content, fillMinimum) : pane.content }
        return min(pane.content, minimum)
    }

    /// Whether Tasks, open, has more rows than the room the others leave it
    /// by default.
    static func fillWantsMore(_ panes: [Pane], room: CGFloat) -> Bool {
        panes.first { $0.fills && $0.expanded }.map { $0.content > max(room, 0) * (1 - capShare) } ?? false
    }

    /// Each open pane's height for its rows, given `room`, the height left
    /// for them once the headers and rules are drawn, and the heights a drag
    /// `chosen` (ov-292). A pane is as tall as its rows while the room
    /// allows, and scrolls only when it can't: what's left after the drags is
    /// shared out evenly, each pane taking no more than its rows, and what a
    /// short pane doesn't need goes on to the taller ones. A closed pane takes
    /// none, so its room is the others'. Each open pane keeps at least its
    /// `floor` while the room lasts; a window too short for every floor
    /// squeezes them all in proportion, to nothing at the least, so every
    /// header stays on screen and the navigator itself never scrolls.
    static func viewports(_ panes: [Pane], room: CGFloat, chosen: [String: CGFloat] = [:]) -> [String: CGFloat] {
        let room = max(room, 0)
        var heights: [String: CGFloat] = [:]
        var free: [Pane] = []
        for pane in panes where pane.expanded {
            if !pane.fills, let want = chosen[pane.id] {
                heights[pane.id] = min(max(want, floor(pane)), pane.content)
            } else {
                free.append(pane)
                heights[pane.id] = floor(pane, fill: pane.fills)
            }
        }
        // One water level for the panes no drag sized: each is as tall as
        // the level, held between its floor and its rows, and the level is the
        // highest that still fits. A short pane takes its rows and the rest
        // share what it leaves.
        let left = room - heights.filter { id, _ in !free.contains { $0.id == id } }.values.reduce(0, +)
        func drawn(_ pane: Pane, at level: CGFloat) -> CGFloat {
            min(pane.content, max(floor(pane, fill: pane.fills), level))
        }
        // The level is found exactly: the total rises linearly between any two
        // of the floors and row heights, so it is read off the segment where
        // it reaches `left`.
        let marks = Set(free.flatMap { [floor($0, fill: $0.fills), $0.content] }).sorted()
        var level = marks.last ?? 0
        for (from, to) in zip(marks, marks.dropFirst()) {
            let at = free.reduce(0) { $0 + drawn($1, at: from) }
            let end = free.reduce(0) { $0 + drawn($1, at: to) }
            if end > left {
                level = end > at ? from + (to - from) * (left - at) / (end - at) : from
                break
            }
        }
        if free.reduce(0, { $0 + drawn($1, at: marks.first ?? 0) }) > left { level = marks.first ?? 0 }
        for pane in free { heights[pane.id] = drawn(pane, at: level) }
        // Over the room: first drags' heights give up what's over their
        // floors, then everything is squeezed in proportion.
        var total = heights.values.reduce(0, +)
        if total > room {
            let open = panes.filter { $0.expanded }
            func least(_ pane: Pane) -> CGFloat { floor(pane, fill: pane.fills) }
            let over = open.reduce(0) { $0 + max(0, heights[$1.id]! - least($1)) }
            if over > 0 {
                let share = min(1, (total - room) / over)
                for pane in open { heights[pane.id]! -= max(0, heights[pane.id]! - least(pane)) * share }
            }
            total = heights.values.reduce(0, +)
            if total > room, total > 0 {
                let scale = room / total
                for pane in open { heights[pane.id]! *= scale }
            }
        }
        return heights
    }

    /// What a rule resizes: the open, sizable panes either side of it. Two
    /// such panes trade room with each other; one trades with Tasks and the
    /// room nobody uses.
    struct Resize: Equatable {
        /// The pane that grows as the rule goes up, if any.
        var below: String?
        /// The pane that grows as the rule goes down, if any.
        var above: String?
        /// How far the rule can go up, and down, from where it's drawn.
        var up: CGFloat
        var down: CGFloat
        /// The pane VoiceOver names: the one under the rule if it's sizable.
        var name: String? { below ?? above }
        var canMove: Bool { up > 0.5 || down > 0.5 }
    }

    /// What the rule over pane `index` resizes, with `heights` drawn in `room`.
    static func resize(rule index: Int, panes: [Pane], heights: [String: CGFloat], room: CGFloat) -> Resize {
        let none = Resize(below: nil, above: nil, up: 0, down: 0)
        guard index > 0, index < panes.count else { return none }
        let sizable = { (pane: Pane) in pane.expanded && !pane.fills && heights[pane.id] != nil }
        let below = panes[index], above = panes[index - 1]
        func h(_ pane: Pane) -> CGFloat { heights[pane.id] ?? 0 }
        // What Tasks and the empty room can give: Tasks down to its floor.
        let wantsMore = fillWantsMore(panes, room: room)
        let pool = max(0, room - heights.values.reduce(0, +))
            + (panes.first { $0.fills && $0.expanded }.map { max(0, h($0) - floor($0, fill: true, wantsMore: wantsMore)) } ?? 0)
        switch (sizable(above), sizable(below)) {
        case (true, true):
            // Up: the one below grows to its rows, the one above shrinks to its floor.
            let up = min(below.content - h(below), h(above) - floor(above))
            let down = min(above.content - h(above), h(below) - floor(below))
            return Resize(below: below.id, above: above.id, up: max(0, up), down: max(0, down))
        case (false, true):
            return Resize(
                below: below.id, above: nil, up: max(0, min(below.content - h(below), pool)),
                down: max(0, h(below) - floor(below)))
        case (true, false):
            return Resize(
                below: nil, above: above.id, up: max(0, h(above) - floor(above)),
                down: max(0, min(above.content - h(above), pool)))
        case (false, false):
            return none
        }
    }

    /// The heights the window keeps once the rule over pane `index` has gone
    /// `dy` down from where `heights` drew it: held within how far it can go
    /// (`resize`), so what's kept is what's drawn and the rule stays under
    /// the pointer up to its limit. Only the panes it resizes change.
    static func dragged(
        rule index: Int, panes: [Pane], heights: [String: CGFloat], room: CGFloat, by dy: CGFloat,
        chosen: [String: CGFloat]
    ) -> [String: CGFloat] {
        let resize = resize(rule: index, panes: panes, heights: heights, room: room)
        let moved = min(max(dy, -resize.up), resize.down)
        var chosen = chosen
        if let below = resize.below { chosen[below] = (heights[below] ?? 0) - moved }
        if let above = resize.above { chosen[above] = (heights[above] ?? 0) + moved }
        return chosen
    }

    /// The heights a drag chose, as the window keeps them:
    /// "terminals=120,worktrees=80", whole points, by pane.
    static func encode(_ chosen: [String: CGFloat]) -> String {
        chosen.filter { $0.value.isFinite }.sorted { $0.key < $1.key }
            .map { "\($0.key)=\(Int(min(max($0.value, 0), tallest).rounded()))" }.joined(separator: ",")
    }

    /// `encode`'s heights read back: anything it didn't write, ignored, and
    /// a height past `tallest` held to it. It's restored with the window, so
    /// it's read as input, never trusted.
    static func decode(_ kept: String) -> [String: CGFloat] {
        var chosen: [String: CGFloat] = [:]
        for pair in kept.split(separator: ",") {
            let parts = pair.split(separator: "=")
            guard parts.count == 2, let value = Double(parts[1]), value.isFinite, value >= 0 else { continue }
            chosen[String(parts[0])] = CGFloat(min(value, Double(tallest)))
        }
        return chosen
    }

    /// A rule's slot: the line, and nothing else (ov-258). The owner, 4
    /// October: "the scroll content should probably bump up against the
    /// lines". The pane over a rule clips right at it, so the rhythm's
    /// `NavigatorRhythm.rule` over and under a line is not a margin the
    /// panes leave, but an inset in what scrolls: `paneInset`.
    static let ruleSlot: CGFloat = WorkspaceColumns.divider

    /// The room a pane's rows keep from the rule under them: at the foot of
    /// the scrolling content, so at rest the last row sits `rule` from the
    /// line, and scrolled, rows run right up to it. A closed pane has no
    /// scroll view, so its header keeps the room instead.
    static let paneInset: CGFloat = NavigatorRhythm.rule
    /// The room between a rule and the header of the pane under it: the
    /// header is fixed, not scrolled, so it keeps this on its own top.
    static let headerInset: CGFloat = NavigatorRhythm.rule

    /// The room over the header of the pane at `index`: a rule's, but for
    /// the first pane, which has none over it.
    static func headerTop(_ index: Int) -> CGFloat { index > 0 ? headerInset : 0 }

    /// The room under a pane's header, when the pane is closed and another
    /// is under it: the rule's. An open pane keeps it in its content.
    static func headerBottom(expanded: Bool, last: Bool) -> CGFloat { expanded || last ? 0 : paneInset }

    /// The room at the foot of an open pane's content: none in the last,
    /// which has no rule under it.
    static func contentBottom(last: Bool) -> CGFloat { last ? 0 : paneInset }
}

/// One pane of the split, drawn: its header, and its rows.
struct NavigatorSplitPane: Identifiable {
    let id: String
    var fills = false
    var expanded: Bool
    let header: AnyView
    let content: AnyView
}

/// The navigator's panes, one under the next, each scrolling on its own,
/// a rule between each two that's dragged to resize them (ov-244). A window
/// too short for every pane's least height squeezes them all, so every header
/// stays in sight and only the panes scroll (ov-292).
struct NavigatorSplitView: View {
    let panes: [NavigatorSplitPane]
    /// What drags chose, as `NavigatorSplit.encode` writes it: the
    /// window's to keep.
    @Binding var kept: String
    /// A rule took the keyboard, or gave it up: ↑ and ↓ resize while it
    /// has it, rather than step through the rows.
    var onRuleFocus: (Bool) -> Void = { _ in }
    /// The selected row's id, as its content tags it with `.id`: kept in
    /// sight, scrolled by as little as that takes, whenever the selection
    /// moves or the panes' heights do (ov-295). Nothing moves while it shows.
    var reveal: AnyHashable?

    @State private var headers: [String: CGFloat] = [:]
    @State private var contents: [String: CGFloat] = [:]
    /// The heights drawn when the drag under way began.
    @State private var dragStart: [String: CGFloat]?

    private var model: [NavigatorSplit.Pane] {
        panes.map { .init(id: $0.id, fills: $0.fills, expanded: $0.expanded, content: contents[$0.id] ?? 0) }
    }

    /// The room the panes' rows have in `height`: what the headers and the
    /// rules leave.
    private func room(in height: CGFloat) -> CGFloat {
        height - panes.reduce(0) { $0 + (headers[$1.id] ?? 0) }
            - CGFloat(max(panes.count - 1, 0)) * NavigatorSplit.ruleSlot - NavigatorRhythm.band
    }

    var body: some View {
        GeometryReader { proxy in
          ScrollViewReader { scroller in
            // For the rules' drags, VoiceOver and keeping the selected row
            // in sight: the heights as last drawn. Where each pane is drawn
            // is `NavigatorSplitLayout`'s, measured in the same pass it
            // places them (ov-298).
            let room = room(in: proxy.size.height)
            let heights = NavigatorSplit.viewports(model, room: room, chosen: NavigatorSplit.decode(kept))
            // Never taller than the window: the panes squeeze (`viewports`)
            // rather than the navigator scrolling (ov-292).
            NavigatorSplitLayout(panes: panes.map { ($0.id, $0.fills, $0.expanded) }, chosen: NavigatorSplit.decode(kept)) {
                ForEach(Array(panes.enumerated()), id: \.element.id) { index, pane in
                    if index > 0 {
                        rule(index, heights: heights, room: room).layoutValue(key: NavigatorSplitRole.self, value: .rule)
                    }
                    let isLast = index == panes.count - 1
                    // Fixed chrome, so the rule's room over it and, in a
                    // closed pane, under it, are its own (ov-258).
                    pane.header
                        .padding(.top, NavigatorSplit.headerTop(index))  // rhythm-exempt: NavigatorRhythm.rule, or none over the first
                        .padding(.bottom, NavigatorSplit.headerBottom(expanded: pane.expanded, last: isLast))  // rhythm-exempt: NavigatorRhythm.rule in a closed pane over a rule
                        .onGeometryChange(for: CGFloat.self, of: { $0.size.height }) { headers[pane.id] = $0 }
                        .layoutValue(key: NavigatorSplitRole.self, value: .header(index))
                    if pane.expanded {
                        ScrollView {
                            // The room under the last row is in the
                            // content, so the scroll view itself meets
                            // the rule with no gap.
                            pane.content
                                .padding(.bottom, NavigatorSplit.contentBottom(last: isLast))  // rhythm-exempt: NavigatorRhythm.rule, or none in the last
                                .onGeometryChange(for: CGFloat.self, of: { $0.size.height }) {
                                    contents[pane.id] = $0
                                }
                        }
                        .scrollBounceBehavior(.basedOnSize)
                        .probed("navigator-pane-\(pane.id)")
                        .layoutValue(key: NavigatorSplitRole.self, value: .content(index))
                    }
                }
            }
            .frame(width: proxy.size.width, height: proxy.size.height, alignment: .top)  // rhythm-exempt: the window's height
            .clipped()
            .probed("navigator-split")
            // The selected row stays in sight: when it changes (a short
            // spring), and when a pane's height or openness does, a window
            // resize, a drag, a collapse, or the first layout of a launch or
            // restore (no animation). `scrollTo` with no anchor moves by the
            // least it takes, and not at all for a row already in view.
            .onChange(of: reveal) { _, row in
                guard let row else { return }
                withAnimation(WorkspaceMotion.spring) { scroller.scrollTo(row) }
            }
            .onChange(of: heights, initial: true) { _, _ in show(reveal, with: scroller) }
            // Rows arriving or leaving above it move it without a pane's
            // height changing: the Unread lines settling after a launch.
            .onChange(of: contents) { _, _ in show(reveal, with: scroller) }
          }
        }
    }

    /// Scroll `row` into view once the heights that were just set are laid
    /// out: a pane that has just opened or been resized hasn't yet.
    private func show(_ row: AnyHashable?, with scroller: ScrollViewProxy) {
        guard let row else { return }
        DispatchQueue.main.async { scroller.scrollTo(row) }
    }

    /// Keep what the rule over pane `index` does, gone `dy` down from where
    /// `heights` drew it (`NavigatorSplit.dragged`).
    private func move(_ index: Int, heights: [String: CGFloat], room: CGFloat, by dy: CGFloat) {
        kept = NavigatorSplit.encode(
            NavigatorSplit.dragged(
                rule: index, panes: model, heights: heights, room: room, by: dy, chosen: NavigatorSplit.decode(kept)))
    }

    /// The rule over pane `index`: dragged, or stepped a row at a time by
    /// ↑ and ↓ while it has the keyboard and by VoiceOver's adjust, it
    /// resizes the panes either side (`NavigatorSplit.resize`);
    /// double-clicked, it lets them size themselves again.
    private func rule(_ index: Int, heights: [String: CGFloat], room: CGFloat) -> some View {
        let resize = NavigatorSplit.resize(rule: index, panes: model, heights: heights, room: room)
        let title = resize.name.flatMap { id in Self.titles[id] } ?? "Sections"
        return NavigatorSplitRule(
            index: index, canMove: resize.canMove,
            label: "Resize \(title)",
            value: resize.name.flatMap { heights[$0] }.map { "\(Int($0.rounded())) points" } ?? "",
            onDrag: { dy in
                let start = dragStart ?? heights
                dragStart = start
                move(index, heights: start, room: room, by: dy)
            },
            onDragEnd: { dragStart = nil },
            onStep: { rows in
                // Up grows the pane under the rule: a row up is a row less of dy.
                move(index, heights: heights, room: room, by: -CGFloat(rows) * NavigatorSplit.row)
            },
            onFocus: onRuleFocus,
            onReset: {
                var chosen = NavigatorSplit.decode(kept)
                for pane in panes { chosen[pane.id] = nil }
                kept = NavigatorSplit.encode(chosen)
            })
        .probed("navigator-rule-\(index)")
    }

    /// The names the rules read for VoiceOver, by pane.
    static let titles = [
        "themes": "Themes", "pages": "Pages", "tasks": "Tasks", "terminals": "Terminals", "worktrees": "Worktrees",
    ]
}

/// A rule between two panes: a line, the grip over it, and its keyboard
/// and VoiceOver ways to do the same.
struct NavigatorSplitRule: View {
    /// Which rule: the one over pane `index`.
    let index: Int
    /// Whether it can move at all: a rule over a pane whose rows all show,
    /// with nothing either side to trade, doesn't offer to.
    let canMove: Bool
    let label: String
    let value: String
    /// Gone `dy` down since the drag began.
    let onDrag: (CGFloat) -> Void
    let onDragEnd: () -> Void
    /// Up (positive) or down (negative) a number of rows: the pane under
    /// the rule grows going up.
    let onStep: (Int) -> Void
    let onFocus: (Bool) -> Void
    let onReset: () -> Void

    @FocusState private var focused: Bool

    /// VoiceOver's adjust: up a row, or down.
    func adjust(_ direction: AccessibilityAdjustmentDirection) {
        switch direction {
        case .increment: onStep(1)
        case .decrement: onStep(-1)
        @unknown default: break
        }
    }

    /// ↑ or ↓ while it has the keyboard: the same steps.
    func press(_ key: KeyEquivalent) -> KeyPress.Result {
        switch key {
        case .upArrow: adjust(.increment)
        case .downArrow: adjust(.decrement)
        default: return .ignored
        }
        return .handled
    }

    var body: some View {
        Divider().probed("navigator-divider")  // style-exempt: the rule between two navigator panes, dragged
            .background(focused ? Fill.selection(active: true) : Color.clear)
            .contentShape(Rectangle())
            .pointerStyle(canMove ? .rowResize : nil)
            .gesture(
                // In the window's space: the rule moves with the drag, so
                // its own would chase it.
                DragGesture(minimumDistance: 1, coordinateSpace: .global)
                    .onChanged { onDrag($0.translation.height) }
                    .onEnded { _ in onDragEnd() },
                isEnabled: canMove)
            .onTapGesture(count: 2, perform: onReset)
            .focusable(canMove)
            .focused($focused)
            .onChange(of: focused) { _, now in onFocus(now) }
            .focusEffectDisabled()
            .onKeyPress(keys: [.upArrow, .downArrow]) { press($0.key) }
            .accessibilityElement()
            .accessibilityLabel(label)
            .accessibilityValue(value)
            .accessibilityAdjustableAction(adjust)
            .accessibilityHidden(!canMove)
            .modifier(NavigatorRuleProbe(rule: self))
    }
}

/// A rule as a test finds it (`NavigatorSplitTests`): what VoiceOver reads,
/// and its own handlers for a drag, a key and an adjust. An offscreen
/// window builds no accessibility tree and takes no real input, so a test
/// drives these, the very closures the gesture, the key press and the
/// adjustable action call. Reported only under `gridProbing`.
struct NavigatorRuleReport {
    let rule: NavigatorSplitRule
}

struct NavigatorRulesKey: PreferenceKey {
    static let defaultValue: [NavigatorRuleReport] = []
    static func reduce(value: inout [NavigatorRuleReport], nextValue: () -> [NavigatorRuleReport]) {
        value += nextValue()
    }
}

private struct NavigatorRuleProbe: ViewModifier {
    let rule: NavigatorSplitRule
    @Environment(\.gridProbing) private var probing

    func body(content: Content) -> some View {
        if probing {
            content.transformPreference(NavigatorRulesKey.self) { $0.append(NavigatorRuleReport(rule: rule)) }
        } else {
            content
        }
    }
}

/// The list's ↑ and ↓ as the window's key monitor sends them
/// (`TaskBoardView.Heard.arrow`), for a test: nil while a rule has them.
struct NavigatorArrowsReport {
    let arrow: (Int) -> KeyPress.Result?
}

struct NavigatorArrowsKey: PreferenceKey {
    static var defaultValue: [NavigatorArrowsReport] { [] }
    static func reduce(value: inout [NavigatorArrowsReport], nextValue: () -> [NavigatorArrowsReport]) {
        value += nextValue()
    }
}

struct NavigatorArrowsProbe: ViewModifier {
    let arrow: (Int) -> KeyPress.Result?
    @Environment(\.gridProbing) private var probing

    func body(content: Content) -> some View {
        if probing {
            content.transformPreference(NavigatorArrowsKey.self) { $0.append(NavigatorArrowsReport(arrow: arrow)) }
        } else {
            content
        }
    }
}

/// What a subview of `NavigatorSplitLayout` is.
enum NavigatorSplitRole: LayoutValueKey, Equatable {
    case rule
    case header(Int)
    case content(Int)
    static let defaultValue = NavigatorSplitRole.rule
}

/// The navigator's panes, placed one under the next with each open pane as
/// tall as `NavigatorSplit.viewports` says, from its rows' real height
/// measured in the same layout pass (ov-298). Measured a beat later, from a
/// geometry reading, a pane was first drawn at a guess and then corrected:
/// a gap that closed a moment after the navigator appeared, or a section
/// that jumped as a lazy grid's estimate gave way to its rows.
struct NavigatorSplitLayout: Layout {
    /// Each pane's id, whether it fills, and whether it's open, in order.
    let panes: [(id: String, fills: Bool, expanded: Bool)]
    /// The heights drags chose (`NavigatorSplit.decode`).
    let chosen: [String: CGFloat]

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        proposal.replacingUnspecifiedDimensions()
    }

    /// Each pane's height for its rows, in a column `size` big.
    func heights(in size: CGSize, subviews: Subviews) -> [Int: CGFloat] {
        var fixed: CGFloat = NavigatorRhythm.band
        var content: [Int: CGFloat] = [:]
        for subview in subviews {
            switch subview[NavigatorSplitRole.self] {
            case .rule: fixed += NavigatorSplit.ruleSlot
            case .header: fixed += subview.sizeThatFits(.init(width: size.width, height: nil)).height
            case .content(let index):
                // A scroll view's ideal height is its rows'.
                content[index] = subview.sizeThatFits(.init(width: size.width, height: nil)).height
            }
        }
        let model = panes.enumerated().map { index, pane in
            NavigatorSplit.Pane(id: pane.id, fills: pane.fills, expanded: pane.expanded, content: content[index] ?? 0)
        }
        let viewports = NavigatorSplit.viewports(model, room: size.height - fixed, chosen: chosen)
        return Dictionary(uniqueKeysWithValues: panes.enumerated().map { ($0.offset, viewports[$0.element.id] ?? 0) })
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        let heights = heights(in: bounds.size, subviews: subviews)
        var y = bounds.minY
        for subview in subviews {
            let height: CGFloat
            switch subview[NavigatorSplitRole.self] {
            case .rule: height = NavigatorSplit.ruleSlot
            case .header: height = subview.sizeThatFits(.init(width: bounds.width, height: nil)).height
            case .content(let index): height = heights[index] ?? 0
            }
            subview.place(at: CGPoint(x: bounds.minX, y: y), anchor: .topLeading, proposal: .init(width: bounds.width, height: height))
            y += height
        }
    }
}
