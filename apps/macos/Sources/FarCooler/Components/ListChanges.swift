import AgentKit
import SwiftUI

// The board's data-change motion, shared (ov-298). The owner, Oct 4: lanes
// and themes "just appear or change subtly, and it's hard to tell what
// changed". The task list already answered that: a row that arrives washes in
// the accent and fades over `BoardMotion.highlightFade`, and rows come, go
// and move on the shared spring (`BoardMotion`). This is that, once, for any
// list: the Unread strip and the plan's lists use it.
//
// Data, never navigation: what's animated is keyed on what the rows show, so
// a selection moving (ov-293: drawn on the next frame, no transition) runs in
// a transaction this never touches.

/// What a row of a `listChanges` list tells a test (`listChangeProbe`).
enum ListChangeEvent: Equatable {
    /// The row was drawn in a transaction, animated or not.
    case drawn(id: String, animated: Bool)
    /// The row began to wash.
    case washed(String)
}

/// One row of a list, as `listChanges` tells what changed: its identity, and
/// what it shows (a lane's state and reason, a theme's counts). A row whose
/// signature moves washes as a new one does.
struct ListChangeRow: Hashable {
    var id: String
    var signature = ""
}

extension EnvironmentValues {
    /// The rows of the list around this view that just arrived or changed,
    /// by id: washed while they're in it (`changeWashed`).
    @Entry var listChanged: Set<String> = []
    /// A test's ear on a `listChanges` list's rows: each transaction one is
    /// drawn in, and each wash as it starts. Nil in the app.
    @Entry var listChangeProbe: ((ListChangeEvent) -> Void)? = nil
}

extension View {
    /// This list's rows come, go and move on the shared spring (a cross-fade
    /// under Reduce Motion), and a row that arrives or changes washes in the
    /// accent, fading over `BoardMotion.highlightFade`. Nothing washes on the
    /// first draw. Each row marks itself with `changeWashed(_:)`.
    ///
    /// `arrivals`, when given, is what tells what changed in place of
    /// `rows`: the list before a filter narrowed it, so a filter cleared
    /// washes nothing back in (ov-177).
    func listChanges(_ rows: [ListChangeRow], arrivals: [ListChangeRow]? = nil) -> some View {
        modifier(ListChanges(rows: rows, arrivals: arrivals ?? rows))
    }

    /// A row of a `listChanges` list: washed while it has just arrived or
    /// changed, and coming in and going out as the task list's rows do. A
    /// card washes in its own corners, `card`; a row as a selection is drawn.
    func changeWashed(_ id: String, card: Bool = false) -> some View { modifier(ChangeWash(id: id, card: card)) }
}

/// `listChanges`: what was drawn last, and the rows still washed.
private struct ListChanges: ViewModifier {
    let rows: [ListChangeRow]
    let arrivals: [ListChangeRow]
    /// What was drawn last, by id: nil before the first draw, so nothing
    /// washes on opening a list.
    @State private var drawn: [String: String]?
    @State private var changed: Set<String> = []
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.boardMotionSlowdown) private var slowdown

    func body(content: Content) -> some View {
        content
            .environment(\.listChanged, changed)
            .animation(BoardMotion.list(reduceMotion: reduceMotion, slowedBy: slowdown), value: rows)
            .onChange(of: arrivals, initial: true) { _, now in arrive(now) }
    }

    /// Wash what arrived or changed, and fade it out.
    private func arrive(_ rows: [ListChangeRow]) {
        let now = Dictionary(rows.map { ($0.id, $0.signature) }, uniquingKeysWith: { first, _ in first })
        let new = BoardArrivals.changed(old: drawn, now: now)
        drawn = now
        guard !new.isEmpty else { return }
        changed.formUnion(new)
        DispatchQueue.main.async {
            withAnimation(.easeOut(duration: BoardMotion.highlightFade * slowdown)) { changed.subtract(new) }
        }
    }
}

/// `changeWashed`: the wash behind a row, and its transition.
private struct ChangeWash: ViewModifier {
    let id: String
    let card: Bool
    @Environment(\.listChanged) private var changed
    @Environment(\.listChangeProbe) private var heard
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.boardMotionSlowdown) private var slowdown

    func body(content: Content) -> some View {
        let washed = changed.contains(id)
        content
            .background {
                let fill = washed ? Fill.selection(active: true) : Color.clear
                if card {
                    RoundedRectangle.card.fill(fill)
                } else {
                    RoundedRectangle.control.fill(fill).boxOutset()
                }
            }
            .transaction { transaction in heard?(.drawn(id: id, animated: transaction.animation != nil)) }
            .onChange(of: washed) { _, now in if now { heard?(.washed(id)) } }
            .transition(BoardMotion.rowTransition(reduceMotion: reduceMotion, slowedBy: slowdown))
    }
}
