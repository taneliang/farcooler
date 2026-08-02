import AgentKit
import SwiftUI

/// The agent pane: a chat drawn into the same rectangle a terminal would
/// occupy.
///
/// `TerminalPane`'s doc comment explains why a terminal grew no header or
/// footer — both restated what the sidebar already showed. This surface keeps
/// that rule: no title bar, no permanent status line. What a terminal pane
/// does not need at all, this one folds into the one row that earns its
/// place — the composer, in `AgentComposer` — rather than adding a second
/// strip above the transcript.
///
/// `TileView` computes no arrangement for the same reason it never has: this
/// view is handed the pixel rectangle tmux already assigned its pane, and
/// draws inside it. The one thing it still owes the layout, same as a
/// terminal, is an honest cell size back — see `report(size:)`.
struct AgentSurface: View {
    let terminal: Terminal
    let binary: String?
    let environment: [String: String]
    let hostArguments: [String]
    let isFocused: Bool
    let searchFiles: (String) async -> [String]
    /// Report this pane's grid, in cells. Empty or dishonest here and tmux
    /// lays the whole window out against stale numbers — see the doc comment
    /// on `report(size:)`.
    let onResize: (Int, Int) async -> Void

    @StateObject private var stream: AgentStream
    @ObservedObject private var preferences = Preferences.shared
    @State private var lastReportedGeometry: (columns: Int, rows: Int) = (0, 0)
    /// How much of the pane's bottom the floating composer covers.
    @State private var composerHeight: CGFloat = 0
    /// The pane's own height, so the fade can be placed against the composer.
    @State private var paneHeight: CGFloat = 0

    init(
        terminal: Terminal, binary: String?, environment: [String: String],
        hostArguments: [String], isFocused: Bool,
        searchFiles: @escaping (String) async -> [String],
        onResize: @escaping (Int, Int) async -> Void
    ) {
        self.terminal = terminal
        self.binary = binary
        self.environment = environment
        self.hostArguments = hostArguments
        self.isFocused = isFocused
        self.searchFiles = searchFiles
        self.onResize = onResize
        // One stream per terminal, and the caller forces a fresh instance
        // whenever the terminal changes — see `.id(terminal.id)` at both call
        // sites — the same way `TerminalSurface` is re-attached rather than
        // updated in place when the selected terminal changes underneath it.
        _stream = StateObject(wrappedValue: AgentStream(terminal: terminal.short))
    }

    var body: some View {
        GeometryReader { proxy in
            // The composer floats OVER the transcript rather than sitting
            // under it, so the conversation runs the full height of the pane
            // and scrolls behind the control resting on top of it. The
            // transcript pads its own tail by however tall that control is, so
            // the last line can still be read.
            ZStack(alignment: .bottom) {
                VStack(spacing: 0) {
                    if !stream.transcript.plan.isEmpty {
                        PlanPanel(entries: stream.transcript.plan)
                        Divider()
                    }

                    transcriptView(bottomInset: composerHeight + 20)
                }

                // Only when there is no tool call to hang it on. A permission
                // names the call it gates, so the buttons normally live on that
                // row — see `AgentRowView.pending`. This is the fallback for a
                // request about something the transcript is not showing.
                if let pending = unattachedPermission {
                    ApprovalCard(pending: pending) { optionID in
                        Task { await stream.answer(pending.id, optionID) }
                    }
                    .padding(10)
                }

                AgentComposer(
                    stream: stream, terminal: terminal, isFocused: isFocused,
                    searchFiles: searchFiles)
                    .padding(10)
                    .background {
                        GeometryReader { composer in
                            Color.clear.preference(
                                key: ComposerHeightKey.self, value: composer.size.height)
                        }
                    }
            }
            .onPreferenceChange(ComposerHeightKey.self) { composerHeight = $0 }
            .onChange(of: proxy.size.height, initial: true) { _, height in paneHeight = height }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            // The native content background, NOT `Palette.background`.
            //
            // That constant is the VT grid's own near-black, and it is right
            // for a terminal because a terminal draws its own colours into
            // every cell. A chat draws none: it uses `.primary` and
            // `.secondary`, which resolve against the SYSTEM appearance. So
            // painting the VT colour under them put black text on a near-black
            // panel in light mode — unreadable, and sitting in the middle of an
            // otherwise light window like a hole.
            //
            // `textBackgroundColor` is what every native document surface uses,
            // so this follows the appearance the rest of the app already does.
            .background(Color(nsColor: .textBackgroundColor))
            // Debounced by `.task(id:)` cancelling its predecessor, the same
            // trick `TileView.panels` uses against a window drag producing
            // one of these per frame.
            .task(id: GeometryKey(size: proxy.size, font: preferences.revision)) {
                try? await Task.sleep(for: .milliseconds(200))
                guard !Task.isCancelled else { return }
                await report(size: proxy.size)
            }
        }
        .onAppear {
            stream.start(binary: binary, environment: environment, hostArguments: hostArguments)
        }
        .onDisappear { stream.stop() }
        // The CLI path can arrive after this view does — the service is
        // still resolving it on first launch — so a stream started against
        // `nil` has to be restarted once a real binary shows up rather than
        // staying inert for the life of the pane.
        .onChange(of: binary) { _, newBinary in
            stream.start(binary: newBinary, environment: environment, hostArguments: hostArguments)
        }
    }

