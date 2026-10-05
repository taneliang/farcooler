import AgentKit
import AppKit
import SwiftUI

// The title bar's status area (ov-214): the toolbar's empty middle, put to
// work the way Xcode's activity view is. One standard toolbar item in the
// principal (center) place, on no glass (`sharedBackgroundVisibility(.hidden)`), holding the
// orchestrator's state and what it's doing, then how many of this
// workspace's tasks need you, are running and are in review.
//
// Native by construction: the window, its traffic lights, the toolbar, its
// overflow menu and every button and menu here are the system's. What's
// custom is only text and a status mark inside one item.
//
// The toolbar measures an item once, when the window's content is
// installed, and doesn't flex a principal item between a minimum and a
// maximum (ov-177; the design's probe). So the area never asks it to: it has
// four forms, each a fixed width, chosen from the window's width, and its
// identity is the form and nothing else. Text that grows truncates inside
// the form's width, and a new form is a new view, which the toolbar measures
// again (`TitleStatusWidthTests`).

/// The status area's values and rules.
enum TitleStatus {
    /// The area's height: the regular toolbar's control height (ov-214).
    static let height: CGFloat = 36

    /// How far the area's parts sit in from its ends: none, now that no
    /// capsule is drawn around them (ov-291).
    static func capsuleInset(_ form: Form) -> CGFloat { 0 }

    /// What the area says, built from what the navigator and the board
    /// already work out.
    struct Model: Equatable {
        /// The orchestrator's state, or nil where the workspace has no
        /// conversation column to have one in.
        var orchestrator: OrchestratorRow.State?
        /// Its pane's status, for the status mark.
        var status: Status?
        /// What it's doing now (`OrchestratorRow.nowDoing`).
        var nowDoing: String?
        /// This workspace's count of what's waiting on you: the board's
        /// waiting count (`DaemonClient.boardWaiting`).
        var needYou: Int
        /// Its tasks in progress and in review, in board order.
        var running: [TaskRow]
        var inReview: [TaskRow]
        /// Tasks queued to start, with a wait or a blocker (ov-212).
        var queued: Int = 0
        /// Agents whose run or turn failed, or that were lost: shown only
        /// above zero, in the failure color.
        var failed: [ActivityLine] = []
    }

    /// How much the area can afford to say, narrowest first.
    enum Form: Int, CaseIterable, Comparable, CustomStringConvertible {
        /// The status mark, and the need-you count.
        case ring
        /// Then the state's word.
        case short
        /// Then what it's doing, and every count as a glyph and a number.
        case medium
        /// Then the counts in words.
        case wide

        /// Its width, fixed: see the file's comment.
        var width: CGFloat {
            switch self {
            case .ring: 64
            case .short: 184
            case .medium: 380
            case .wide: 680
            }
        }

        static func < (a: Form, b: Form) -> Bool { a.rawValue < b.rawValue }

        var description: String {
            switch self {
            case .ring: "ring"
            case .short: "short"
            case .medium: "medium"
            case .wide: "wide"
            }
        }
    }

    /// Whether the title bar's field opens in the status area's place at
    /// `form`: at medium and wider. Narrower, the area can't hold a field
    /// worth typing in, and it opens in the panel under the bar instead.
    static func fieldInline(_ form: Form) -> Bool { form >= .medium }

    /// The widest form that fits in `available` points; the ring when none
    /// does, so the orchestrator's mark and the count are always offered.
    static func form(available: CGFloat) -> Form {
        Form.allCases.reversed().first { $0.width <= available } ?? .ring
    }

    /// The room between the toolbar's leading and trailing items in a
    /// window `window` wide, less `gap` each side of the area.
    ///
    /// Strict, because a center item too wide doesn't shrink: the toolbar
    /// keeps it whole and moves the trailing items, the tray among them,
    /// into the overflow menu (the lane's probe, at 600 pt).
    static func available(window: CGFloat, leading: CGFloat, trailing: CGFloat, gap: CGFloat = 8) -> CGFloat {
        window - leading - trailing - 2 * gap
    }

