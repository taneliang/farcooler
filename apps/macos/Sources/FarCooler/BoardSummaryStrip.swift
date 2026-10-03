import AgentKit
import SwiftUI

/// Unread (ov-104): a compact, collapsible strip at the top of the board
/// listing what's new to this person, each item opening its task. An item
/// stays until its ticket is opened, and opening it clears that ticket's
/// items (`BoardReads`). Mark All as Read is its header's button
/// (`MarkAllReadButton`), in its context menu and in the Board menu (⇧⌘K).
///
/// The board's header hosts it. What goes in the strip is `BoardSummary`'s;
/// this view only draws the lines and reads the records of the tasks that
/// moved.
struct BoardSummaryStrip: View {
    @ObservedObject var store: TaskBoardStore
    let defaults: UserDefaults
    /// The navigator's filter (⌘F), which narrows this too.
    var filter = ""
    /// The line the selection was chosen at, drawn selected in place
    /// (ov-177), and whether the navigator has the keyboard.
    var selectedLine: String?
    var keyed = false
    /// A line chosen: the navigator opens its task, and remembers the line.
    /// Nil opens the task here.
    var onChooseLine: ((String) -> Void)?

    /// Closed: the navigator's, when it keeps it to walk the lines with ↑
    /// and ↓; else this strip's own.
    private var collapsedBinding: Binding<Bool>?
    @State private var ownCollapsed: Bool
    private var collapsed: Bool { collapsedBinding?.wrappedValue ?? ownCollapsed }
    /// What was listed at the last draw, by identity: what tells a new
    /// arrival from an item that was already there (`BoardArrivals`). Nil
    /// before the first, so nothing flashes on opening the board.
    @State private var listed: [String]?
    /// The arrivals still washed in the accent.
    @State private var arrived: Set<String> = []
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.boardMotionSlowdown) private var slowdown

    init(
        store: TaskBoardStore, defaults: UserDefaults = .standard, filter: String = "", selectedLine: String? = nil,
        keyed: Bool = false, collapsed: Binding<Bool>? = nil, onChooseLine: ((String) -> Void)? = nil
    ) {
        self.store = store
        self.defaults = defaults
        self.filter = filter
        self.selectedLine = selectedLine
        self.keyed = keyed
        self.collapsedBinding = collapsed
        self.onChooseLine = onChooseLine
        _ownCollapsed = State(initialValue: defaults.bool(forKey: Self.collapsedKey(store)))
    }

    static func collapsedKey(_ store: TaskBoardStore) -> String {
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
                        if let collapsedBinding { collapsedBinding.wrappedValue = !open } else { ownCollapsed = !open }
                        defaults.set(!open, forKey: Self.collapsedKey(store))
                    }),
                count: collapsed ? nil : summary.count,
                accessibilityLabel: "Unread",
                label: { open in headerLabel(summary, open: open) },
                accessory: {
                    // In sight, not only in the menus (ov-177: the owner
                    // didn't find it there).
                    if !collapsed, !summary.isEmpty { MarkAllReadButton { store.markAllRead() } }
                }
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
                        let first = Self.firstGroup(summary)
                        group("Finished", summary.finished, follows: first != 0, now: now)
                        group("Needs You or Review", summary.moved, follows: first != 1, now: now)
                        group("New", summary.created, follows: first != 2, now: now)
                        activity(summary.activity, follows: first != 3, now: now)
                    }
                }
                .animation(BoardMotion.list(reduceMotion: reduceMotion, slowedBy: slowdown), value: Self.identities(summary))
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
            // Read again for a selection, too: the task selected keeps its
            // notes here until the selection moves on (ov-177).
            .task(id: Self.notesKey(reads: store.reads, generation: store.generation, count: store.board.rows.count, collapsed: collapsed, held: store.held?.taskID)) {
                guard !collapsed else { return }
                await store.readSummaryNotes(reads: store.reads)
            }
            // What arrived on the board, not what the filter let back in:
            // clearing the filter washes nothing (ov-177).
            .onChange(of: Self.identities(Self.summary(store: store, reads: store.reads, filter: "")), initial: true) {
                _, ids in arrive(ids)
            }
        }
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("board-summary")
    }

    /// The summary drawn: what's unread on `store`'s board by `reads`,
    /// narrowed by the navigator's filter.
    static func summary(store: TaskBoardStore, reads: BoardReads, filter: String) -> BoardSummary {
        // The task selected by the reads it was selected under (ov-177).
        let summary = BoardSummary.make(
            rows: store.board.rows, notes: store.summaryNotes, reads: reads, held: store.held)
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
            withAnimation(.easeOut(duration: BoardMotion.highlightFade * slowdown)) { arrived.subtract(new) }
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
        var held: String?
    }

    static func notesKey(
        reads: BoardReads, generation: Int, count: Int, collapsed: Bool, held: String? = nil
    ) -> NotesKey {
        let marks = reads.opened.values.map(\.timeIntervalSince1970).reduce(0, +)
        return NotesKey(
            reads: "\(reads.floor.timeIntervalSince1970) \(marks)", generation: generation, count: count,
            collapsed: collapsed, held: held)
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

    /// A line chosen: its task opens, and the line stays where it is while
    /// the task is selected (`TaskBoardStore.hold`).
    private func open(_ line: String, task taskID: String) {
        store.hold(taskID)
        if let onChooseLine { return onChooseLine(line) }
        if let row = store.board.rows.first(where: { $0.id == taskID }) { store.choose(row) }
    }

    /// Every line ↑ and ↓ walk, top to bottom, by the id it's drawn with:
    /// each group's lines as drawn, past five only "and 2 more", then
    /// Activity's.
    static func lines(_ summary: BoardSummary) -> [String] {
        [summary.finished, summary.moved, summary.created].flatMap { BoardSummary.capped($0).shown.map(\.id) }
            + BoardSummary.capped(summary.activity).shown.map(\.id)
    }

    /// The task a line is about: a line's id is its task's, then what
    /// happened ("<task>/done", "<task>/activity").
    static func task(ofLine line: String) -> String {
        String(line.prefix { $0 != "/" })
    }

    /// Which group is drawn first, under the header, 0 to 3 in the order
    /// they're drawn: it's the one group no other group's rows are over.
    static func firstGroup(_ summary: BoardSummary) -> Int? {
        [summary.finished.isEmpty, summary.moved.isEmpty, summary.created.isEmpty, summary.activity.isEmpty]
            .firstIndex(of: false)
    }

    /// A group: its header with its count trailing, then its items as
    /// compact rows, "and 2 more" past five.
    @ViewBuilder private func group(
        _ title: String, _ items: [BoardSummary.Item], follows: Bool, now: Date
    ) -> some View {
        if !items.isEmpty {
            let capped = BoardSummary.capped(items)
            VStack(alignment: .leading, spacing: 0) {
                GroupHeader(title: title, count: items.count, follows: follows)
                    .gridMark("summary.group", .text)
                    .padding(.leading, ColumnGrid.step)
                ForEach(capped.shown) { item in
                    CompactTaskRow(
                        key: item.key, title: item.title, selected: item.id == selectedLine, keyed: keyed,
                        highlighted: arrived.contains(item.id), keyMark: "summary.key", titleMark: "summary.title"
                    ) {
                        Text(Self.when(item, now: now))
                            .font(.system(size: WorkspaceStyle.PaneText.minimum))
                            .foregroundStyle(.secondary)
                            .lineLimit(1)
                    }
                    .contentShape(Rectangle())
                    .onTapGesture { open(item.id, task: item.taskID) }
                    .id(NavigatorItem.unread(item.id))
                    .transition(BoardMotion.rowTransition(reduceMotion: reduceMotion, slowedBy: slowdown))
                    .accessibilityElement(children: .combine)
                    .accessibilityAddTraits(.isButton)
                    .accessibilityAction(.default) { open(item.id, task: item.taskID) }
                    .accessibilityIdentifier("board-summary-item-\(item.id)")
                }
                more(capped.more)
            }
            .transition(BoardMotion.rowTransition(reduceMotion: reduceMotion, slowedBy: slowdown))
        }
    }

    /// Activity: one entry per ticket, its newest note's kind and text on
    /// two lines, when, and how many older ones it has.
    @ViewBuilder private func activity(
        _ entries: [BoardSummary.Activity], follows: Bool, now: Date
    ) -> some View {
        if !entries.isEmpty {
            let capped = BoardSummary.capped(entries)
            VStack(alignment: .leading, spacing: 0) {
                GroupHeader(title: "Activity", count: entries.count, follows: follows)
                    .gridMark("summary.group", .text)
                    .padding(.leading, ColumnGrid.step)
                ForEach(capped.shown) { entry in
                    CompactTaskRow(
                        key: entry.key, title: entry.title, selected: entry.id == selectedLine, keyed: keyed,
                        highlighted: arrived.contains("\(entry.id)/\(entry.noteID)"),
                        keyMark: "summary.key", titleMark: "summary.title"
                    ) {
                        ActivityNoteView(entry: entry, now: now)
                    }
                    .contentShape(Rectangle())
                    .onTapGesture { open(entry.id, task: entry.taskID) }
                    .id(NavigatorItem.unread(entry.id))
                    .transition(BoardMotion.rowTransition(reduceMotion: reduceMotion, slowedBy: slowdown))
                    .accessibilityElement(children: .combine)
                    .accessibilityAddTraits(.isButton)
                    .accessibilityAction(.default) { open(entry.id, task: entry.taskID) }
                    .accessibilityIdentifier("board-summary-activity-\(entry.key)")
                }
                more(capped.more)
            }
            .transition(BoardMotion.rowTransition(reduceMotion: reduceMotion, slowedBy: slowdown))
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

/// Mark All as Read on Unread's header (ov-177): a checkmark in a circle, in
/// secondary, trailing beside the count, drawn while there's anything to
/// read. Also in the header's context menu and Board ▸ Mark All as Read
/// (⇧⌘K). The task selected keeps its lines until the selection moves on
/// (`HeldRead`); the rest leave on the shared spring.
struct MarkAllReadButton: View {
    let action: () -> Void
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.boardMotionSlowdown) private var slowdown

    var body: some View {
        Button {
            withAnimation(BoardMotion.list(reduceMotion: reduceMotion, slowedBy: slowdown), action)
        } label: {
            Image(systemName: "checkmark.circle")
                .font(.system(size: 12))
                .foregroundStyle(.secondary)
                .frame(width: ColumnGrid.step, height: ColumnGrid.rowHeight)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .help("Mark All as Read (⇧⌘K)")
        .accessibilityLabel("Mark All as Read")
        .accessibilityIdentifier("board-mark-all-read")
    }
}
