import AgentKit
import SwiftUI

/// Unread (ov-104): a compact, collapsible strip at the top of the board
/// listing what's new to this person, each item opening its task. An item
/// stays until its ticket is opened, and opening it clears that ticket's
/// items (`BoardReads`). Mark All as Read is in its header's context menu and
/// in the Board menu.
///
/// The board's header hosts it. What goes in the strip is `BoardSummary`'s;
/// this view only draws the lines and reads the records of the tasks that
/// moved.
struct BoardSummaryStrip: View {
    @ObservedObject var store: TaskBoardStore
    let defaults: UserDefaults
    /// The navigator's filter (⌘F), which narrows this too.
    var filter = ""

    @State private var collapsed: Bool
    /// What was listed at the last draw, by identity: what tells a new
    /// arrival from an item that was already there (`BoardArrivals`). Nil
    /// before the first, so nothing flashes on opening the board.
    @State private var listed: [String]?
    /// The arrivals still washed in the accent.
    @State private var arrived: Set<String> = []
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    init(store: TaskBoardStore, defaults: UserDefaults = .standard, filter: String = "") {
        self.store = store
        self.defaults = defaults
        self.filter = filter
        _collapsed = State(initialValue: defaults.bool(forKey: Self.collapsedKey(store)))
    }

    private static func collapsedKey(_ store: TaskBoardStore) -> String {
        "board.summary.collapsed.\(store.hostKey).\(store.workspace.id)"
    }

    var body: some View {
        BoardTick { now in
            let summary = Self.summary(store: store, reads: store.reads, filter: filter)
            // Through the board's one collapsible section (ov-92): it opens
            // and closes on the shared spring, as every other one does.
            CollapsibleSection(
                id: "summary",
                isExpanded: Binding(
                    get: { !collapsed },
                    set: { open in
                        collapsed = !open
                        defaults.set(collapsed, forKey: Self.collapsedKey(store))
                    }),
                count: collapsed ? nil : summary.count,
                accessibilityLabel: "Unread",
                label: { open in headerLabel(summary, open: open) },
                accessory: { EmptyView() }
            ) {
                VStack(alignment: .leading, spacing: ColumnGrid.rhythm) {
                    if summary.isEmpty {
                        Text(BoardSummary.nothing)
                            .font(.system(size: WorkspaceStyle.PaneText.body))
                            .foregroundStyle(.secondary)
                            .lineLimit(1)
                            .truncationMode(.tail)
                            .frame(minHeight: Self.lineHeight)
                            .gridMark("summary.empty", .text)
                            .padding(.leading, ColumnGrid.step)
                            .accessibilityIdentifier("board-summary-empty")
                            .transition(.opacity)
                    } else {
                        group("Finished", summary.finished, now: now)
                        group("Needs You or Review", summary.moved, now: now)
                        group("New", summary.created, now: now)
                        activity(summary.activity, now: now)
                    }
                }
                .animation(BoardMotion.list(reduceMotion: reduceMotion), value: Self.identities(summary))
            }
            // Measured from the board column's edge, as the list below is:
            // the disclosure at column A, everything else at B.
            .padding(.horizontal, ColumnGrid.a)
            .padding(.top, Self.insets(collapsed: collapsed).top)
            .padding(.bottom, Self.insets(collapsed: collapsed).bottom)
            .frame(maxWidth: .infinity, alignment: .leading)
            // No slab of its own (ov-104 review): its header and the
            // navigator's spacing set it apart, as variant B drew the list.
            .contentShape(Rectangle())
            .contextMenu {
                Button("Mark All as Read") { store.markAllRead() }
                    .disabled(summary.isEmpty)
            }
            .task(id: Self.notesKey(reads: store.reads, generation: store.generation, count: store.board.rows.count, collapsed: collapsed)) {
                guard !collapsed else { return }
                await store.readSummaryNotes(reads: store.reads)
            }
            .onChange(of: Self.identities(summary), initial: true) { _, ids in arrive(ids) }
        }
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("board-summary")
    }