    /// What the leading items take: the traffic lights, then the navigator's
    /// button (with Back and Forward's platter, when there's room) and the
    /// switcher as the title, whose label is `title · repository ⌄` at the
    /// toolbar's font (ov-291: no platter around the switcher; the sum is kept
    /// as it was, which only over-counts).
    ///
    /// Measured in the regular toolbar on macOS 27 (the lane's probe): the
    /// traffic lights end at 96 pt, the navigator's button is 39 and sits
    /// against the switcher, which is its label and 33 to 42 more. Slightly
    /// over, never under: the room left over is what the area may take.
    static func leading(switcher title: String, repository: String) -> CGFloat {
        let label = repository.isEmpty ? title : "\(title) · \(repository)"
        return 96 + 39 + textWidth(label) + 43 + 8
    }

    /// How far the orchestrator's menu sits below center so its label's
    /// baseline meets the borderless buttons' beside it (integ-9).
    static let menuBaselineNudge: CGFloat = 1

    /// What the trailing items take: the window's edge, the tray and its
    /// count, the space before them, then Open in Editor and Changes, and
    /// the runner trouble's words while there's any. Measured as `leading`
    /// is, in the regular toolbar: the tray 36 pt and its count, the editor 80,
    /// Changes 36, and 8 between items; Show Files 37, in Changes' capsule
    /// with no gap of its own.
    static func trailing(editor: Bool, changes: Bool, files: Bool = false, trouble: String?, needsYou: Int = 0) -> CGFloat {
        var width: CGFloat = 10 + 36 + 12
        if let count = NeedsYouToolbar.countText(count: needsYou) { width += 4 + textWidth(count) }
        if editor { width += 80 + 8 }
        if changes { width += 36 + 8 }
        // Show Files (ov-189): an icon button in Changes' capsule, so no gap.
        if files { width += 37 }
        if let trouble { width += 34 + textWidth(trouble) + 8 }
        return width
    }

    /// `text`'s width at the toolbar's font, rounded up.
    static func textWidth(_ text: String) -> CGFloat {
        let font = NSFont.systemFont(ofSize: NSFont.systemFontSize)
        return (text as NSString).size(withAttributes: [.font: font]).width.rounded(.up)
    }

    /// The board's tasks in progress and in review, as the board's own
    /// columns hold them: the status area counts what the navigator lists.
    static func counts(_ board: TaskBoardModel) -> (running: [TaskRow], inReview: [TaskRow]) {
        let column = { (status: TaskStatus) in board.columns.first { $0.status == status }?.rows ?? [] }
        return (column(.inProgress), column(.inReview))
    }

    /// The area's model: `source`'s orchestrator and panes, and `board` and
    /// its tasks' `starts` as read now.
    @MainActor
    static func model(_ source: TitleStatusSource, board: TaskBoardModel, starts: [String: TaskStart] = [:]) -> Model {
        let counts = counts(board)
        return Model(
            orchestrator: source.orchestrator, status: source.status, nowDoing: source.nowDoing,
            needYou: source.waiting(board.waitingOnYou), running: counts.running, inReview: counts.inReview,
            queued: TitleActivity.queuedCount(starts), failed: activity(source, board: board, starts: starts).failed)
    }

    /// The activity panel's content for `source`, over `board` and `starts`.
    @MainActor
    static func activity(_ source: TitleStatusSource, board: TaskBoardModel, starts: [String: TaskStart]) -> TitleActivity {
        TitleActivity.make(
            orchestrator: source.orchestrator, seat: source.seat, panes: source.panes, board: board, starts: starts)
    }

    static func failedWords(_ count: Int) -> String? { count > 0 ? "\(number(count)) failed" : nil }
    static func failedLabel(_ count: Int) -> String { count == 1 ? "1 agent failed" : "\(count) agents failed" }
    static func queuedWords(_ count: Int) -> String? { count > 0 ? "\(number(count)) queued" : nil }

    /// A count as the area draws it: "99+" past 99.
    static func number(_ count: Int) -> String { count > 99 ? "99+" : "\(count)" }