    private func transcriptView(bottomInset: CGFloat) -> some View {
        ScrollViewReader { proxy in
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 14) {
                    ForEach(stream.transcript.rows) { row in
                        AgentRowView(
                            row: row,
                            isLast: row.id == stream.transcript.rows.last?.id,
                            pending: permission(gating: row),
                            onAnswer: { optionID in
                                guard let id = stream.transcript.pendingPermission?.id else {
                                    return
                                }
                                Task { await stream.answer(id, optionID) }
                            }
                        )
                        .id(row.id)
                    }

                    // The turn that is still running, one line ahead of what it
                    // has produced.
                    if terminal.agent == .working {
                        WorkingRow()
                    }

                    // Written, not yet sent. Below the working row because
                    // that is where they are in time: after everything the
                    // agent is doing now.
                    ForEach(stream.transcript.queue) { queued in
                        QueuedRow(
                            queued: queued,
                            onEdit: { text in Task { await stream.editQueued(queued.id, text) } },
                            onCancel: { Task { await stream.cancelQueued(queued.id) } })
                    }
                }
                // Roomier than a terminal, on purpose. A VT grid is dense
                // because every cell is addressable; prose is read, and the
                // line spacing a terminal wants makes a paragraph a wall.
                .padding(.horizontal, 16)
                .padding(.top, 14)
                .padding(.bottom, 4)

                // The end of the content, and what autoscroll actually targets.
                //
                // Scrolling to the last ROW aligns that row's bottom with the
                // viewport's, which is now UNDERNEATH the composer floating over
                // it — the newest line, the one being written, ends up the one
                // hidden. Scrolling to the end of the content instead leaves
                // exactly this spacer covered, and the last line clear of the
                // glass.
                Color.clear
                    .frame(height: bottomInset)
                    .id(Self.endOfTranscript)
                .frame(maxWidth: .infinity, alignment: .leading)
            }
            // Pinned to the bottom: a chat is read at its most recent line,
            // the same reason a terminal scrolls to follow its own output.
            // Keyed on the CURSOR, not the row count.
            //
            // A streamed reply arrives as chunks that coalesce into the row
            // already on screen, so the count does not change and a
            // count-keyed scroll sits still while text grows off the bottom.
            // The cursor moves for every event, which is exactly when there is
            // something new to see.
            // Dissolved into the composer rather than cut off by it.
            //
            // The transcript runs the full height of the pane and the composer
            // floats over its last inch, so a line scrolling past simply
            // reappeared below the card, sliced in half. A fade over the same
            // band the card occupies makes the two read as one surface — text
            // going under something — instead of two overlapping ones.
            .mask {
                LinearGradient(
                    stops: [
                        .init(color: .black, location: 0),
                        .init(color: .black, location: max(0, fadeStart - 0.06)),
                        .init(color: .clear, location: fadeStart),
                    ],
                    startPoint: .top,
                    endPoint: .bottom)
            }
            .onChange(of: stream.transcript.cursor) { _, _ in
                // The bottom, always.
                //
                // This used to pin a sent message to the top of the view with
                // the reply streaming in beneath it — Cursor's reading
                // position, and a nice one. Getting it right needs a spacer
                // sized to the viewport that appears and collapses with the
                // turn, and every version of that left content stranded
                // somewhere unreachable. Following the newest line is what a
                // terminal does, it is what this pane replaced, and it is never
                // wrong about where the reader is looking.
                withAnimation(Motion.snap) {
                    proxy.scrollTo(Self.endOfTranscript, anchor: .bottom)
                }
            }
            .onAppear {
                proxy.scrollTo(Self.endOfTranscript, anchor: .bottom)
            }
        }
    }

    /// Where the transcript starts fading out, as a fraction of its height:
    /// the top of the floating composer.
    private var fadeStart: CGFloat {
        guard paneHeight > 0 else { return 1 }
        return min(1, max(0.5, 1 - (composerHeight + 10) / paneHeight))
    }

    /// The anchor at the very end of the transcript's content.
    private static let endOfTranscript = "end-of-transcript"

    /// The pending permission, if it is this row that it is asking about.
    private func permission(gating row: TranscriptRow) -> PendingPermission? {
        guard let pending = stream.transcript.pendingPermission,
            case let .tool(tool) = row.kind, tool.id == pending.toolCall
        else { return nil }
        return pending
    }

    /// A request naming a tool call the transcript does not have a row for.
    ///
    /// It should not happen — a permission follows the call it is about — but
    /// an unanswerable request that is also invisible would wedge the agent
    /// with no way for anyone to see why.
    private var unattachedPermission: PendingPermission? {
        guard let pending = stream.transcript.pendingPermission else { return nil }
        let shown = stream.transcript.rows.contains { row in
            if case let .tool(tool) = row.kind { return tool.id == pending.toolCall }
            return false
        }
        return shown ? nil : pending
    }

    /// Everything that changes what this view's size is worth in cells.
    private struct GeometryKey: Equatable {
        var size: CGSize
        var font: Int
    }

    /// Tell tmux how big this pane's grid is.
    ///
    /// Computed from pixels with the exact same font metrics and padding
    /// `TerminalRenderView.reportGeometry` uses for a terminal pane in the
    /// same rectangle — not because this surface draws a VT grid (it does
    /// not), but because the pane underneath is a real tmux pane either way,
    /// and reporting a made-up or absent size here is indistinguishable, from
    /// tmux's side, from lying about it. A pane that lies is a pane whose
    /// neighbours get sized against numbers that are not true.
    private func report(size: CGSize) async {
        let cell = TerminalMetrics.cell(preferences.terminalFont())
        guard cell.width > 0, cell.height > 0 else { return }

        let usableWidth = size.width - TerminalMetrics.padding.left - TerminalMetrics.padding.right
        let usableHeight = size.height - TerminalMetrics.padding.top - TerminalMetrics.padding.bottom
        guard usableWidth > 0, usableHeight > 0 else { return }

        let columns = max(20, Int(usableWidth / cell.width))
        let rows = max(5, Int(usableHeight / cell.height))
        guard (columns, rows) != lastReportedGeometry else { return }
        lastReportedGeometry = (columns, rows)
        await onResize(columns, rows)
    }
}

