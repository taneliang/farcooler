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
            VStack(spacing: 0) {
                if !stream.transcript.plan.isEmpty {
                    PlanPanel(entries: stream.transcript.plan)
                    Divider()
                }

                transcript(viewportHeight: proxy.size.height)

                if let pending = stream.transcript.pendingPermission {
                    ApprovalCard(pending: pending) { optionID in
                        Task { await stream.answer(pending.id, optionID) }
                    }
                    .padding(10)
                }

                AgentComposer(
                    stream: stream, terminal: terminal, isFocused: isFocused,
                    searchFiles: searchFiles)
            }
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

    private func transcript(viewportHeight: CGFloat) -> some View {
        ScrollViewReader { proxy in
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 14) {
                    ForEach(stream.transcript.rows) { row in
                        AgentRowView(row: row)
                            .id(row.id)
                    }
                }
                // Roomier than a terminal, on purpose. A VT grid is dense
                // because every cell is addressable; prose is read, and the
                // line spacing a terminal wants makes a paragraph a wall.
                .padding(.horizontal, 16)
                .padding(.top, 14)
                .padding(.bottom, 18)
                .frame(maxWidth: .infinity, alignment: .leading)

                // Room to scroll past the end.
                //
                // Two jobs. It stops the last line sitting flush against the
                // composer, which reads as content clipped rather than content
                // finished. And it is what makes "put my question at the top"
                // possible at all: without space below, a short exchange
                // cannot scroll far enough for its first line to reach the
                // top of the view.
                Color.clear.frame(height: max(120, viewportHeight * 0.45))
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
            .onChange(of: stream.transcript.cursor) { _, _ in
                // A message the user just sent goes to the TOP and stays
                // there, with the answer streaming into the space below — the
                // reading position Cursor uses, and the right one: the
                // question is the context for everything that follows, and
                // chasing the bottom of a growing reply moves the text you are
                // trying to read.
                if let pinned = stream.pinnedRow {
                    withAnimation(.snappy) { proxy.scrollTo(pinned, anchor: .top) }
                    return
                }
                guard let last = stream.transcript.rows.last else { return }
                // Not `.bottom`: that puts the final line flush against the
                // composer. Just above it, so the text has somewhere to sit.
                withAnimation(.snappy) {
                    proxy.scrollTo(last.id, anchor: UnitPoint(x: 0, y: 0.88))
                }
            }
            .onAppear {
                guard let last = stream.transcript.rows.last else { return }
                proxy.scrollTo(last.id, anchor: .bottom)
            }
        }
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

/// The agent's own plan, shown wholesale because the daemon sends it
/// wholesale — see `Transcript.apply`'s comment on why it replaces rather
/// than appends.
private struct PlanPanel: View {
    let entries: [PlanEntry]

    var body: some View {
        VStack(alignment: .leading, spacing: 3) {
            ForEach(Array(entries.enumerated()), id: \.offset) { _, entry in
                HStack(alignment: .top, spacing: 6) {
                    Image(systemName: symbol(for: entry.status))
                        .font(.system(size: 10))
                        .foregroundStyle(isDone(entry.status) ? Color.green : .secondary)
                    Text(entry.content)
                        .font(.system(size: 11.5))
                        .strikethrough(isDone(entry.status))
                        .foregroundStyle(isDone(entry.status) ? .secondary : .primary)
                    Spacer(minLength: 0)
                }
            }
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 8)
        .background(Color.primary.opacity(0.03))
    }

    private func isDone(_ status: String) -> Bool {
        status.lowercased().contains("done") || status.lowercased().contains("complet")
    }

    private func symbol(for status: String) -> String {
        let lowered = status.lowercased()
        if isDone(lowered) { return "checkmark.circle.fill" }
        if lowered.contains("progress") || lowered.contains("active") { return "circle.lefthalf.filled" }
        return "circle"
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

            HStack(spacing: 8) {
                ForEach(Array(pending.options.enumerated()), id: \.element.id) { index, option in
                    Button(option.name) { onChoose(option.id) }
                        .modifier(NumberedShortcut(index: index))
                }
            }
        }
        .padding(12)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Color.orange.opacity(0.08), in: RoundedRectangle(cornerRadius: 8))
        .overlay(RoundedRectangle(cornerRadius: 8).strokeBorder(Color.orange.opacity(0.3)))
    }
}

/// The first option is also what Return chooses, and the first three each get
/// their own digit — the plan's own wording for this card. A fourth or later
/// option is still reachable, just by clicking; a keyboard that runs out of
/// digits before an agent runs out of options is not worth over-engineering
/// for the sessions that never offer four.
private struct NumberedShortcut: ViewModifier {
    let index: Int

    func body(content: Content) -> some View {
        if index == 0 {
            content.keyboardShortcut(.defaultAction)
        } else if index < 3 {
            content.keyboardShortcut(KeyEquivalent(Character("\(index + 1)")), modifiers: [])
        } else {
            content
        }
    }
}
