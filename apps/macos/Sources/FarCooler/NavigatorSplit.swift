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
// A closed section is only its header. Terminals and Worktrees are as tall as
// their rows, up to a third of the room while Tasks has rows to show; Tasks
// takes what's left. What a drag chose is kept for the window
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

    /// The least a pane is squeezed to while it has more than that to show:
    /// two rows.
    static let minimum: CGFloat = 2 * ColumnGrid.twoLineRowHeight
    /// The least Tasks is squeezed to: four rows.
    static let fillMinimum: CGFloat = 4 * ColumnGrid.twoLineRowHeight
    /// The share of the room a pane takes by itself, while Tasks wants more
    /// than it has, unless a drag chose otherwise.
    static let capShare: CGFloat = 1.0 / 3

    /// Each open pane's height for its rows, given `room`, the height left
    /// for them once the headers and rules are drawn, and the heights a drag
    /// `chosen`. Never taller than its rows. Tasks gets what the others
    /// leave, and the others give up room, down to `minimum`, until it has
    /// `fillMinimum`. Room nobody needs is left under the last pane.
    static func viewports(_ panes: [Pane], room: CGFloat, chosen: [String: CGFloat] = [:]) -> [String: CGFloat] {
        let room = max(room, 0)
        let fill = panes.first { $0.fills && $0.expanded }
        let others = panes.filter { !$0.fills && $0.expanded }
        // Tasks with more rows than its share keeps at least four rows;
        // with fewer, all of them.
        let wantsMore = fill.map { $0.content > room * (1 - capShare) } ?? false
        let fillFloor = fill.map { wantsMore ? min($0.content, fillMinimum) : $0.content } ?? 0
        let cap = wantsMore ? max(minimum, room * capShare) : .infinity
        func floor(_ pane: Pane) -> CGFloat { min(pane.content, minimum) }

        var heights: [String: CGFloat] = [:]
        for pane in others {
            let want = chosen[pane.id] ?? min(pane.content, cap)
            heights[pane.id] = min(max(want, floor(pane)), pane.content)
        }
        // Room for Tasks' floor, taken from the others in proportion to
        // what each has over its own.
        let taken = others.reduce(0) { $0 + heights[$1.id]! }
        if fill != nil, room - taken < fillFloor {
            let give = others.reduce(0) { $0 + heights[$1.id]! - floor($1) }
            let share = give > 0 ? min(1, (fillFloor - (room - taken)) / give) : 0
            for pane in others { heights[pane.id]! -= (heights[pane.id]! - floor(pane)) * share }
        }
        if let fill {
            let left = room - others.reduce(0) { $0 + heights[$1.id]! }
            heights[fill.id] = min(fill.content, max(fillFloor, left))
        }
        // Room left over goes to the panes no drag sized, top down, up to
        // their rows: few tasks, many terminals.
        var spare = room - heights.values.reduce(0, +)
        for pane in others where chosen[pane.id] == nil && spare > 0 {
            let grow = min(spare, pane.content - heights[pane.id]!)
            heights[pane.id]! += grow
            spare -= grow
        }
        // A window too short for every floor: all of them in proportion.
        let total = heights.values.reduce(0, +)
        if total > room, total > 0 {
            for id in heights.keys { heights[id]! *= room / total }
        }
        return heights
    }

    /// The pane a drag of the rule `above` pane `index` resizes, and its new
    /// height, from `heights` as they were when the drag began, the drag
    /// having gone `dy` down. The pane under the rule grows as the rule
    /// goes up; if it's Tasks or closed, the pane over it grows as the
    /// rule goes down. Nil when neither can be sized.
    static func drag(
        rule index: Int, panes: [Pane], heights: [String: CGFloat], by dy: CGFloat
    ) -> (id: String, height: CGFloat)? {
        guard index > 0, index < panes.count else { return nil }
        let below = panes[index], above = panes[index - 1]
        if below.expanded, !below.fills, let height = heights[below.id] {
            return (below.id, max(0, height - dy))
        }
        if above.expanded, !above.fills, let height = heights[above.id] {
            return (above.id, max(0, height + dy))
        }
        return nil
    }

    /// The heights a drag chose, as the window keeps them:
    /// "terminals=120,worktrees=80", whole points, by pane.
    static func encode(_ chosen: [String: CGFloat]) -> String {
        chosen.sorted { $0.key < $1.key }.map { "\($0.key)=\(Int($0.value.rounded()))" }.joined(separator: ",")
    }

    /// `encode`'s heights read back; anything it didn't write, ignored.
    static func decode(_ kept: String) -> [String: CGFloat] {
        var chosen: [String: CGFloat] = [:]
        for pair in kept.split(separator: ",") {
            let parts = pair.split(separator: "=")
            guard parts.count == 2, let value = Double(parts[1]), value >= 0 else { continue }
            chosen[String(parts[0])] = CGFloat(value)
        }
        return chosen
    }

    /// A rule's slot: the line and `NavigatorRhythm.rule` over and under it.
    static let ruleSlot: CGFloat = 2 * NavigatorRhythm.rule + WorkspaceColumns.divider
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
/// a rule between each two that's dragged to resize them (ov-244).
struct NavigatorSplitView: View {
    let panes: [NavigatorSplitPane]
    /// What drags chose, as `NavigatorSplit.encode` writes it: the
    /// window's to keep.
    @Binding var kept: String