    /// The counts in words, for the wide form: nil at zero, since a count of
    /// nothing teaches people to stop reading it.
    static func needYouWords(_ count: Int) -> String? { count > 0 ? "\(number(count)) need you" : nil }
    static func runningWords(_ count: Int) -> String? { count > 0 ? "\(number(count)) running" : nil }
    static func inReviewWords(_ count: Int) -> String? { count > 0 ? "\(number(count)) in review" : nil }

    /// Whether the need-you count wears the attention color: only above
    /// zero (and it's not drawn at zero at all).
    static func needYouIsTinted(_ count: Int) -> Bool { count > 0 }

    /// The orchestrator's part in words: "Working — Reading the diff", or
    /// the word alone with nothing more to say.
    static func orchestratorLine(_ model: Model) -> String? {
        guard let state = model.orchestrator else { return nil }
        let word = OrchestratorRow.word(state)
        guard let doing = model.nowDoing else { return word }
        return "\(word) — \(doing)"
    }

    /// The state as the title bar and the activity panel name it, with whose
    /// it is: "Orchestrator · Working", or "No Orchestrator" with none
    /// (ov-320).
    ///
    /// With what it's doing (ov-329), one line of status, not a second field:
    /// "Orchestrator · Working: Reviewing the gesture fix".
    static func orchestratorWords(_ state: OrchestratorRow.State, doing: String? = nil) -> String {
        guard state != .none else { return OrchestratorRow.word(.none) }
        let words = "Orchestrator · \(OrchestratorRow.word(state))"
        return doing.map { "\(words): \($0)" } ?? words
    }

    /// Whether the orchestrator's symbol stands before "Orchestrator · …":
    /// where those words show and the mark is a status dot, not the symbol
    /// already (ov-320).
    static func showsGlyph(_ state: OrchestratorRow.State, form: Form) -> Bool {
        form >= .medium && OrchestratorMark.marksStatus(state)
    }

    /// The tooltip over the orchestrator's part: what it shows (ov-320).
    static let orchestratorHelp = "The orchestrator’s state and its current session"

    /// What VoiceOver reads for the orchestrator's part, whatever the form:
    /// "Orchestrator, Working, Reading the diff".
    static func orchestratorLabel(_ model: Model) -> String? {
        guard let state = model.orchestrator else { return nil }
        guard state != .none else { return "No Orchestrator" }
        return ["Orchestrator", OrchestratorRow.word(state), model.nowDoing].compactMap { $0 }.joined(separator: ", ")
    }

    /// The counts as VoiceOver and the tooltips say them: "3 need you".
    static func needYouLabel(_ count: Int) -> String { count == 0 ? "Nothing needs you" : "\(count) need you" }
    static func runningLabel(_ count: Int) -> String { count == 1 ? "1 task running" : "\(count) tasks running" }
    static func inReviewLabel(_ count: Int) -> String { count == 1 ? "1 task in review" : "\(count) tasks in review" }
}

/// Where the status area's model comes from: the orchestrator as the
/// navigator's row works it out, and the workspace's board, observed by the
/// area itself so its counts move when the board does, not only when the
/// window happens to redraw.
struct TitleStatusSource {
    /// The orchestrator's state, or nil where the workspace has no
    /// conversation column (`TitleStatus.Model.orchestrator`).
    var orchestrator: OrchestratorRow.State?
    var status: Status?
    var nowDoing: String?
    /// The workspace's board; nil before there is one to read.
    var board: TaskBoardStore?
    /// The waiting count shown, from the board's Needs Decision count
    /// (`DaemonClient.boardWaiting`, which prefers the runner's list).
    var waiting: (Int) -> Int = { $0 }
    /// The orchestrator's own terminal, for what it last said.
    var seat: Terminal? = nil
    /// The workspace's other panes, for the agents at work and failed.
    var panes: [BoardPane] = []
}

/// What the status area's parts do. `ContentView` routes each to what the
/// window already does for it.
struct TitleStatusActions {
    /// Show the orchestrator, closing whatever is open.
    var goToOrchestrator: () -> Void = {}
    /// The orchestrator's menu: what the conversation column's header held.
    var orchestratorMenu: OrchestratorMenu?
    /// Open the next thing in this workspace that needs you.
    var nextNeedingYou: () -> Void = {}
    /// Open a task from the running or in-review menu.
    var openTask: (TaskRow) -> Void = { _ in }
    /// Open a row of the activity panel or the failed menu: its pane, or
    /// its task.
    var openLine: (ActivityLine) -> Void = { _ in }
    /// Today's spend, read when the activity panel opens.
    var readSpend: () async -> ActivitySpend = { .nothing }
    /// The title bar's field (slice 4), where the window has one.
    var console: TitleConsoleModel?
    var consoleActions = TitleConsoleActions()
}

