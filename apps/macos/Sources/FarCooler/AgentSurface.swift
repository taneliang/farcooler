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
    /// Whether the transcript should follow its own tail — true while the
    /// reader is parked at the bottom, false once they scroll away.
    @State private var followingTail = true
    /// The row the scroll view holds still while heights around it resolve.
    @State private var scrollPosition = ScrollPosition(idType: Int.self)

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
            // The composer sits in the transcript's bottom safe area, which is
            // the framework's own answer to "a control resting on scrolling
            // content": the conversation runs the full height of the pane and
            // scrolls behind it, and the scroll view insets itself so the last
            // line is still reachable.
            //
            // This was built by hand first — a `ZStack`, a `GeometryReader`
            // measuring the composer, a `PreferenceKey` carrying that height up,
            // a spacer sized from it, and a gradient mask placed against it.
            // That is four pieces of machinery to restate one thing the
            // framework already knows, and it PEGGED A CORE: the measurement fed
            // a layout that fed the measurement, so every frame invalidated the
            // next one, with a full-content mask re-composited each time.
            //
            // `safeAreaInset` has no measurement to feed back.
            transcriptView
            // How the conversation MEETS the glass over it.
            //
            // `safeAreaInset` places the composer and insets the scroll view;
            // this says what the boundary looks like. Without it the last line
            // passes under a hard edge — and it is exactly what the hand-built
            // gradient mask was reaching for, the one that fed a layout that fed
            // itself and pegged a core. Available now that the floor is macOS 26.
            .scrollEdgeEffectStyle(.soft, for: .bottom)
            .safeAreaInset(edge: .bottom, spacing: 0) {
                GlassEffectContainer(spacing: 8) {
                    VStack(spacing: 8) {
                        // The plan and the queue are ATTACHED to the composer, not
                        // scattered around the pane.
                        //
                        // The plan used to be pinned above the transcript and the
                        // queue drawn inline at its end, which put three things
                        // that are all "what happens next" in three different
                        // places — and the plan, at the far top, was furthest from
                        // the message that would change it. Everything about the
                        // NEXT turn now sits together, directly over the box you
                        // type into.
                        if !stream.transcript.plan.isEmpty {
                            PlanPanel(entries: stream.transcript.plan)
                                .padding(.horizontal, 10)
                        }

                        ForEach(stream.transcript.queue) { queued in
                            QueuedRow(
                                queued: queued,
                                onEdit: { text in Task { await stream.editQueued(queued.id, text) } },
                                onCancel: { Task { await stream.cancelQueued(queued.id) } },
                                onSteer: { Task { await stream.steerQueued(queued.id) } })
                                .padding(.horizontal, 10)
                        }

                        // Only when there is no tool call to hang it on. A
                        // permission names the call it gates, so the buttons
                        // normally live on that row — see `AgentRowView.pending`.
                        // This is the fallback for a request about something the
                        // transcript is not showing.
                        if let pending = unattachedPermission {
                            ApprovalCard(pending: pending) { optionID in
                                Task { await stream.answer(pending.id, optionID) }
                            }
                            .padding(.horizontal, 10)
                            .padding(.bottom, 8)
                        }

                        AgentComposer(
                            stream: stream, terminal: terminal, isFocused: isFocused,
                            searchFiles: searchFiles,
                            // The pane's own width, passed down as a plain number.
                            //
                            // Safe in a way the last two attempts were not: the
                            // composer's WIDTH does not depend on its contents — it
                            // fills what it is given — so a layout chosen from it
                            // cannot feed back into it. The freeze came from height,
                            // which does.
                            width: proxy.size.width)
                            .padding(10)
                    }
                }
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

    private var transcriptView: some View {
        ScrollView {
            // Lazy, and ANCHORED — which is the pair that makes it work.
            //
            // A lazy stack estimates the height of rows it has not built and
            // corrects itself as they scroll in. With rows as uneven as these —
            // a one-word message beside two hundred lines of tool output — the
            // corrections are large, and scrolling up made the content jump
            // under the pointer.
            //
            // Building every row eagerly fixes that and is the wrong trade: a
            // transcript runs to thousands of rows and each one lays out
            // markdown. The virtualisation is worth keeping; what was missing is
            // telling the scroll view which row to hold still while the
            // estimates around it resolve. That is `scrollPosition` below — a
            // height correction above the viewport then moves the SCROLLBAR,
            // which is honest because the content really did get taller, without
            // moving what is being read.
            LazyVStack(alignment: .leading, spacing: 14) {
                ForEach(stream.transcript.rows) { row in
                    AgentRowView(
                        row: row,
                        isLast: row.id == stream.transcript.rows.last?.id,
                        pending: permission(gating: row),
                        onAnswer: { optionID in
                            guard let id = stream.transcript.pendingPermission?.id else { return }
                            Task { await stream.answer(id, optionID) }
                        }
                    )
                    .id(row.id)
                }

                // The turn that is still running, one line ahead of what it has
                // produced.
                if terminal.agent == .working {
                    WorkingRow()
                }
            }
            // Roomier than a terminal, on purpose. A VT grid is dense because
            // every cell is addressable; prose is read, and the line spacing a
            // terminal wants makes a paragraph a wall.
            .padding(.horizontal, 16)
            .padding(.top, 14)
            .padding(.bottom, 4)

            // The end of the content, and what following the tail targets.
            Color.clear
                .frame(height: 1)
                .id(Self.endOfTranscript)
        }
        // What the scroll view holds still.
        //
        // Bound rather than merely observed: setting it scrolls, and SwiftUI
        // keeps whatever it names in place while content around it changes
        // height. That second half is the point — it is what stops a lazy
        // stack's corrections from dragging the text out from under the reader.
        .scrollPosition($scrollPosition, anchor: .bottom)
        .onScrollGeometryChange(for: Bool.self) { geometry in
            // Whether the reader is parked at the tail. 40pt of slack, because
            // "at the bottom" after a redraw is rarely exact.
            geometry.contentOffset.y + geometry.containerSize.height
                >= geometry.contentSize.height - 40
        } action: { _, atBottom in
            followingTail = atBottom
        }
        // Keyed on the CURSOR, not the row count: a streamed reply coalesces
        // into the row already on screen, so the count does not change while
        // the text grows off the bottom.
        .onChange(of: stream.transcript.cursor) { _, _ in
            // Only while the reader is at the tail. Scrolling to the end on
            // every event made reading anything older impossible — a streamed
            // reply fires several a second, and each one yanked the view down.
            guard followingTail else { return }
            withAnimation(Motion.snap) {
                scrollPosition.scrollTo(id: Self.endOfTranscript, anchor: .bottom)
            }
        }
        .onAppear { scrollPosition.scrollTo(id: Self.endOfTranscript, anchor: .bottom) }
    }

    /// The end of the transcript's content.
    ///
    /// `Int`, like every row id, because `scrollPosition(id:)` binds ONE type —
    /// a `String` sentinel among `Int` rows could never be named as the anchor.
    private static let endOfTranscript = Int.max

    /// The pending permission, if it is this row that it is asking about.
    ///
    /// A subagent's call is not a row of the transcript — it is a child of a
    /// block — so matching only top-level `.tool` rows left every request
    /// raised inside a subagent looking unattached, and the buttons appeared in
    /// the fallback card at the bottom of the pane, away from the command they
    /// were approving. `SubagentBlockView` hands the request down to the child
    /// that named it.
    private func permission(gating row: TranscriptRow) -> PendingPermission? {
        guard let pending = stream.transcript.pendingPermission else { return nil }
        switch row.kind {
        case let .tool(tool):
            return tool.id == pending.toolCall ? pending : nil
        case let .subagent(block):
            return block.children.contains { child in
                if case let .tool(tool) = child.kind { return tool.id == pending.toolCall }
                return false
            } ? pending : nil
        default:
            return nil
        }
    }

    /// A request naming a tool call the transcript has no row for.
    ///
    /// It should not happen — a permission follows the call it is about — but
    /// an unanswerable request that is also invisible would wedge the agent
    /// with no way for anyone to see why.
    ///
    /// Searches inside blocks for the same reason `permission(gating:)` does,
    /// and it must search exactly as far: a request this said was unattached
    /// while the row view had already drawn it on a child would render the same
    /// buttons twice.
    private var unattachedPermission: PendingPermission? {
        guard let pending = stream.transcript.pendingPermission else { return nil }
        let shown = stream.transcript.rows.contains { row in
            permission(gating: row) != nil
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
                    Text("\(entries.doneCount) of \(entries.count)")
                        .font(.caption)
                        .foregroundStyle(.secondary)

                    // The one currently being worked on, named in the header —
                    // so a collapsed list still answers the question people
                    // actually open it to ask.
                    if !expanded, let active = entries.active {
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
                            Image(systemName: PlanStatus(entry.status).symbol)
                                .font(.system(size: 10))
                                .foregroundStyle(PlanStatus(entry.status).tint)
                                .frame(width: 12)
                            Text(entry.content)
                                .font(.system(size: 11.5))
                                .strikethrough(PlanStatus(entry.status).isDone)
                                .foregroundStyle(PlanStatus(entry.status).isDone ? .secondary : .primary)
                                .fixedSize(horizontal: false, vertical: true)
                            Spacer(minLength: 0)
                        }
                    }
                }
                .padding(.horizontal, 12)
                .padding(.bottom, 8)
            }
        }
        // OPAQUE, because it floats over a scrolling transcript.
        //
        // `.quinary` is a translucent fill, which was fine when this was a
        // header with nothing behind it. Over the conversation it let the text
        // through, and expanding the list turned both into one unreadable
        // overlap. A material is the platform's answer to "something legible
        // resting on content that moves under it".
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 8))
        .overlay(RoundedRectangle(cornerRadius: 8).strokeBorder(.quaternary))
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
