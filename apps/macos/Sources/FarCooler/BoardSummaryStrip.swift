import AgentKit
import SwiftUI

/// "Since you were last here": a compact, collapsible strip at the top of the
/// board listing what changed over a period, each line opening its task.
///
/// The board's header hosts it. What goes in the strip is `BoardSummary`'s;
/// this view only chooses the period, draws the lines and reads the records of
/// the tasks that moved.
struct BoardSummaryStrip: View {
    @ObservedObject var store: TaskBoardStore
    let defaults: UserDefaults

    @State private var period: BoardSummary.Period
    @State private var collapsed: Bool
    /// Where the period's menu pops.
    @State private var periodAnchor = MenuAnchor()

    init(store: TaskBoardStore, defaults: UserDefaults = .standard) {
        self.store = store
        self.defaults = defaults
        _period = State(initialValue: Self.readPeriod(store, defaults))
        _collapsed = State(initialValue: defaults.bool(forKey: Self.collapsedKey(store)))
    }

    private static func collapsedKey(_ store: TaskBoardStore) -> String {
        "board.summary.collapsed.\(store.hostKey).\(store.workspace.id)"
    }

    private static func periodKey(_ store: TaskBoardStore) -> String {
        "board.summary.period.\(store.hostKey).\(store.workspace.id)"
    }

    private static func readPeriod(_ store: TaskBoardStore, _ defaults: UserDefaults) -> BoardSummary.Period {
        defaults.string(forKey: periodKey(store)).flatMap(BoardSummary.Period.init(rawValue:))
            ?? .sinceLastVisit
    }