    /// The summary drawn: what's unread on `store`'s board by `reads`,
    /// narrowed by the navigator's filter.
    static func summary(store: TaskBoardStore, reads: BoardReads, filter: String) -> BoardSummary {
        let summary = BoardSummary.make(rows: store.board.rows, notes: store.summaryNotes, reads: reads)
        guard !BoardFilter.isEmpty(filter) else { return summary }
        let keep = Set(store.board.rows.filter { BoardFilter.matches($0, filter) }.map(\.id))
        return summary.filtered { keep.contains($0) }
    }

    /// Every line's identity, the ticket's and what happened, a note by its
    /// own id: what the list animates by, and what tells an arrival.
    static func identities(_ summary: BoardSummary) -> [String] {
        (summary.finished + summary.moved + summary.created).map(\.id)
            + summary.activity.map { "\($0.id)/\($0.noteID)" }
    }

    /// New arrivals washed in the accent, fading over `highlightFade`.
    private func arrive(_ ids: [String]) {
        let new = BoardArrivals.new(old: listed, now: ids)
        listed = ids
        guard !new.isEmpty else { return }
        arrived.formUnion(new)
        DispatchQueue.main.async {
            withAnimation(.easeOut(duration: BoardMotion.highlightFade)) { arrived.subtract(new) }
        }
    }

    /// A line of the strip: a group's label, "and 2 more".
    static let lineHeight: CGFloat = 2 * ColumnGrid.rhythm

    /// Its padding above and below. Open, the last line gets the air the
    /// header's own 24 pt row gives its title above, so the text has as much
    /// room under it as over it and the strip reads as a finished block, not
    /// a list cut off at the divider (owner, 2 Oct). Closed, the one row is
    /// centered already.
    static func insets(collapsed: Bool) -> (top: CGFloat, bottom: CGFloat) {
        let top = ColumnGrid.rhythm
        return (top, collapsed ? top : top + headerAir)
    }

    /// The air over the header's title inside its row.
    static let headerAir = (ColumnGrid.rowHeight - lineHeight) / 2

    /// The room over its first line of text and under its last, as drawn.
    static func visibleInsets(collapsed: Bool) -> (top: CGFloat, bottom: CGFloat) {
        let insets = insets(collapsed: collapsed)
        return (insets.top + headerAir, insets.bottom + (collapsed ? headerAir : 0))
    }

    /// The collapsed strip's one line: "3 unread".
    static func collapsedLine(count: Int) -> String { BoardSummary.collapsedLine(count: count) }

    /// What a notes read is keyed on. `collapsed` is in it so a strip expanded
    /// after launch reads its notes then, not at the next minute; the read
    /// state so a ticket opened reads again.
    struct NotesKey: Hashable {
        var reads: String
        var generation: Int
        var count: Int
        var collapsed: Bool
    }

    static func notesKey(reads: BoardReads, generation: Int, count: Int, collapsed: Bool) -> NotesKey {
        let marks = reads.opened.values.map(\.timeIntervalSince1970).reduce(0, +)
        return NotesKey(
            reads: "\(reads.floor.timeIntervalSince1970) \(marks)", generation: generation, count: count,
            collapsed: collapsed)
    }

    /// "Unread" open; closed, one quiet line saying how much (ov-83).
    @ViewBuilder
    private func headerLabel(_ summary: BoardSummary, open: Bool) -> some View {
        if open {
            SectionTitle(text: "Unread", style: .group, gridRow: "summary")
        } else {
            Text(Self.collapsedLine(count: summary.count))
                .font(.system(size: WorkspaceStyle.PaneText.body))
                .foregroundStyle(.secondary)
                .lineLimit(1)
                .truncationMode(.tail)
                .gridMark("summary", .text)
        }
    }

    private func open(_ taskID: String) {
        if let row = store.board.rows.first(where: { $0.id == taskID }) { store.choose(row) }
    }

