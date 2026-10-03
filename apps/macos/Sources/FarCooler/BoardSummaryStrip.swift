import AgentKit
import SwiftUI

/// Unread (ov-104): a compact, collapsible strip at the top of the board
/// listing what's new to this person, each item opening its task. An item
/// stays until its ticket is opened, and opening it clears that ticket's
/// items (`BoardReads`). Last Hour and Today are plain time windows instead,
/// which reading doesn't change.
///
/// The board's header hosts it. What goes in the strip is `BoardSummary`'s;
/// this view only chooses the period, draws the lines and reads the records of
/// the tasks that moved.
struct BoardSummaryStrip: View {
    @ObservedObject var store: TaskBoardStore
    let defaults: UserDefaults
    /// The navigator's filter (⌘F), which narrows this too.
    var filter = ""

    @State private var period: BoardSummary.Period
    @State private var collapsed: Bool
    /// Where the period's menu pops.
    @State private var periodAnchor = MenuAnchor()
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
        _period = State(initialValue: Self.readPeriod(store, defaults))
        _collapsed = State(initialValue: defaults.bool(forKey: Self.collapsedKey(store)))
    }

    private static func collapsedKey(_ store: TaskBoardStore) -> String {
        "board.summary.collapsed.\(store.hostKey).\(store.workspace.id)"
    }

    private static func periodKey(_ store: TaskBoardStore) -> String {
        "board.summary.period.\(store.hostKey).\(store.workspace.id)"
    }

    /// The period chosen on this Mac. Since Last Visit's old choice reads as
    /// Unread, which replaced it.
    static func readPeriod(_ store: TaskBoardStore, _ defaults: UserDefaults) -> BoardSummary.Period {
        defaults.string(forKey: periodKey(store)).flatMap(BoardSummary.Period.init(rawValue:)) ?? .unread
    }

    var body: some View {
        BoardTick { now in
            let window = BoardSummary.window(period, reads: store.reads, now: now)
            let summary = Self.summary(store: store, window: window, filter: filter)
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
                accessibilityLabel: period.title, fillsRow: collapsed,
                label: { open in headerLabel(summary, open: open) },
                accessory: { periodMenu(summary) }
            ) {
                VStack(alignment: .leading, spacing: ColumnGrid.rhythm) {
                    if summary.isEmpty {
                        Text(BoardSummary.nothing(in: period))
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
            .background(WorkspaceStyle.document)
            .task(id: Self.notesKey(window: window, generation: store.generation, count: store.board.rows.count, collapsed: collapsed)) {
                guard !collapsed else { return }
                await store.readSummaryNotes(window: window)
            }
            .onChange(of: Self.identities(summary), initial: true) { _, ids in arrive(ids) }
        }
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("board-summary")
    }

    /// The summary drawn: `store`'s board in `window`, narrowed by the
    /// navigator's filter.
    static func summary(store: TaskBoardStore, window: BoardSummary.Window, filter: String) -> BoardSummary {
        let summary = BoardSummary.make(rows: store.board.rows, notes: store.summaryNotes, window: window)
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

    /// The collapsed strip's one line: "3 unread", "2 new today".
    static func collapsedLine(count: Int, period: BoardSummary.Period) -> String {
        BoardSummary.collapsedLine(count: count, period: period)
    }

    /// What a notes read is keyed on. `collapsed` is in it so a strip expanded
    /// after launch reads its notes then, not at the next minute; the window
    /// so a ticket opened, or the period changed, reads again.
    struct NotesKey: Hashable {
        var window: String
        var generation: Int
        var count: Int
        var collapsed: Bool
    }

    static func notesKey(window: BoardSummary.Window, generation: Int, count: Int, collapsed: Bool) -> NotesKey {
        let key: String
        switch window {
        case .unread(let reads):
            key = "unread \(reads.floor.timeIntervalSince1970) \(reads.opened.values.map(\.timeIntervalSince1970).reduce(0, +))"
        case .since(let start):
            key = "since \((start.timeIntervalSince1970 / 60).rounded(.down))"
        }
        return NotesKey(window: key, generation: generation, count: count, collapsed: collapsed)
    }

    /// Closed, the strip is one quiet line saying how much is new (ov-83).
    @ViewBuilder
    private func headerLabel(_ summary: BoardSummary, open: Bool) -> some View {
        if !open {
            Text(Self.collapsedLine(count: summary.count, period: period))
                .font(.system(size: WorkspaceStyle.PaneText.body))
                .foregroundStyle(.secondary)
                .lineLimit(1)
                .truncationMode(.tail)
                .gridMark("summary", .text)
        }
    }

    /// Open, the period is the heading, as a menu, with Mark All as Read
    /// under Unread's.
    @ViewBuilder
    private func periodMenu(_ summary: BoardSummary) -> some View {
        if !collapsed {
            // A real menu, popped at its words, rather than a `Picker`:
            // a pop-up button's bezel would put its words past column B
            // by an inset nobody chose (see `SidebarMenuButton`).
            Button {
                var items = BoardSummary.Period.allCases.map { choice in
                    SidebarMenuItem(
                        title: choice.title, action: { choose(choice) },
                        isChecked: choice == period)
                }
                if period == .unread, !summary.isEmpty {
                    items.append(SidebarMenuItem(title: "Mark All as Read", action: { store.markAllRead() }))
                }
                SidebarMenuItem.popUp(items, under: periodAnchor)
            } label: {
                HStack(spacing: 4) {
                    Text(period.title)
                        .font(.system(size: WorkspaceStyle.PaneText.body, weight: .semibold))
                        .lineLimit(1)
                        .gridMark("summary", .text)
                    Image(systemName: "chevron.down")
                        .font(.system(size: 8, weight: .bold))
                        .foregroundStyle(.secondary)
                }
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .background(MenuAnchorView(anchor: periodAnchor))
            .help("Choose what to list")
            .accessibilityLabel("Period")
            .accessibilityValue(period.title)
            .accessibilityIdentifier("board-summary-period")
        }
    }

    private func choose(_ choice: BoardSummary.Period) {
        period = choice
        defaults.set(choice.rawValue, forKey: Self.periodKey(store))
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