    var body: some View {
        BoardTick { now in
            let since = BoardSummary.start(of: period, lastVisit: store.visitBaseline, now: now)
            let summary = BoardSummary.make(
                rows: store.board.rows, notes: store.summaryNotes, since: since)
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
                accessibilityLabel: "Summary", fillsRow: collapsed,
                label: { open in headerLabel(summary, open: open) },
                accessory: { periodMenu }
            ) {
              VStack(alignment: .leading, spacing: ColumnGrid.rhythm) {
                    if summary.isEmpty {
                        Text(BoardSummary.nothingNew)
                            .font(.system(size: WorkspaceStyle.PaneText.body))
                            .foregroundStyle(.secondary)
                            .lineLimit(1)
                            .truncationMode(.tail)
                            .frame(minHeight: Self.lineHeight)
                            .gridMark("summary.empty", .text)
                            .padding(.leading, ColumnGrid.step)
                            .accessibilityIdentifier("board-summary-empty")
                    } else {
                        let keys = Self.keyColumn(for: summary)
                        group("Finished", summary.finished, keys: keys)
                        group("Needs You or Review", summary.moved, keys: keys)
                        group("New", summary.created, keys: keys)
                        group("Decisions and Findings", summary.notes, keys: keys)
                    }
              }
            }
            // Measured from the board column's edge, as the list below is:
            // the disclosure at column A, everything else at B.
            .padding(.horizontal, ColumnGrid.a)
            .padding(.top, Self.insets(collapsed: collapsed).top)
            .padding(.bottom, Self.insets(collapsed: collapsed).bottom)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(WorkspaceStyle.document)
            .task(id: Self.notesKey(since: since, generation: store.generation, count: store.board.rows.count, collapsed: collapsed)) {
                guard !collapsed else { return }
                await store.readSummaryNotes(since: since)
            }
        }
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("board-summary")
    }

    /// A line of the strip: a group's label, one item, "and 2 more".
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

    /// The font an item's key is set in, which `keyColumn` measures.
    private static let keyFont = NSFont.monospacedSystemFont(
        ofSize: WorkspaceStyle.PaneText.secondary, weight: .regular)

    /// How wide the key column is, so every item's title starts at the same
    /// x, on a grid column: the widest key shown, and a gap, rounded up to
    /// whole steps. "ov-81" and "ov-1" get one column; "bil-1234" two.
    static func keyColumn(for summary: BoardSummary) -> CGFloat {
        let items = [summary.finished, summary.moved, summary.created, summary.notes]
            .flatMap { BoardSummary.capped($0).shown }
        let widest = items.map {
            ($0.key as NSString).size(withAttributes: [.font: keyFont]).width
        }.max() ?? 0
        let steps = max(1, ((widest + SidebarGrid.cellGap) / ColumnGrid.step).rounded(.up))
        return steps * ColumnGrid.step
    }

    /// The collapsed strip's one line: "2 new since your last visit". Counts
    /// the tasks that moved: the decisions and findings are read only while
    /// the strip is open, so counting them here would change the line on
    /// opening it.
    static func collapsedLine(count: Int, period: BoardSummary.Period) -> String {
        let span: String = {
            switch period {
            case .sinceLastVisit: return "since your last visit"
            case .lastHour: return "in the last hour"
            case .today: return "today"
            }
        }()
        return count == 0 ? "Nothing new \(span)" : "\(count) new \(span)"
    }

    /// What a notes read is keyed on. `collapsed` is in it so a strip expanded
    /// after launch reads its decisions and findings then, not at the next minute.
    struct NotesKey: Hashable {
        var since: Double
        var generation: Int
        var count: Int
        var collapsed: Bool
    }

    static func notesKey(since: Date, generation: Int, count: Int, collapsed: Bool) -> NotesKey {
        NotesKey(
            since: (since.timeIntervalSince1970 / 60).rounded(.down), generation: generation,
            count: count, collapsed: collapsed)
    }

    /// The disclosure at column A, and at B the period, which is the
    /// heading: it was a title ("Since you were last here") wrapping to two
    /// lines beside a picker saying "Since Last Visit" (ov-81 P5), so the
    /// same idea cost a line and was said twice. Open, the period is a menu;
    /// closed, the strip is one quiet line saying how much is new (ov-83).
    @ViewBuilder
    private func headerLabel(_ summary: BoardSummary, open: Bool) -> some View {
        if !open {
            Text(
                Self.collapsedLine(
                    count: summary.finished.count + summary.moved.count + summary.created.count, period: period)
            )
            .font(.system(size: WorkspaceStyle.PaneText.body))
            .foregroundStyle(.secondary)
            .lineLimit(1)
            .truncationMode(.tail)
            .gridMark("summary", .text)
        }
    }

    /// Open, the period is the heading, as a menu.
    @ViewBuilder
    private var periodMenu: some View {
        if !collapsed {
            // A real menu, popped at its words, rather than a `Picker`:
            // a pop-up button's bezel would put its words past column B
            // by an inset nobody chose (see `SidebarMenuButton`).
            Button {
                SidebarMenuItem.popUp(
                    BoardSummary.Period.allCases.map { choice in
                        SidebarMenuItem(
                            title: choice.title, action: { choose(choice) },
                            isChecked: choice == period)
                    },
                    under: periodAnchor)
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
            .help("Choose the period")
            .accessibilityLabel("Period")
            .accessibilityValue(period.title)
            .accessibilityIdentifier("board-summary-period")
        }
    }

    private func choose(_ choice: BoardSummary.Period) {
        period = choice
        defaults.set(choice.rawValue, forKey: Self.periodKey(store))
    }

    @ViewBuilder private func group(
        _ title: String, _ items: [BoardSummary.Item], keys: CGFloat
    ) -> some View {
        if !items.isEmpty {
            // At column B: the label, each item's key, "and 2 more"; each
            // item's title in the column after the widest key.
            VStack(alignment: .leading, spacing: 0) {
                Text("\(title) (\(items.count))")
                    .font(.system(size: WorkspaceStyle.PaneText.secondary, weight: .medium))
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                    .truncationMode(.tail)
                    .frame(minHeight: Self.lineHeight)
                    .gridMark("summary.group", .text)
                ForEach(BoardSummary.capped(items).shown) { item in
                    Button {
                        if let row = store.board.rows.first(where: { $0.id == item.taskID }) {
                            store.choose(row)
                        }
                    } label: {
                        HStack(spacing: 0) {
                            Text(item.key)
                                .font(.system(size: WorkspaceStyle.PaneText.secondary, design: .monospaced))
                                .foregroundStyle(.secondary)
                                .lineLimit(1)
                                .gridMark("summary.key", .text)
                                .frame(width: keys, alignment: .leading)
                            Text(item.title).lineLimit(1)
                                .gridMark("summary.title", .text)
                            if let detail = item.detail {
                                Text(detail)
                                    .foregroundStyle(.secondary)
                                    .lineLimit(1)
                                    .padding(.leading, SidebarGrid.cellGap)
                            }
                            Spacer(minLength: 0)
                        }
                        .font(.system(size: WorkspaceStyle.PaneText.body))
                        .frame(minHeight: Self.lineHeight)
                        .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                    .accessibilityIdentifier("board-summary-item-\(item.id)")
                }
                if BoardSummary.capped(items).more > 0 {
                    Text("and \(BoardSummary.capped(items).more) more")
                        .font(.system(size: WorkspaceStyle.PaneText.secondary))
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                        .frame(minHeight: Self.lineHeight)
                }
            }
            .padding(.leading, ColumnGrid.step)
        }
    }
}