/// The status area drawn: `form` decides what's said, `model` what it says.
struct TitleStatusView: View {
    let model: TitleStatus.Model
    let form: TitleStatus.Form
    let actions: TitleStatusActions
    /// What the activity panel shows (slice 2).
    var activity = TitleActivity(orchestrator: nil, working: [], queued: [], failed: [])

    @Environment(\.colorScheme) private var scheme
    @State private var showingActivity = false

    /// Whether the field is open in the status area's place (slice 4): at
    /// the medium form and wider; narrower, it opens in the panel under the
    /// bar instead, so the item never changes width.
    private var fieldOpen: Bool { TitleStatus.fieldInline(form) && actions.console?.console.isOpen == true }

    var body: some View {
        HStack(spacing: form >= .medium ? 14 : 8) {
            if fieldOpen, let console = actions.console {
                TitleConsoleField(model: console, actions: actions.consoleActions)
            } else {
                if model.orchestrator != nil { orchestrator }
                if TitleStatus.showsActivityButton(model, form: form) { activityButton }
                Spacer(minLength: 0)
            }
            needYou
            if form >= .short { failed }
            if form >= .medium, !fieldOpen {
                taskMenu(
                    model.running, symbol: "circle.dotted",
                    words: [TitleStatus.runningWords(model.running.count), TitleStatus.queuedWords(model.queued)]
                        .compactMap { $0 }.joined(separator: " · "),
                    label: [TitleStatus.runningLabel(model.running.count), TitleStatus.queuedWords(model.queued)]
                        .compactMap { $0 }.joined(separator: ", "),
                    id: "title-status-running", extra: activity.queued)
                taskMenu(
                    model.inReview, symbol: "eye", words: TitleStatus.inReviewWords(model.inReview.count),
                    label: TitleStatus.inReviewLabel(model.inReview.count), id: "title-status-in-review")
            }
        }
        .font(.system(size: NSFont.systemFontSize))
        .padding(.horizontal, TitleStatus.capsuleInset(form))
        // The regular bar's 36 pt control height, to hit; the field inside
        // is the 28 pt of Xcode's activity view.
        .frame(width: form.width, height: TitleStatus.height)
        .background(TitleStatusAnchor.Mark())
        .popover(isPresented: $showingActivity, arrowEdge: .bottom) {
            TitleActivityPopover(activity: activity, actions: actions) { showingActivity = false }
        }
        .accessibilityElement(children: .contain)
        .accessibilityLabel("Status")
    }

    /// The orchestrator: a click shows it, its ⌄ holds its menu, with Show
    /// Activity first, so the panel is reachable at every width.
    @ViewBuilder
    private var orchestrator: some View {
        let label = HStack(spacing: 6) {
            OrchestratorMark(state: model.orchestrator ?? .none, status: model.status)
            if let state = model.orchestrator, TitleStatus.showsGlyph(state, form: form) {
                // The sidebar's orchestrator symbol before its name, where the
                // mark is a status dot rather than that symbol itself (ov-320).
                OrchestratorMark.glyph.foregroundStyle(.secondary)
            }
            if form >= .short, let state = model.orchestrator {
                // The activity is part of the words, before the caret, and the
                // tail of it gives way, cut to fit (`fittedWords`, ov-329).
                Text(TitleStatus.fittedWords(model, form: form))
                    .foregroundStyle(.primary)
                    .lineLimit(1)
                    .fixedSize()
            }
        }
        Menu {
            Button("Show Activity") { showActivity() }
            if let menu = actions.orchestratorMenu {
                Divider()  // style-exempt: menu
                menu
            }
        } label: {
            label
        } primaryAction: {
            actions.goToOrchestrator()
        }
        .menuStyle(.borderlessButton)
        .fixedSize()
        // A borderless menu draws its label 1 pt higher than a borderless
        // button draws its title in the regular bar (integ-9, measured at
        // 2x: baseline row 59 against the switcher's and the activity's 61,
        // at every width). Padded above by twice that and still centered,
        // its words come down to the same baseline.
        .padding(.top, 2 * TitleStatus.menuBaselineNudge)
        .help(TitleStatus.orchestratorHelp)
        .accessibilityLabel(TitleStatus.orchestratorLabel(model) ?? "Orchestrator")
        .accessibilityIdentifier("title-status-orchestrator")
    }