// MARK: - Plan

/// The agent's task list, as the agent maintains it.
///
/// ACP calls this a plan and sends it whole on every change, which is why it is
/// replaced rather than appended to — see `Transcript.apply`. It used to render
/// as a thin strip of bullets, which is the same information and none of the
/// use: what a reader wants from a task list is how far through it is and what
/// is happening RIGHT NOW, and neither was visible without reading every line.
///
/// Pinned above the transcript rather than placed in it. The list is current
/// state, not something that was said at a moment — inline it would scroll away
/// exactly when the work it describes is still going on.
private struct PlanPanel: View {
    let entries: [PlanEntry]

    @State private var expanded = true

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Button {
                withAnimation(Motion.snap) { expanded.toggle() }
            } label: {
                HStack(spacing: 6) {
                    Image(systemName: "chevron.right")
                        .font(.caption2)
                        .rotationEffect(.degrees(expanded ? 90 : 0))
                    Text("Tasks")
                        .font(.caption.weight(.semibold))
                    Text("\(doneCount) of \(entries.count)")
                        .font(.caption)
                        .foregroundStyle(.secondary)

                    // The one currently being worked on, named in the header —
                    // so a collapsed list still answers the question people
                    // actually open it to ask.
                    if !expanded, let active {
                        Text("· \(active.content)")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                            .lineLimit(1)
                    }
                    Spacer(minLength: 0)
                }
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .padding(.horizontal, 12)
            .padding(.vertical, 7)

            if expanded {
                VStack(alignment: .leading, spacing: 4) {
                    ForEach(Array(entries.enumerated()), id: \.offset) { _, entry in
                        HStack(alignment: .top, spacing: 7) {
                            Image(systemName: symbol(for: entry.status))
                                .font(.system(size: 10))
                                .foregroundStyle(tint(for: entry.status))
                                .frame(width: 12)
                            Text(entry.content)
                                .font(.system(size: 11.5))
                                .strikethrough(isDone(entry.status))
                                .foregroundStyle(colour(for: entry.status))
                                .fixedSize(horizontal: false, vertical: true)
                            Spacer(minLength: 0)
                        }
                    }
                }
                .padding(.horizontal, 12)
                .padding(.bottom, 8)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Color.primary.opacity(0.03))
    }