    /// A group: its header with its count trailing, then its items as
    /// compact rows, "and 2 more" past five.
    @ViewBuilder private func group(_ title: String, _ items: [BoardSummary.Item], now: Date) -> some View {
        if !items.isEmpty {
            let capped = BoardSummary.capped(items)
            VStack(alignment: .leading, spacing: 0) {
                GroupHeader(title: title, count: items.count)
                    .gridMark("summary.group", .text)
                    .padding(.leading, ColumnGrid.step)
                ForEach(capped.shown) { item in
                    CompactTaskRow(
                        key: item.key, title: item.title, highlighted: arrived.contains(item.id),
                        keyMark: "summary.key", titleMark: "summary.title"
                    ) {
                        Text(Self.when(item, now: now))
                            .font(.system(size: WorkspaceStyle.PaneText.minimum))
                            .foregroundStyle(.secondary)
                            .lineLimit(1)
                    }
                    .contentShape(Rectangle())
                    .onTapGesture { open(item.taskID) }
                    .transition(BoardMotion.rowTransition(reduceMotion: reduceMotion))
                    .accessibilityElement(children: .combine)
                    .accessibilityAddTraits(.isButton)
                    .accessibilityIdentifier("board-summary-item-\(item.id)")
                }
                more(capped.more)
            }
            .transition(.opacity)
        }
    }

    /// Activity: one entry per ticket, its newest note's kind and text on
    /// two lines, when, and how many older ones it has.
    @ViewBuilder private func activity(_ entries: [BoardSummary.Activity], now: Date) -> some View {
        if !entries.isEmpty {
            let capped = BoardSummary.capped(entries)
            VStack(alignment: .leading, spacing: 0) {
                GroupHeader(title: "Activity", count: entries.count)
                    .gridMark("summary.group", .text)
                    .padding(.leading, ColumnGrid.step)
                ForEach(capped.shown) { entry in
                    CompactTaskRow(
                        key: entry.key, title: entry.title,
                        highlighted: arrived.contains("\(entry.id)/\(entry.noteID)"),
                        keyMark: "summary.key", titleMark: "summary.title"
                    ) {
                        ActivityNoteView(entry: entry, now: now)
                    }
                    .contentShape(Rectangle())
                    .onTapGesture { open(entry.taskID) }
                    .transition(BoardMotion.rowTransition(reduceMotion: reduceMotion))
                    .accessibilityElement(children: .combine)
                    .accessibilityAddTraits(.isButton)
                    .accessibilityIdentifier("board-summary-activity-\(entry.key)")
                }
                more(capped.more)
            }
            .transition(.opacity)
        }
    }

    @ViewBuilder private func more(_ count: Int) -> some View {
        if count > 0 {
            Text("and \(count) more")
                .font(.system(size: WorkspaceStyle.PaneText.secondary))
                .foregroundStyle(.secondary)
                .lineLimit(1)
                .frame(minHeight: Self.lineHeight)
                .padding(.leading, ColumnGrid.step)
        }
    }

    /// An item's second line: "Done 2h ago", "Needs Decision 5m ago",
    /// "Added 3h ago".
    static func when(_ item: BoardSummary.Item, now: Date) -> String {
        let what = item.detail ?? (item.id.hasSuffix("/done") ? "Done" : "Added")
        return "\(what) \(TaskRow.ago(now.timeIntervalSince(item.at)))"
    }
}

/// An Activity entry's note: its kind as a quiet word, then its text, two
/// lines at most; under them when it was written and "+2 more".
struct ActivityNoteView: View {
    let entry: BoardSummary.Activity
    let now: Date

    var body: some View {
        VStack(alignment: .leading, spacing: 1) {
            Text(Self.note(entry))
                .font(.system(size: WorkspaceStyle.PaneText.secondary))
                .lineLimit(2)
                .truncationMode(.tail)
                .fixedSize(horizontal: false, vertical: true)
            Text(Self.foot(entry, now: now))
                .font(.system(size: WorkspaceStyle.PaneText.minimum))
                .foregroundStyle(.tertiary)
                .lineLimit(1)
        }
    }

    /// "Decision  Owner: 'latest 5' is too limiting…", the kind secondary.
    static func note(_ entry: BoardSummary.Activity) -> AttributedString {
        var kind = AttributedString(entry.kind.title)
        kind.foregroundColor = .secondary
        var text = AttributedString("  " + entry.text)
        text.foregroundColor = .primary
        return kind + text
    }

    /// "12m ago · +2 more".
    static func foot(_ entry: BoardSummary.Activity, now: Date) -> String {
        [TaskRow.ago(now.timeIntervalSince(entry.at)), entry.moreLine].compactMap { $0 }.joined(separator: " · ")
    }
}