    /// The activity panel: under the field, empty, where there is one
    /// (slice 4), else in a popover (slice 2).
    private func showActivity() {
        if let console = actions.console {
            console.console.open(recents: false)
        } else {
            showingActivity = true
        }
    }

    /// What it's doing, or "Activity" with nothing to say: a click opens
    /// the activity panel, which is the field's, empty. With no workspace,
    /// "Go to Anything", which opens it to find.
    private var activityButton: some View {
        Button {
            if let console = actions.console, model.orchestrator == nil, model.nowDoing == nil {
                console.console.open(recents: true)
            } else {
                showActivity()
            }
        } label: {
            HStack(spacing: 6) {
                Text(model.orchestrator == nil ? "Go to Anything" : "Activity")
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                    // Keeps its width: the orchestrator's line is what gives way (ov-329).
                    .fixedSize()
                if actions.console != nil {
                    Text(model.orchestrator == nil ? "⌘P" : "⌘K").foregroundStyle(.tertiary).fixedSize()
                }
            }
            .contentShape(Rectangle())
        }
        .buttonStyle(.borderless)
        .help(model.nowDoing ?? "Show what’s happening in this workspace")
        .accessibilityLabel("Activity")
        .accessibilityValue(model.nowDoing ?? "")
        .accessibilityIdentifier("title-status-activity")
    }

    /// This workspace's need-you count, in the attention color, with words
    /// at the wide form and a glyph below it. Nothing at zero.
    @ViewBuilder
    private var needYou: some View {
        let count = model.needYou
        if count > 0 {
            Button(action: actions.nextNeedingYou) {
                HStack(spacing: 4) {
                    Image(systemName: "exclamationmark.bubble")
                    if form == .wide, let words = TitleStatus.needYouWords(count) {
                        Text(words)
                    } else {
                        Text(TitleStatus.number(count)).monospacedDigit()
                    }
                }
                .foregroundStyle(TitleStatus.needYouIsTinted(count) ? Tint.attention(scheme) : Color.secondary)
                .fixedSize()
            }
            .buttonStyle(.borderless)
            .help("Open the next thing in this workspace that needs you")
            .accessibilityLabel(TitleStatus.needYouLabel(count))
            .accessibilityIdentifier("title-status-need-you")
        }
    }

    /// Failed or lost agents, only above zero, in the failure color with
    /// its glyph: a menu of them, each opening its pane.
    @ViewBuilder
    private var failed: some View {
        let lines = model.failed
        if !lines.isEmpty {
            PullDownMenu(entries: lines.map { line in
                .item([line.title, line.detail].compactMap { $0 }.joined(separator: " — ")) { actions.openLine(line) }
            }) {
                HStack(spacing: 4) {
                    Image(systemName: "xmark.octagon")
                    Text(form == .wide ? TitleStatus.failedWords(lines.count) ?? "" : TitleStatus.number(lines.count))
                        .monospacedDigit()
                }
                .foregroundStyle(Tint.failure)
            }
            .help(TitleStatus.failedLabel(lines.count))
            .accessibilityLabel(TitleStatus.failedLabel(lines.count))
            .accessibilityIdentifier("title-status-failed")
        }
    }

    /// The running or in-review menu's rows: the tasks, then a Queued section.
    static func taskEntries(
        _ rows: [TaskRow], extra: [ActivityLine], open: @escaping (TaskRow) -> Void,
        openLine: @escaping (ActivityLine) -> Void
    ) -> [PullDownEntry] {
        var entries = rows.map { row in PullDownEntry.item("\(row.key) \(row.title)") { open(row) } }
        if !extra.isEmpty {
            entries.append(.header("Queued"))
            entries += extra.map { line in
                .item([line.title, line.detail].compactMap { $0 }.joined(separator: " — ")) { openLine(line) }
            }
        }
        return entries
    }