    private var doneCount: Int { entries.filter { isDone($0.status) }.count }

    private var active: PlanEntry? { entries.first { isActive($0.status) } }

    private func isDone(_ status: String) -> Bool {
        let lowered = status.lowercased()
        return lowered.contains("done") || lowered.contains("complet")
    }

    private func isActive(_ status: String) -> Bool {
        let lowered = status.lowercased()
        return lowered.contains("progress") || lowered.contains("active")
    }

    private func symbol(for status: String) -> String {
        if isDone(status) { return "checkmark.circle.fill" }
        if isActive(status) { return "circle.lefthalf.filled" }
        return "circle"
    }

    private func tint(for status: String) -> Color {
        if isDone(status) { return .green }
        if isActive(status) { return .accentColor }
        return .secondary
    }

    private func colour(for status: String) -> Color {
        if isDone(status) { return .secondary }
        return .primary
    }
}

// MARK: - Approval

/// A permission request, as an inline card rather than a sheet — the agent is
/// paused waiting on it, so it belongs in the flow of the conversation it
/// interrupted, not in a window layered on top of it.
private struct ApprovalCard: View {
    let pending: PendingPermission
    let onChoose: (String) -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(spacing: 6) {
                Image(systemName: "exclamationmark.circle.fill")
                Text("Needs your approval")
                    .font(.system(size: 12, weight: .semibold))
            }
            .foregroundStyle(.orange)

            ApprovalControls(options: pending.options, onChoose: onChoose)
        }
        .padding(12)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Color.orange.opacity(0.08), in: RoundedRectangle(cornerRadius: 8))
        .overlay(RoundedRectangle(cornerRadius: 8).strokeBorder(Color.orange.opacity(0.3)))
    }
}

/// How tall the floating composer is, so the transcript can pad past it.
private struct ComposerHeightKey: PreferenceKey {
    static let defaultValue: CGFloat = 0

    static func reduce(value: inout CGFloat, nextValue: () -> CGFloat) {
        value = max(value, nextValue())
    }
}