    @State private var headers: [String: CGFloat] = [:]
    @State private var contents: [String: CGFloat] = [:]
    /// The heights when the drag under way began, and what it chose so far.
    @State private var dragStart: [String: CGFloat]?

    private var model: [NavigatorSplit.Pane] {
        panes.map { .init(id: $0.id, fills: $0.fills, expanded: $0.expanded, content: contents[$0.id] ?? 0) }
    }

    private func heights(in height: CGFloat) -> [String: CGFloat] {
        let chrome = panes.reduce(0) { $0 + (headers[$1.id] ?? 0) }
            + CGFloat(max(panes.count - 1, 0)) * NavigatorSplit.ruleSlot + NavigatorRhythm.band
        return NavigatorSplit.viewports(model, room: height - chrome, chosen: NavigatorSplit.decode(kept))
    }

    var body: some View {
        GeometryReader { proxy in
            let heights = heights(in: proxy.size.height)
            VStack(alignment: .leading, spacing: 0) {
                ForEach(Array(panes.enumerated()), id: \.element.id) { index, pane in
                    if index > 0 { rule(index, heights: heights) }
                    pane.header
                        .onGeometryChange(for: CGFloat.self, of: { $0.size.height }) { headers[pane.id] = $0 }
                    if pane.expanded {
                        ScrollView {
                            pane.content
                                .onGeometryChange(for: CGFloat.self, of: { $0.size.height }) { contents[pane.id] = $0 }
                        }
                        .scrollBounceBehavior(.basedOnSize)
                        .frame(height: heights[pane.id] ?? 0)
                        .probed("navigator-pane-\(pane.id)")
                    }
                }
                Spacer(minLength: NavigatorRhythm.band)
            }
            .frame(width: proxy.size.width, height: proxy.size.height, alignment: .top)
        }
    }

    /// The rule over pane `index`: dragged, it resizes the panes either side
    /// (`NavigatorSplit.drag`); double-clicked, it lets them size themselves
    /// again.
    private func rule(_ index: Int, heights: [String: CGFloat]) -> some View {
        Divider().probed("navigator-divider")  // style-exempt: the rule between two navigator panes, dragged
            .padding(.vertical, NavigatorRhythm.rule)
            .contentShape(Rectangle())
            .pointerStyle(.rowResize)
            .gesture(
                // In the window's space: the rule moves with the drag, so
                // its own would chase it.
                DragGesture(minimumDistance: 1, coordinateSpace: .global)
                    .onChanged { value in
                        let start = dragStart ?? heights
                        dragStart = start
                        guard let (id, height) = NavigatorSplit.drag(
                            rule: index, panes: model, heights: start, by: value.translation.height)
                        else { return }
                        var chosen = NavigatorSplit.decode(kept)
                        chosen[id] = height
                        kept = NavigatorSplit.encode(chosen)
                    }
                    .onEnded { _ in dragStart = nil })
            .onTapGesture(count: 2) {
                var chosen = NavigatorSplit.decode(kept)
                for pane in panes { chosen[pane.id] = nil }
                kept = NavigatorSplit.encode(chosen)
            }
            .accessibilityHidden(true)
    }
}