    /// Running or in review: a count, and a menu of those tasks; a pick
    /// opens it. `extra` lists queued tasks after the running ones. Nothing
    /// with neither.
    @ViewBuilder
    private func taskMenu(
        _ rows: [TaskRow], symbol: String, words: String?, label: String, id: String, extra: [ActivityLine] = []
    ) -> some View {
        if !rows.isEmpty || !extra.isEmpty {
            PullDownMenu(entries: Self.taskEntries(rows, extra: extra, open: actions.openTask, openLine: actions.openLine)) {
                HStack(spacing: 4) {
                    Image(systemName: symbol)
                    if form == .wide, let words, !words.isEmpty {
                        Text(words)
                    } else {
                        Text(TitleStatus.number(rows.count)).monospacedDigit()
                    }
                }
                .foregroundStyle(.secondary)
            }
            .help(label)
            .accessibilityLabel(label)
            .accessibilityIdentifier(id)
        }
    }
}

/// The activity panel in its popover, reading today's spend as it opens.
struct TitleActivityPopover: View {
    let activity: TitleActivity
    let actions: TitleStatusActions
    /// Where today's spend is kept between openings, by workspace: read at
    /// most once a minute, not each time the panel comes back.
    var cache: TitleConsoleModel? = nil
    let close: () -> Void

    @State private var spend: ActivitySpend = .reading

    var body: some View {
        TitleActivityPanel(
            activity: activity, spend: spend,
            onOpen: { line in
                close()
                actions.openLine(line)
            },
            onOrchestrator: {
                close()
                actions.goToOrchestrator()
            })
        .task {
            let key = cache?.console.workspace ?? ""
            if let kept = cache?.spend[key], Date().timeIntervalSince(kept.at) < 60 {
                spend = kept.spend
                return
            }
            spend = await actions.readSpend()
            cache?.spend[key] = (Date(), spend)
        }
    }
}

/// The orchestrator's state as a mark, the one the navigator's row and the
/// status area both draw: the app's own agent status mark while it works or
/// starts (ov-177: never the system's spinner), its status's mark when it
/// needs you, failed or has news, else its glyph, dimmed when nothing runs.
struct OrchestratorMark: View {
    let state: OrchestratorRow.State
    let status: Status?

    var body: some View {
        switch state {
        case .working, .starting:
            // Drawn over the frame the resting icon has, not by itself
            // (ov-260). In the navigator's row the glyph column aligns its
            // content by first-text baseline, and the pulse dot, being an
            // AppKit layer (ov-229) with no baseline of its own, sat on that
            // line's bottom edge, eight points below the icon and off its
            // center. Laid over the icon's own frame, hidden, it takes the
            // icon's place exactly, on every platform's metrics.
            restingIcon
                .hidden()
                .overlay {
                    StatusGlyph(status: status ?? (state == .starting ? .starting : .working))
                        .gridMark("orchestratorDot", .icon)
                }
        case .needsYou, .unread, .failed:
            // Over the resting icon's frame too, so it takes the icon's place
            // rather than sitting on its own baseline (ov-260 review, L4).
            // The status's own mark (ov-137): needs you was the accent here
            // and a finished turn amber, the reverse of every other row.
            restingIcon
                .hidden()
                .overlay {
                    StatusGlyph(status: state.status ?? .idle)
                        .gridMark("orchestratorDot", .icon)
                }
        case .idle:
            restingIcon.foregroundStyle(.secondary)
        case .none, .stopped:
            restingIcon.foregroundStyle(.tertiary)
        }
    }

    /// Whether the mark is a status dot in place of the orchestrator's
    /// symbol, rather than the symbol itself.
    nonisolated static func marksStatus(_ state: OrchestratorRow.State) -> Bool {
        switch state {
        case .working, .starting, .needsYou, .unread, .failed: true
        case .idle, .none, .stopped: false
        }
    }

    /// The orchestrator's symbol at rest (`OneTreeGlyph.orchestrator`).
    static let glyphName = OneTreeGlyph.orchestrator
    static var glyph: some View {
        Image(systemName: glyphName).font(.system(size: 10))
    }

    /// The glyph shown when nothing runs; its frame is the one every other
    /// state's mark occupies.
    private var restingIcon: some View { Self.glyph }
}

/// The view behind the status area, so a test can find the toolbar item
/// holding it and compare the two widths (`TitleStatusWidthTests`), as the
/// switcher's menu anchor lets `SwitcherWidthTests` do.
enum TitleStatusAnchor {
    final class View: NSView {}

    struct Mark: NSViewRepresentable {
        func makeNSView(context: Context) -> NSView { View() }
        func updateNSView(_ view: NSView, context: Context) {}
    }
}

/// The status area in the toolbar: in the center, on no glass, and
/// measured again whenever its form changes (and only then).
struct TitleStatusItem: ToolbarContent {
    let source: TitleStatusSource
    let form: TitleStatus.Form
    let actions: TitleStatusActions

    var body: some ToolbarContent {
        ToolbarItem(placement: .principal) {
            // The form, not the text: a "now doing" line changes many times
            // a minute, and rebuilding the item for each would close its
            // menu and move VoiceOver's focus. The text truncates inside the
            // form's fixed width instead.
            TitleStatusBoard(source: source, form: form, actions: actions)
                // Still, not breathing: this mark is on screen however long an
                // agent works, the window hidden or not (ov-229).
                .environment(\.statusGlyphStill, true)
                .id(form)
        }
        // On no glass (ov-291, reversing ov-263): Apple's own example of a
        // principal status item hides the shared background, and HIG asks for
        // fewer toolbar backgrounds, not a slab around buttons that already sit
        // on the window's plane. The field a ⌘K opens draws its own bezel.
        .sharedBackgroundVisibility(.hidden)
    }
}

/// The status area over its board: observed here, so the counts follow it.
private struct TitleStatusBoard: View {
    let source: TitleStatusSource
    let form: TitleStatus.Form
    let actions: TitleStatusActions

    var body: some View {
        if let store = source.board {
            Observed(store: store, source: source, form: form, actions: actions)
        } else {
            TitleStatusView(
                model: TitleStatus.model(source, board: .empty), form: form, actions: actions,
                activity: TitleStatus.activity(source, board: .empty, starts: [:]))
        }
    }

    private struct Observed: View {
        @ObservedObject var store: TaskBoardStore
        let source: TitleStatusSource
        let form: TitleStatus.Form
        let actions: TitleStatusActions

        var body: some View {
            TitleStatusView(
                model: TitleStatus.model(source, board: store.board, starts: store.starts), form: form,
                actions: actions, activity: TitleStatus.activity(source, board: store.board, starts: store.starts))
        }
    }
}

/// What the window's leading and trailing items take, for the status area
/// to size itself around.
struct TitleStatusRoom: Equatable {
    let switcherTitle: String
    let switcherRepository: String
    let editor: Bool
    let changes: Bool
    /// Whether Show Files is in the toolbar (ov-189).
    let files: Bool
    let trouble: String?
    /// The tray's count, every workspace's.
    let needsYou: Int
    /// Whether the window offers Back and Forward in its leading group
    /// (slice 3); they're drawn only where they leave the status area
    /// room for its medium form (`layout`).
    var backForward = false
    /// The leading and trailing items' widths, measured once, here: the
    /// window reads `layout` several times a pass (ov-229).
    let leadingWidth: CGFloat
    let trailingWidth: CGFloat

    init(
        switcherTitle: String, switcherRepository: String, editor: Bool, changes: Bool, files: Bool = false,
        trouble: String?, needsYou: Int = 0, backForward: Bool = false
    ) {
        self.switcherTitle = switcherTitle
        self.switcherRepository = switcherRepository
        self.editor = editor
        self.changes = changes
        self.files = files
        self.trouble = trouble
        self.needsYou = needsYou
        self.backForward = backForward
        leadingWidth = TitleStatus.leading(switcher: switcherTitle, repository: switcherRepository)
        trailingWidth = TitleStatus.trailing(
            editor: editor, changes: changes, files: files, trouble: trouble, needsYou: needsYou)
    }

    /// What Back and Forward take beside the switcher: their own capsule,
    /// two 28 pt buttons, and the space before it.
    static let backForwardWidth: CGFloat = 64 + 12

    /// The status area's form in a window `window` wide, and whether Back
    /// and Forward are drawn: with them while the area keeps its medium
    /// form or wider, else without them, since what the orchestrator is
    /// doing is worth more than two buttons ⌃⌘← and ⌃⌘→ already press.
    func layout(window: CGFloat) -> (form: TitleStatus.Form, backForward: Bool) {
        let (leading, trailing) = (leadingWidth, trailingWidth)
        if backForward {
            let with = TitleStatus.form(
                available: TitleStatus.available(
                    window: window, leading: leading + Self.backForwardWidth, trailing: trailing))
            if with >= .medium { return (with, true) }
        }
        return (TitleStatus.form(available: TitleStatus.available(window: window, leading: leading, trailing: trailing)), false)
    }

    func form(window: CGFloat) -> TitleStatus.Form { layout(window: window).form }
}

private struct TitleStatusModifier: ViewModifier {
    let source: TitleStatusSource?
    let room: TitleStatusRoom
    let actions: TitleStatusActions
    /// The window's width, kept where the window can read it too (Back and
    /// Forward are drawn by it): this is applied to the window's root view.
    @Binding var width: CGFloat

    func body(content: Content) -> some View {
        content
            .onGeometryChange(for: CGFloat.self) { $0.size.width } action: { width = $0 }
            .toolbar {
                if let source {
                    TitleStatusItem(source: source, form: room.form(window: width), actions: actions)
                }
            }
    }
}

extension View {
    /// The title bar's status area, for the window this is the root of.
    /// Nothing with no workspace on screen (`source` nil). `width` is the
    /// window's, measured here.
    func titleBarStatus(
        _ source: TitleStatusSource?, room: TitleStatusRoom, actions: TitleStatusActions, width: Binding<CGFloat>
    ) -> some View {
        modifier(TitleStatusModifier(source: source, room: room, actions: actions, width: width))
    }
}

/// The main window's chrome (ov-214): the system's regular unified toolbar,
/// 52 pt with 36 pt controls, as Xcode and Finder have it (the compact one
/// is 40 with 24, and glass that short looked wrong). The scene asks
/// for it (`FarCoolerApp`), and the window's root sets it on its window as
/// well, so a window made any other way, a test's included, is the same.
enum MainWindowChrome {
    static let toolbarStyle: NSWindow.ToolbarStyle = .unified
    /// The main window's narrowest: the regular bar holds every item at it,
    /// the tray and Show Files included. They need 605 pt (integ-9,
    /// measured); 600 sent the tray to the overflow menu. 640 leaves some room.
    static let minimumWidth: CGFloat = 640

    /// The toolbar's band shows the window's plane (ov-289): a transparent
    /// titlebar with no rule under it, so the toolbar, the navigator and the
    /// gutters are one frosted surface, and Increase Contrast's and Reduce
    /// Transparency's opaque fallback is the plane's own.
    static func unify(_ window: NSWindow) {
        if window.toolbarStyle != toolbarStyle { window.toolbarStyle = toolbarStyle }
        if !window.titlebarAppearsTransparent { window.titlebarAppearsTransparent = true }
        if window.titlebarSeparatorStyle != .none { window.titlebarSeparatorStyle = .none }
    }

    struct Setter: NSViewRepresentable {
        final class Probe: NSView {
            override func viewDidMoveToWindow() {
                super.viewDidMoveToWindow()
                if let window { MainWindowChrome.unify(window) }
            }
        }
        func makeNSView(context: Context) -> NSView { Probe() }
        func updateNSView(_ view: NSView, context: Context) {}
    }
}

extension View {
    /// This view's window in the main window's chrome (`MainWindowChrome`).
    func mainWindowChrome() -> some View { background(MainWindowChrome.Setter()) }
}
