import PhotosUI
import SwiftUI
import UIKit

/// One agent session, full screen — the surface `TerminalView` swaps in for
/// `TerminalSurface` when `terminal.isAgentPane`.
///
/// Unlike the Mac, there is no tmux rectangle to draw into: this pane is
/// always the whole screen, and the tab strip that lets you leave it lives
/// below it exactly the way it lives below a terminal — `TerminalView` owns
/// that, not this view. Nothing here is new permanent chrome: mode and
/// attachments live in the composer row, matching the constraint that ruled
/// out a second header for this surface the way `TerminalPane` has none.
@MainActor
struct AgentView: View {
    let terminalID: String
    let workspaceID: String?
    @ObservedObject var connection: Connection
    /// Whether this pane is the one on screen.
    ///
    /// The view stays mounted when it is not — that is what keeps the
    /// transcript's scroll position and the composer's draft — so the polling
    /// subscription has to be started and stopped from here rather than from
    /// `onAppear`/`onDisappear`, which no longer fire on a pane switch.
    var isVisible: Bool = true

    @StateObject private var stream: AgentStream
    /// How far the keyboard — the docked composer included — reaches up the
    /// screen. See `KeyboardInset`.
    @StateObject private var keyboard = KeyboardInset()
    /// How tall the docked composer measured. Reported up out of `DockedBar`.
    @State private var barHeight: CGFloat = 0
    /// Whether the transcript should follow its own tail — true while the
    /// reader is parked at the bottom, false once they scroll away.
    @State private var followingTail = true
    /// Holds the reader's pre-keyboard intent across the viewport animation.
    ///
    /// The keyboard and the scroll view do not update in one layout pass: the
    /// viewport becomes shorter first, which briefly makes an actually
    /// tail-following transcript report that it is no longer at the bottom.
    /// Remembering the intent until the keyboard settles prevents that
    /// transient geometry from being mistaken for a user scroll.
    @State private var pinsTailThroughKeyboardResize = false
    /// Invalidates an older keyboard-settle task when another frame arrives.
    @State private var keyboardResizeGeneration = 0
    /// The row the scroll view holds still while heights around it resolve.
    @State private var scrollPosition = ScrollPosition(idType: Int.self)
    /// The end of the content. `Int` like every row id, because
    /// `scrollPosition(id:)` binds one type.
    private static let endOfTranscript = Int.max

    init(
        terminalID: String, workspaceID: String?, connection: Connection, isVisible: Bool = true
    ) {
        self.terminalID = terminalID
        self.workspaceID = workspaceID
        self.connection = connection
        self.isVisible = isVisible
        _stream = StateObject(wrappedValue: AgentStream(terminal: terminalID, core: connection.core))
    }

    private var transcript: Transcript { stream.transcript }

    /// The pending permission, if it is this row that it is asking about.
    private func permission(gating row: TranscriptRow) -> PendingPermission? {
        guard let pending = transcript.pendingPermission else { return nil }
        switch row.kind {
        case .tool:
            return names(pending, row) ? pending : nil
        // A subagent's tool calls live inside its block, not beside it, so a
        // request raised by one names a row this loop never visits. Matching
        // only the top level left that request drawn nowhere — and a request
        // nobody can answer wedges the turn.
        case let .subagent(block):
            return block.children.contains { names(pending, $0) } ? pending : nil
        default:
            return nil
        }
    }

    /// A request naming a tool call the transcript has no row for.
    ///
    /// It should not happen — a permission follows the call it is about — but
    /// an unanswerable request that is also invisible would wedge the agent
    /// with no way for anyone to see why.
    private var unattachedPermission: PendingPermission? {
        guard let pending = transcript.pendingPermission else { return nil }
        let shown = transcript.rows.contains { row in
            if names(pending, row) { return true }
            // Searched to the same depth `permission(gating:)` searches, and it
            // has to be: a block that claims the request while this predicate
            // says it is unattached would ask the same question twice, once
            // inside the block and once in the banner below the transcript.
            guard case let .subagent(block) = row.kind else { return false }
            return block.children.contains { names(pending, $0) }
        }
        return shown ? nil : pending
    }

    /// Whether a turn is running, as the daemon sees it.
    ///
    /// From the fleet rather than inferred here: the daemon derives activity
    /// for every surface that shows it, and a second opinion computed on the
    /// phone is exactly the disagreement this design exists to prevent.
    /// The agent behind this pane, capitalised for the placeholder.
    private var harnessName: String {
        guard let workspaceID,
            let preset = connection.terminal(terminalID, in: workspaceID)?.preset,
            !preset.isEmpty
        else { return "the agent" }
        return preset.capitalized
    }

    private var isWorking: Bool {
        guard let workspaceID else { return false }
        return connection.terminal(terminalID, in: workspaceID)?.agent == .working
    }

    var body: some View {
        VStack(spacing: 0) {
            transcriptBody
                .modifier(
                    AgentLayoutProbe(
                        keyboardHeight: keyboard.height, barHeight: barHeight,
                        followingTail: followingTail))
                // The composer sits in the transcript's bottom safe area, which
                // is the framework's own answer to "a control resting on
                // scrolling content": the conversation runs the full height and
                // scrolls behind it, and the scroll view insets itself so the
                // last line stays reachable. The Mac reached the same place by
                // the same route, after building it by hand first and freezing
                // the app.
                // The composer is part of the KEYBOARD now, not part of this
                // screen — see `DockedBar`. `DockedBar` itself draws nothing
                // here; it is a zero-size representable whose only job is to
                // exist in the hierarchy so its controller can hold first
                // responder and vend the bar.
                .background(
                    DockedBar(height: $barHeight, isActive: isVisible) { composerStack }
                        .frame(width: 0, height: 0)
                        .accessibilityHidden(true)
                )
                // Automatic avoidance off, and the room made by hand.
                //
                // A docked accessory covers the bottom of the screen with the
                // keyboard DOWN, and SwiftUI's avoidance insets by nothing in
                // that state — so the conversation ran underneath the composer.
                // Adding the bar's height on top of avoidance is not the fix
                // either: with the keyboard UP, avoidance already counts the
                // accessory, and adding it again leaves a bar-sized gap. So the
                // one number that is correct in both states — the keyboard's
                // own overlap, accessory included — is used for both.
                .ignoresSafeArea(.keyboard, edges: .bottom)
                .safeAreaInset(edge: .bottom, spacing: 0) {
                    // The larger of the two, because each is right in one state
                    // and blind in the other. With the keyboard DOWN the
                    // accessory is simply on screen and posts no keyboard-frame
                    // notification at all, so only its measured height knows it
                    // is there. With the keyboard UP the reported frame already
                    // includes the accessory and is the taller number, so it
                    // wins — and nothing is counted twice.
                    // Nothing reserved by a pane that is not on screen: its bar
                    // is not docked, so there is nothing down there to clear.
                    Color.clear.frame(height: isVisible ? max(keyboard.height, barHeight) : 0)
                }
                // How the conversation MEETS the glass over it.
                //
                // `safeAreaInset` puts the composer there and tells the scroll
                // view to inset itself; this says what happens at the boundary.
                // Without it the last line simply passes under a hard edge,
                // which is the cut-off-mid-sentence look. `.soft` is the
                // platform's own fade, and the reason the earlier hand-built
                // gradient mask — which pegged a core — was trying to exist.
                .scrollEdgeEffectStyle(.soft, for: .bottom)
        }
        // Polling follows VISIBILITY, not mounting. A hidden pane keeps its
        // transcript and its scroll offset and costs the host nothing.
        .task(id: isVisible) {
            if isVisible { stream.start() } else { stream.stop() }
        }
    }

    /// Everything that sits over the bottom of the conversation.
    ///
    /// Carries its own swipe-down-to-dismiss, because `scrollDismissesKeyboard`
    /// cannot: that only responds to drags on the SCROLL VIEW, and on a phone
    /// the composer and the tab strip below it cover most of the bottom of the
    /// screen — which is exactly where a thumb starts a downward flick. Swiping
    /// the keyboard away worked only if you reached up into the transcript
    /// first, which is not a gesture anybody performs deliberately.
    @ViewBuilder
    private var composerStack: some View {
        // Grouped so the composer and whatever sits above it behave as ONE
        // piece of glass. Each `glassEffect` composites independently
        // otherwise: two surfaces a few points apart both sample the background
        // on their own and neither knows the other is there, so they never
        // blend at the seam the way the platform's own stacked controls do.
        GlassEffectContainer(spacing: PaneMetrics.step) {
            VStack(spacing: PaneMetrics.step) {
                // The plan and the queue are ATTACHED to the composer, not
                // scattered around the screen. The plan used to be pinned at the
                // top and the queue drawn inline at the transcript's end, which put
                // three things that are all "what happens next" in three different
                // places — and the plan, furthest away, was the one the next
                // message is most likely to change.
                if !transcript.plan.isEmpty {
                    PlanPanel(entries: transcript.plan)
                }

                ForEach(transcript.queue) { queued in
                    QueuedRow(
                        queued: queued,
                        onEdit: { text in Task { await stream.editQueued(queued.id, text) } },
                        onCancel: { Task { await stream.cancelQueued(queued.id) } },
                        onSteer: { Task { await stream.steerQueued(queued.id) } })
                }

                if let pending = unattachedPermission {
                    ApprovalCard(pending: pending) { optionID in
                        Task { await stream.answer(pending.id, optionID) }
                    }
                }

                // A message that did not go, said so beside the composer.
                //
                // Here rather than in the transcript, and beside the composer
                // rather than at the top: the undelivered message is already
                // drawn in the transcript looking sent, so the correction
                // belongs where the eye is — on the thing that would send it
                // again.
                //
                // RED, not amber. A message that did not send is a failure,
                // and amber in this app means one thing — an agent is waiting
                // on you — which is the opposite of what this row reports.
                if let failure = stream.sendFailure {
                    HStack(spacing: PaneMetrics.step) {
                        Image(systemName: "exclamationmark.triangle.fill")
                            .foregroundStyle(.red)
                        Text(failure.message)
                            .font(.footnote)
                        Spacer(minLength: PaneMetrics.step)
                        // The row's height is its buttons now, so the padding
                        // below is only the gap to the glass edge.
                        Button("Retry") { Task { await failure.retry() } }
                            .font(.footnote.weight(.semibold))
                            .frame(minHeight: PaneMetrics.target)
                            .contentShape(.rect)
                        Button {
                            stream.sendFailure = nil
                        } label: {
                            Image(systemName: "xmark")
                                .frame(
                                    width: PaneMetrics.target, height: PaneMetrics.target)
                                .contentShape(.rect)
                        }
                        .accessibilityLabel("Dismiss")
                    }
                    .padding(.horizontal, PaneMetrics.edge)
                    .padding(.vertical, PaneMetrics.tight)
                    .modifier(GlassSurface())
                }

                    AgentComposer(
                    availableModes: transcript.availableModes,
                    agentMode: transcript.agentMode,
                    configOptions: transcript.configOptions,
                    onSetConfig: { id, value in Task { await stream.setConfig(id, value) } },
                    harness: harnessName,
                    availableCommands: transcript.availableCommands,
                    workspaceID: workspaceID,
                    core: connection.core,
                    onSend: { text, images in
                        Task { await stream.send(text, images: images) }
                    },
                    onSetMode: { mode in Task { await stream.setMode(mode) } }
                )
            }
            // ONE left edge, said once.
            //
            // Every surface in this stack used to carry its own horizontal
            // padding — 12 on the plan, the queue, the card and the banner,
            // 10 on the composer — so the container whose whole purpose is
            // that these "behave as ONE piece of glass" drew them with their
            // left edges two points apart. Hoisted here, there is one number
            // and it is the one the tab strip below them uses.
            .padding(.horizontal, PaneMetrics.surfaceInset)
        }
    }

    @ViewBuilder
    private var transcriptBody: some View {
        if transcript.rows.isEmpty {
            // The error goes with the empty state, not inside a transcript
            // that does not exist.
            //
            // The banner used to live in the scroll view, which only exists
            // once there are rows — so a session that never loaded at all
            // showed "Say something to begin" with the reason it was empty
            // hidden behind the very condition that made it empty. The one
            // moment the message is worth reading is the one moment it could
            // not be seen.
            //
            // And one copy of it, not two. A second banner sat here, above
            // `emptyState`, which has an error arm of its own — a triangle, a
            // headline and the same string — so with nothing loaded the reason
            // was drawn twice on one screen, once as a tinted caption and
            // again a few points lower under a heading. The arm that draws it
            // with a headline is the one that reads as an explanation, so it is
            // the one that stayed.
            emptyState
                .frame(maxWidth: .infinity, maxHeight: .infinity)
        } else {
            ScrollView {
                // Lazy, and ANCHORED — the pair that makes it work.
                //
                // A lazy stack estimates the height of rows it has not
                // built and corrects as they scroll in, which made a
                // transcript of wildly uneven rows jump under the finger.
                // Building every row eagerly fixes that and is the wrong
                // trade at thousands of rows, each laying out markdown.
                // `scrollPosition` below is the missing half: it names the
                // row to hold still while the estimates around it resolve.
                LazyVStack(alignment: .leading, spacing: PaneMetrics.card) {
                    // A stale error banner rather than a blanked screen: a
                    // failed poll is not a disconnection, the same rule
                    // `Connection.refresh()` follows — the last known
                    // transcript stays up while this device tries again.
                    //
                    // Red rather than amber, for the reason the send failure
                    // beside the composer is: a poll that did not come back is
                    // a failure, and amber is spoken for.
                    if let trouble = stream.connectionError {
                        Text(trouble.sentence)
                            .font(.caption)
                            .foregroundStyle(.red)
                        // Rare by construction, and that is what makes it
                        // bearable here: the failure that actually happens on a
                        // phone is a dropped link, which carries a written
                        // sentence and no transcript. A box appears only for
                        // the failures nothing on this side can account for,
                        // and it sits at the head of a transcript that is
                        // already scrolled to its tail.
                        if let words = trouble.transcript, !words.isEmpty {
                            DetailBox(text: words)
                        }
                    }
                    ForEach(transcript.rows) { row in
                        AgentRowView(
                            row: row,
                            isLast: row.id == transcript.rows.last?.id,
                            pending: permission(gating: row),
                            onAnswer: { optionID in
                                guard let id = transcript.pendingPermission?.id else { return }
                                Task { await stream.answer(id, optionID) }
                            })
                            // Pinned to the row's own identity, which `ForEach`
                            // would otherwise infer from position in a lazy
                            // stack that recycles its views. A recycled view
                            // keeps its `@State`, so a subagent block a reader
                            // had opened by hand could hand that decision to a
                            // different block scrolling into its place.
                            .id(row.id)
                    }

                    // The turn that is still running, one line ahead of
                    // what it has produced.
                    if isWorking {
                        WorkingRow()
                    }

                    // An invisible anchor rather than scrolling to the
                    // last row's own id: the last row mutates in place
                    // while a tool streams progress (see `Transcript`),
                    // so its id does not change and `scrollTo` would have
                    // nothing new to react to.
                    Color.clear.frame(height: 1).id(Self.endOfTranscript)
                }
                .padding(PaneMetrics.card)
            }
            // What the scroll view holds still while content around it
            // changes height — see the stack above.
            // Start at the conversation's tail even when the transcript is too
            // long for the lazy stack to finish laying out before `onAppear`.
            // The imperative scroll below remains useful when returning to a
            // mounted pane; this is the reliable first-layout anchor.
            .defaultScrollAnchor(.bottom)
            // Drag the transcript down and the keyboard goes with it.
            //
            // `.interactively`: the keyboard tracks the finger and comes back if
            // the drag is reversed, which is what the platform's own messaging
            // surfaces do and what a thumb already expects.
            //
            // It starts moving only once the touch reaches the keyboard's own
            // frame — that is `UIScrollView.keyboardDismissMode = .interactive`,
            // which is defined in terms of the keyboard's rect and knows nothing
            // about the composer and tab strip stacked above it. So the drag has
            // to cross both bars before anything happens. Making the keyboard
            // respond sooner means making those bars part of it — an
            // `inputAccessoryView` — which is a trade this app has already
            // refused once for the tab strip: an accessory floats OVER content,
            // so the grid was laid out as though the strip were not there and
            // its last line ended up behind it.
            .scrollDismissesKeyboard(.interactively)
            .scrollPosition($scrollPosition, anchor: .bottom)
            .onScrollGeometryChange(for: Bool.self) { geometry in
                // `visibleRect` already accounts for every safe-area inset.
                // Adding `containerSize` to `contentOffset` does not, which
                // makes a bottom-inset scroll view look permanently short of
                // its tail even after the user has reached it.
                // 40pt of slack, because "at the bottom" after a redraw is
                // rarely exact while a lazy row is resolving its height.
                geometry.visibleRect.maxY >= geometry.contentSize.height - 40
            } action: { _, atBottom in
                // A keyboard resize briefly reports "not at bottom" before
                // the new content inset and scroll position meet. That is
                // layout churn, not the reader scrolling away.
                if pinsTailThroughKeyboardResize {
                    followingTail = true
                } else {
                    followingTail = atBottom
                }
            }
            // Keyed on the CURSOR, not the row count. A streamed reply
            // coalesces into the row already on screen, so the count does
            // not change while the text grows off the bottom.
            .onChange(of: transcript.cursor) { _, _ in
                // Only while the reader is at the tail. Scrolling to the
                // end on every event meant reading anything older was
                // impossible — a streamed reply fires several events a
                // second and each one threw the view back to the bottom.
                guard followingTail else { return }
                withAnimation(.easeOut(duration: 0.2)) {
                    scrollPosition.scrollTo(id: Self.endOfTranscript, anchor: .bottom)
                }
            }
            .onChange(of: keyboard.height) { oldHeight, newHeight in
                let oldInset = max(oldHeight, barHeight)
                let newInset = max(newHeight, barHeight)
                guard newInset > oldInset,
                    followingTail || pinsTailThroughKeyboardResize
                else { return }

                pinsTailThroughKeyboardResize = true
                keyboardResizeGeneration += 1
                let generation = keyboardResizeGeneration

                // Preserve the reader's intent, not the old coordinates. The
                // keyboard changes its frame over several layout passes. Pin
                // throughout that animation, then release only after one final
                // re-anchor at the settled size. A single next-run-loop scroll
                // is too early on iOS 26 and leaves the final rows underneath
                // the keyboard.
                Task { @MainActor in
                    for delay in [0, 80, 160, 260, 420] {
                        if delay == 0 {
                            await Task.yield()
                        } else {
                            try? await Task.sleep(for: .milliseconds(delay))
                        }
                        guard generation == keyboardResizeGeneration else { return }
                        scrollPosition.scrollTo(id: Self.endOfTranscript, anchor: .bottom)
                    }
                    // Let the final scroll geometry publish while the pin is
                    // still active, so it cannot undo the preserved intent.
                    await Task.yield()
                    guard generation == keyboardResizeGeneration else { return }
                    followingTail = true
                    pinsTailThroughKeyboardResize = false
                }
            }
            .onAppear { scrollPosition.scrollTo(id: Self.endOfTranscript, anchor: .bottom) }
        }
    }

    /// A full-screen state, sized like one.
    ///
    /// The proportions are `FleetView.failure`'s, which `TerminalView.status`
    /// also adopted: a 42-point thin mark, a `.title2` headline, a `.callout`
    /// sentence, 22 and 8 between them. This was `.largeTitle` over
    /// `.headline`, and `.headline` is 17 points — a list row's title standing
    /// in for a page's, on a screen with nothing else on it.
    ///
    /// The mark is red rather than amber. A session that would not load is a
    /// failure; amber in this app means an agent is waiting on you, and a
    /// screen that cannot show you an agent at all is not that.
    private var emptyState: some View {
        VStack(spacing: 0) {
            Spacer()
            if let trouble = stream.connectionError {
                Image(systemName: "exclamationmark.triangle")
                    .font(.system(size: 42, weight: .thin))
                    .foregroundStyle(.red)
                    .padding(.bottom, 22)
                Text("Could not load this session")
                    .font(.title2.weight(.semibold))
                    .multilineTextAlignment(.center)
                    .padding(.bottom, PaneMetrics.step)
                Text(trouble.sentence)
                    .font(.callout)
                    .foregroundStyle(.secondary)
                    .multilineTextAlignment(.center)
                    .frame(maxWidth: 320)
                // The core's own words, below a sentence rather than standing
                // in for one. Under this headline, in this face, they used to
                // read as Far Cooler's account of the runner; they are not, and
                // they are also the only account anybody debugging an
                // unreachable runner is going to get, so they stay.
                if let words = trouble.transcript, !words.isEmpty {
                    DetailBox(text: words)
                        .frame(maxWidth: 320)
                        .padding(.top, 14)
                }
            } else {
                Text("Say something to begin.")
                    .font(.callout)
                    .foregroundStyle(.secondary)
            }
            Spacer()
        }
        .padding(.horizontal, 32)
        .frame(maxWidth: .infinity)
    }
}

/// Debug-only layout telemetry for the real-device keyboard regression.
/// Release builds keep the stable identifier but do not expose implementation
/// measurements as a VoiceOver value.
private struct AgentLayoutProbe: ViewModifier {
    let keyboardHeight: CGFloat
    let barHeight: CGFloat
    let followingTail: Bool

    func body(content: Content) -> some View {
        #if DEBUG
        content
            .accessibilityIdentifier("agent-transcript")
            .accessibilityValue(
                Text(verbatim:
                    "keyboard=\(Int(keyboardHeight.rounded()));bar=\(Int(barHeight.rounded()));tail=\(followingTail ? "true" : "false")"))
        #else
        content.accessibilityIdentifier("agent-transcript")
        #endif
    }
}

// MARK: - Rows

/// Whether a row IS the tool call a request is asking about.
///
/// Free rather than a method, because the same question is asked at two depths
/// — of a top-level row and of a block's child — and the two answers drifting
/// apart is how a request ends up claimed by nobody or by two views at once.
private func names(_ pending: PendingPermission, _ row: TranscriptRow) -> Bool {
    if case let .tool(tool) = row.kind { return tool.id == pending.toolCall }
    return false
}

/// One row of a rendered agent transcript.
///
/// A thin switch, deliberately: `Transcript` already decided what happened —
/// coalesced message chunks, mutated a tool call in place rather than
/// appending, kept a gap as its own row — this only decides how each of the
/// three shapes it can hand back gets drawn.
private struct AgentRowView: View {
    let row: TranscriptRow
    /// The request this row is blocked on, if it is the one being asked about.
    ///
    /// A permission names the tool call it gates, so it is shown ON that call
    /// rather than in a panel elsewhere: what you are approving and what it
    /// will run become the same object, with nothing to match up by eye.
    /// Whether this is the newest row. A thought is still being written exactly
    /// while nothing has followed it — the transcript already knows the order,
    /// so asking "is anything after me" is the same question.
    var isLast: Bool = false
    var pending: PendingPermission?
    var onAnswer: ((String) -> Void)?

    var body: some View {
        switch row.kind {
        // The parent pointer is deliberately ignored here. It exists so the
        // reducer can refuse to coalesce an orphan into the agent's own words;
        // once a message has been placed, where it came from changes nothing
        // about how it is drawn.
        case let .message(role, text, _):
            MessageRow(role: role, text: text, isLive: isLast)
        case let .tool(tool):
            ToolRowView(tool: tool, isLive: isLast, pending: pending, onAnswer: onAnswer)
        case let .subagent(block):
            SubagentBlockView(block: block, pending: pending, onAnswer: onAnswer)
        case let .gap(reason):
            GapRow(reason: reason)
        }
    }
}

/// One message. Three shapes for three roles, because they answer three
/// different questions: what did I say, what did it say, and what did it
/// think before saying that.
private struct MessageRow: View {
    let role: Role
    let text: String
    /// Whether this is the newest row, which is what makes a thought "live".
    var isLive: Bool = false

    var body: some View {
        switch role {
        case .user:
            // Right-aligned with a fill — the one voice in the transcript
            // that is not the agent talking, and it has to read as a
            // different speaker at a glance, not on close reading.
            //
            // The comment was right and the fill had drifted past it. This was
            // `Color.primary.opacity(0.07)` and so was every tool card in the
            // file, so the one thing that had to be a different speaker was
            // the same grey as a container around some output. It is a full
            // step above them now — see `TranscriptFill`.
            //
            // `.body`, which is the size the agent's own words are: the two
            // were one step apart, `.callout` here against `AgentReplyText`'s
            // `.body`, and the only thing that difference said was that what
            // you wrote matters slightly less than the answer to it.
            HStack {
                Spacer(minLength: 40)
                Text(text)
                    .font(.body)
                    .textSelection(.enabled)
                    .padding(.horizontal, PaneMetrics.card)
                    .padding(.vertical, PaneMetrics.step)
                    .background(TranscriptFill.speaker, in: RoundedRectangle(cornerRadius: 14))
            }

        case .agent:
            // Full width, through the SHARED renderer — the same one the Mac
            // uses. Plain `Text` here meant a table arrived as a wall of pipes
            // and a heading as a line starting with a hash: the same
            // conversation, unreadable on the phone.
            AgentReplyText(text: text, trailingClearance: 40)

        case .thought:
            // Open while it is being written, closed once it is done.
            //
            // It was collapsed always, on the reasoning that a finished thought
            // is scratch work — true, and it left a phone watching a long turn
            // with one word on screen and no sign of movement, which reads as
            // stuck. The Mac solved this and the phone never got it.
            ThoughtRow(text: text, isLive: isLive)
        }
    }
}

/// One tool call, mutated in place by every update — never a new row — so a
/// call that reports progress four times still occupies the one line it
/// earned.
private struct ToolRowView: View {
    let tool: ToolRow
    /// Whether this is the newest row — nothing has followed it yet.
    var isLive: Bool = false
    var pending: PendingPermission?
    var onAnswer: ((String) -> Void)?

    private var expandable: Bool { tool.content != nil || tool.diff != nil }

    @State private var expanded = false

    var body: some View {
        // The Mac's grey box, not a `DisclosureGroup`.
        //
        // `DisclosureGroup` tints its label with the accent color, so every
        // tool call rendered as blue link text — a command looked like
        // something to navigate to rather than something that ran. It also put
        // the chevron OUTSIDE any container, which left an expanded detail
        // aligned to the wrong edge. The Mac solved both by making the row and
        // what it opens one object on one fill; this is that, on a phone.
        VStack(alignment: .leading, spacing: 0) {
            if expandable {
                Button {
                    withAnimation(.spring(response: 0.22, dampingFraction: 0.82)) {
                        expanded.toggle()
                    }
                } label: {
                    label.contentShape(Rectangle())
                }
                .buttonStyle(.plain)
            } else {
                label
            }

            if showingDetail {
                Divider()
                VStack(alignment: .leading, spacing: PaneMetrics.step) {
                    if let content = tool.content, !content.isEmpty {
                        // Bounded, and for the same reason it is bounded on the
                        // Mac: `Text` measures its whole string on every layout
                        // pass, and a tool that returns thousands of lines
                        // inside an animated disclosure froze the app there.
                        DetailBox(text: content, chrome: false)
                    }
                    if let diff = tool.diff {
                        DiffView(diff: diff)
                    }
                }
                .padding(PaneMetrics.card)
                .frame(maxWidth: .infinity, alignment: .leading)
            }

            // The question, on the thing being asked about.
            //
            // No heading and no orange panel: the ring around this row already
            // says which call is waiting, and repeating it in words inside the
            // row it is drawn on is the same fact twice.
            if let pending, let onAnswer {
                Divider()
                ApprovalControls(options: pending.options, onChoose: onAnswer)
                    .padding(PaneMetrics.card)
            }
        }
        .background(TranscriptFill.container, in: RoundedRectangle(cornerRadius: 8))
        .overlay {
            if pending != nil {
                RoundedRectangle(cornerRadius: 8).strokeBorder(TranscriptFill.attentionRing)
            }
        }
        // Driven by the model rather than a timer, exactly as the thought row
        // is: the fold follows the turn moving on.
        .animation(.spring(response: 0.22, dampingFraction: 0.82), value: isLive)
        .animation(.spring(response: 0.22, dampingFraction: 0.82), value: running)
    }

    /// Open while it is waiting to be approved, and while it is the thing
    /// currently happening.
    ///
    /// Being asked to allow a command without being shown it is a guess, not a
    /// decision. And a command running with nothing after it IS the turn — the
    /// same rule `ThoughtRow` follows — so it opens itself and folds away once
    /// the agent moves on, because a transcript of every command's full output
    /// is unreadable.
    private var showingDetail: Bool { expanded || pending != nil || (isLive && running) }

    /// Still going, as the agent last reported it.
    private var running: Bool { tool.status == .pending || tool.status == .inProgress }

    /// The row, and — where there is anything to open — the thing you tap.
    ///
    /// 44 points tall whether it opens or not. The disclosure was 34, which is
    /// the guideline's floor missed by ten points on the control this
    /// transcript asks you to hit most often; and a tool row with detail
    /// standing ten points taller than one without would have made the
    /// transcript's rhythm depend on whether a command happened to return
    /// anything. The fill is the row, so the height goes on the label rather
    /// than on a hit shape spilling into the row above.
    private var label: some View {
        HStack(spacing: PaneMetrics.step) {
            if expandable {
                Image(systemName: "chevron.right")
                    .font(.caption2)
                    .foregroundStyle(.secondary)
                    .rotationEffect(.degrees(expanded ? 90 : 0))
            }
            // Not `StatusGlyph` — that type lives in the Mac target. Same
            // vocabulary, one dot at one weight for "something is happening",
            // just drawn locally: green finished, red missing, secondary
            // for everything in between. Orange is reserved for "needs you",
            // which a tool call never is.
            Circle()
                .fill(toolStatusColor(tool.status))
                .frame(width: 7, height: 7)
            Text(tool.title)
                .font(.subheadline.weight(.medium))
            if let location = tool.locations.first {
                Text(location)
                    .font(.caption.monospaced())
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }
            Spacer(minLength: PaneMetrics.tight)
        }
        .foregroundStyle(.primary)
        .padding(.horizontal, PaneMetrics.card)
        .frame(maxWidth: .infinity, minHeight: PaneMetrics.target, alignment: .leading)
    }
}

private func toolStatusColor(_ status: ToolStatus) -> Color {
    switch status {
    case .pending: return .secondary.opacity(0.35)
    case .inProgress: return .secondary
    case .completed: return .green
    case .failed: return .red
    // A status from a newer daemon. Neutral on purpose: it finished, and
    // claiming either success or failure would be inventing a detail.
    case .unknown: return .secondary
    }
}

/// A subagent's dispatch, and everything it did, as one object.
///
/// The Mac's block on a phone, and the same grey box `ToolRowView` draws for the
/// same reason: the row and what it opens are one fill, so an expanded block
/// cannot drift to a different edge than the header that opened it. Its
/// children are ordinary `AgentRowView`s — a subagent's messages and tools are
/// the same things the top level shows, and nesting is where they live rather
/// than what they are.
private struct SubagentBlockView: View {
    let block: SubagentBlock
    /// The request this block is answering for, if one of its children raised
    /// it. Passed down rather than looked up here, so the transcript decides
    /// once which row owns a request.
    var pending: PendingPermission?
    var onAnswer: ((String) -> Void)?

    /// `nil` means nobody has said, so the automatic rule below applies. Once a
    /// reader touches it they win permanently: a block that shut itself while
    /// someone was reading it is worse than one left open.
    @State private var toggled: Bool?
    @State private var showingAll = false

    /// How many children an expanded block shows. Enough to see what it is
    /// doing; few enough that a subagent that ran three hundred steps still
    /// leaves room for the conversation around it.
    private static let visibleChildren = 3

    /// This file's spring, spelled out — `Motion` is a Mac-target type and the
    /// phone has never had it.
    private static let motion = Animation.spring(response: 0.22, dampingFraction: 0.82)

    /// Still working, as far as anyone knows.
    ///
    /// Derived on the model, not here: interruption leaves the tool's status
    /// alone, so asking it directly keeps a cut-off block spinning forever.
    private var running: Bool { block.isRunning }

    /// Open while it works, closed once it reports — unless a reader has said
    /// otherwise, or unless something inside it is waiting to be approved.
    /// Being asked to allow a command without being shown it is a guess rather
    /// than a decision, the same rule `ToolRowView.showingDetail` follows.
    private var showing: Bool { pending != nil || (toggled ?? running) }

    /// The last few children rather than the first few: what a subagent is
    /// doing now is what a reader is watching for, and the cap is what bounds a
    /// block's height whether it holds three rows or three hundred.
    private var shown: [TranscriptRow] {
        // A capped view could hide the very row a request is asking about,
        // which is the one thing this block must never do while it holds an
        // unanswered question.
        guard pending == nil, !showingAll, block.children.count > Self.visibleChildren else {
            return block.children
        }
        return Array(block.children.suffix(Self.visibleChildren))
    }

    private var hidden: Int { block.children.count - shown.count }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Button {
                withAnimation(Self.motion) { toggled = !showing }
            } label: {
                header.contentShape(Rectangle())
            }
            .buttonStyle(.plain)

            if showing && !block.children.isEmpty {
                Divider()
                VStack(alignment: .leading, spacing: PaneMetrics.step) {
                    if hidden > 0 {
                        Button("… \(hidden) more") { withAnimation(Self.motion) { showingAll = true } }
                            .buttonStyle(.plain)
                            .font(.caption)
                            .foregroundStyle(.secondary)
                            .frame(
                                maxWidth: .infinity, minHeight: PaneMetrics.target,
                                alignment: .leading)
                            .contentShape(.rect)
                    }
                    ForEach(shown) { child in
                        // The approval controls are drawn by the child that is
                        // actually blocked, not by this block, so what is being
                        // approved and what will run stay the same object.
                        AgentRowView(
                            row: child,
                            pending: gating(child),
                            onAnswer: onAnswer)
                    }
                }
                .padding(PaneMetrics.card)
                .frame(maxWidth: .infinity, alignment: .leading)
            }
        }
        // Working blocks sit at the tool card's tier and finished ones a step
        // below it, which is what 0.07 and 0.035 were reaching for.
        .background(
            running || pending != nil ? TranscriptFill.container : TranscriptFill.recessed,
            in: RoundedRectangle(cornerRadius: 8))
        .animation(Self.motion, value: showing)
        // Children arrive one at a time while the subagent works, and a block
        // that grew by a row per frame with no animation flickered its way down
        // the transcript.
        .animation(Self.motion, value: block.children.count)
    }

    /// 44 points tall, like `ToolRowView`'s: this is the only way into a
    /// block, and it was 32.
    private var header: some View {
        HStack(spacing: PaneMetrics.step) {
            Image(systemName: "chevron.right")
                .font(.caption2)
                .foregroundStyle(.secondary)
                .rotationEffect(.degrees(showing ? 90 : 0))
            Circle()
                .fill(dotColor)
                .frame(width: 7, height: 7)
            Text(block.tool.title)
                .font(.subheadline.weight(running || pending != nil ? .semibold : .medium))
                .foregroundStyle(running || pending != nil ? .primary : .secondary)
                .lineLimit(1)
                // Truncated in the middle, because a dispatch's title is the
                // prompt it was given and the end of that sentence says more
                // about what it went off to do than the middle does.
                .truncationMode(.middle)
            Text(block.subtitle)
                .font(.caption)
                .foregroundStyle(.secondary)
                .lineLimit(1)
            Spacer(minLength: PaneMetrics.tight)
        }
        .foregroundStyle(.primary)
        .padding(.horizontal, PaneMetrics.card)
        .frame(maxWidth: .infinity, minHeight: PaneMetrics.target, alignment: .leading)
    }

    /// What a collapsed block still answers without being opened.

    /// Red for interrupted, overriding the tool's own status on purpose: a
    /// block cut off mid-flight is still `inProgress` on the wire, and a
    /// subagent whose outcome nobody knows must never wear the mark of one that
    /// came back.
    private var dotColor: Color {
        block.interrupted ? .red : toolStatusColor(block.tool.status)
    }

    /// The request, on the child it names.
    private func gating(_ child: TranscriptRow) -> PendingPermission? {
        guard let pending, names(pending, child) else { return nil }
        return pending
    }
}

/// A break in the transcript, named rather than hidden.
///
/// The one row here that is not allowed to be quiet. A gap is the opposite of
/// nothing — it is the transcript admitting history is missing — and drawing
/// it as a thin rule between two messages would let a reader miss the one
/// fact this whole design exists to never hide.
private struct GapRow: View {
    let reason: GapReason

    var body: some View {
        // The sentence, then — where there is one — the adapter's account of
        // the refusal beneath it. `reason.sentence` and `reason.detail` are
        // AgentKit's, shared with the Mac's row: the two used to hold the same
        // `switch` twice, byte for byte, and one gap read on two devices is
        // the only way that drift ever shows itself. The metrics below stay
        // this app's own, which is the part that should differ.
        VStack(alignment: .leading, spacing: PaneMetrics.step) {
            HStack(spacing: PaneMetrics.step) {
                Image(systemName: reason.isInformational ? "info.circle" : "scissors")
                    .font(.caption)
                Text(reason.sentence)
                    .font(.footnote.weight(.medium))
                Spacer(minLength: 0)
            }
            // Tinted here rather than on the whole row, so the box below keeps
            // its own voice: that text is the adapter's, and coloring it like
            // this row's sentence is this app appearing to have said it.
            //
            // RED, not amber. A gap is history the transcript could not show
            // you — a refusal, or a turn cut off — which is a failure and not
            // "an agent wants you", the one thing amber is kept for here.
            //
            // `Color.secondary`, not `.secondary` — that shorthand resolves to
            // `HierarchicalShapeStyle`, a different type from `Color.red`, and
            // a ternary needs both branches to agree.
            .foregroundStyle(reason.isInformational ? Color.secondary : Color.red)

            if let detail = reason.detail, !detail.isEmpty {
                // No fill of its own: it is already inside one, the same call
                // `ToolRow` makes further up this file.
                DetailBox(text: detail, chrome: false)
            }
        }
        .padding(.horizontal, PaneMetrics.card)
        .padding(.vertical, PaneMetrics.step)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(
            reason.isInformational ? TranscriptFill.container : TranscriptFill.alarm,
            in: RoundedRectangle(cornerRadius: 8))
    }
}

// MARK: - Plan

/// The agent's task list, as the agent maintains it.
///
/// The same design as the Mac's, for the same reason the reducer is shared: a
/// list of bullets is the same information and none of the use — what a reader
/// wants is how far through it is and what is happening right now.
private struct PlanPanel: View {
    let entries: [PlanEntry]

    @State private var expanded = true

    var body: some View {
        VStack(alignment: .leading, spacing: PaneMetrics.step) {
            Button {
                withAnimation(.spring(response: 0.22, dampingFraction: 0.82)) {
                    expanded.toggle()
                }
            } label: {
                HStack(spacing: PaneMetrics.step) {
                    Image(systemName: "chevron.right")
                        .font(.caption2)
                        .rotationEffect(.degrees(expanded ? 90 : 0))
                    Text("Tasks").font(.caption.weight(.semibold))
                    Text("\(entries.doneCount) of \(entries.count)")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                    if !expanded, let active = entries.active {
                        Text("· \(active.content)")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                            .lineLimit(1)
                    }
                    Spacer(minLength: 0)
                }
                // The whole width of the panel, 44 points tall: the header is
                // the only way to fold this away, and it was a caption.
                .frame(maxWidth: .infinity, minHeight: PaneMetrics.target)
                .contentShape(.rect)
            }
            .buttonStyle(.plain)

            if expanded {
                ForEach(Array(entries.enumerated()), id: \.offset) { _, entry in
                    HStack(alignment: .top, spacing: PaneMetrics.step) {
                        Image(systemName: PlanStatus(entry.status).symbol)
                            .font(.caption)
                            .foregroundStyle(PlanStatus(entry.status).tint)
                            .frame(width: 14)
                        Text(entry.content)
                            .font(.footnote)
                            .strikethrough(PlanStatus(entry.status).isDone)
                            .foregroundStyle(PlanStatus(entry.status).isDone ? .secondary : .primary)
                            .fixedSize(horizontal: false, vertical: true)
                        Spacer(minLength: 0)
                    }
                }
            }
        }
        // OPAQUE, because it floats over a scrolling transcript. A tinted
        // overlay let the conversation through, and expanding the list turned
        // both into one unreadable overlap.
        //
        // Radius 22 and no inset of its own: it is one of the surfaces in
        // `composerStack`, and that stack now says the edge once.
        .padding(.horizontal, PaneMetrics.edge)
        .padding(.vertical, PaneMetrics.card)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(
            .regularMaterial,
            in: RoundedRectangle(cornerRadius: PaneMetrics.surfaceRadius))
    }




}

/// A message written but not yet sent. See the Mac's `QueuedRow`.
private struct QueuedRow: View {
    let queued: QueuedPrompt
    let onEdit: (String) -> Void
    let onCancel: () -> Void
    /// Send this one into the turn already running.
    ///
    /// The queue's whole point is that a message you can still see and still
    /// edit beats one already gone — so waiting is the default. But a message
    /// written mid-turn is very often a correction, and a correction is worth
    /// nothing once the wrong thing has been done. This is the escape hatch:
    /// you looked at what you wrote and decided it should interrupt.
    let onSteer: () -> Void

    @State private var editing = false
    @State private var draft = ""

    var body: some View {
        HStack(alignment: .top, spacing: PaneMetrics.step) {
            Spacer(minLength: 40)
            VStack(alignment: .trailing, spacing: PaneMetrics.tight) {
                // `.body` in all three arms. A queued message is the reader's
                // own words waiting to be sent, so it is set at the size the
                // transcript sets them — and this row used to hold `.callout`
                // and `.body` in the SAME row, one step apart, depending on
                // whether what you queued had any words in it.
                if editing {
                    TextField("", text: $draft, axis: .vertical)
                        .textFieldStyle(.plain)
                        .font(.body)
                        .frame(minWidth: 140, minHeight: PaneMetrics.target)
                        .onSubmit(commit)
                } else if queued.text.isEmpty && queued.imageCount > 0 {
                    // An image with no words is still a message. Without this
                    // the bubble was empty and read as a dropped attachment.
                    Label(
                        queued.imageCount == 1 ? "1 image" : "\(queued.imageCount) images",
                        systemImage: "photo")
                        .font(.body)
                } else {
                    Text(queued.text).font(.body)
                }

                // A label and three actions, told apart.
                //
                // All four used to be `.caption` in `.secondary` with the
                // buttons set `.plain`, so "Queued", "Send now", "Edit" and
                // "Remove" were one line of identical grey words — three of
                // which do something, with nothing saying which three.
                HStack(spacing: PaneMetrics.card) {
                    Text("Queued")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                    Button("Send now", action: onSteer)
                    Button(editing ? "Save" : "Edit") {
                        if editing {
                            commit()
                        } else {
                            draft = queued.text
                            editing = true
                        }
                    }
                    Button("Remove", action: onCancel)
                }
                .buttonStyle(QueuedActionStyle())
            }
            .padding(.horizontal, PaneMetrics.edge)
            .padding(.vertical, PaneMetrics.card)
            // Radius 22, the corner every surface in `composerStack` draws.
            // The dashed edge is what says "not sent yet"; the rounding was
            // never carrying that and only made this bubble a different object
            // from the composer it is attached to.
            .background {
                RoundedRectangle(cornerRadius: PaneMetrics.surfaceRadius)
                    .fill(.regularMaterial)
                    .overlay {
                        RoundedRectangle(cornerRadius: PaneMetrics.surfaceRadius)
                            .strokeBorder(
                                Color.secondary.opacity(0.4),
                                style: StrokeStyle(lineWidth: 1, dash: [4, 3]))
                    }
            }
        }
    }

    private func commit() {
        let trimmed = draft.trimmingCharacters(in: .whitespacesAndNewlines)
        editing = false
        guard !trimmed.isEmpty, trimmed != queued.text else { return }
        onEdit(trimmed)
    }
}

/// One of the queue's actions: a word you can tap, drawn as one.
///
/// Tinted and semibold, which is what this file already uses for an action
/// standing in prose — the composer's "Retry" — rather than a fifth grey word
/// in a row of grey words. The 44-point band is the hit target a 16-point
/// caption never had, and the `contentShape` is what makes it live: padding
/// around a `Button`'s label is layout only.
private struct QueuedActionStyle: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(.caption.weight(.semibold))
            .foregroundStyle(.tint)
            // The press, said by the label rather than by a fill: there is no
            // fill here to press, and a word that does nothing under a thumb
            // is a word nobody is sure they hit.
            .opacity(configuration.isPressed ? 0.55 : 1)
            .frame(minHeight: PaneMetrics.target)
            .contentShape(.rect)
    }
}

// MARK: - Approval

/// A permission request, blocking the turn until answered — the one place
/// this surface asks something of you rather than reporting something to
/// you. Buttons are full-width and tall on purpose: this is the card a thumb
/// has to hit correctly the first time, on a phone, possibly one-handed.
///
/// The answers to a permission request.
///
/// Ordered by what the question actually is. ACP hands back a flat list —
/// `allow_once`, `allow_always`, `reject_once`, … — and rendering it flat gave
/// three identical full-width buttons, two of them the same blue, with the
/// longest and loudest being a restatement of the command already shown above.
/// A stack of equal-weight options is not a decision; it is a menu.
///
/// So: the decision is one row, Allow prominent and Reject plain beside it, and
/// everything else — the "always" variants, which are a policy change rather
/// than an answer to this question — sits under it in small type.
///
/// Reject is NOT red. Red is for destructive; declining a command destroys
/// nothing, and spending the alarm color here leaves none for when it matters.
///
/// **The sentence above about full-width and tall was right, and the code
/// under it had drifted.** The decision row said `.controlSize(.small)` and
/// ended in a `Spacer`, so what shipped was two roughly 28-point pills bunched
/// against the left edge of the card — neither full-width nor tall, on the one
/// control in this app where hitting Reject instead of Allow means a command
/// ran that nobody allowed. `.large` now, each half of the row expands to
/// share the width, and 44 is the floor under both.
struct ApprovalControls: View {
    let options: [PermissionOption]
    let onChoose: (String) -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: PaneMetrics.step) {
            HStack(spacing: PaneMetrics.step) {
                if let allow {
                    Button(allow.name) { onChoose(allow.id) }
                        .buttonStyle(.borderedProminent)
                        .frame(maxWidth: .infinity, minHeight: PaneMetrics.target)
                }
                if let reject {
                    Button(reject.name) { onChoose(reject.id) }
                        .buttonStyle(.bordered)
                        .frame(maxWidth: .infinity, minHeight: PaneMetrics.target)
                }
            }
            .controlSize(.large)

            // Small type, full-size targets.
            //
            // These stay quiet deliberately — a policy change is not the
            // answer to the question being asked — but quiet is about weight,
            // not about whether a thumb can land on them. They were 16-point
            // rows of caption text. The band is 44 and the `contentShape` is
            // what makes it live rather than merely occupied, so the spacing
            // between two "always" options belongs to one of them.
            VStack(alignment: .leading, spacing: 0) {
                ForEach(secondary) { option in
                    Button(option.name) { onChoose(option.id) }
                        .buttonStyle(.plain)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                        .truncationMode(.middle)
                        .frame(
                            maxWidth: .infinity, minHeight: PaneMetrics.target,
                            alignment: .leading)
                        .contentShape(.rect)
                }
            }
        }
    }

    /// The plain yes: `allow_once` if the adapter offers one, otherwise the
    /// first thing that allows at all.
    private var allow: PermissionOption? {
        options.first { $0.kind.lowercased().contains("once") && isAllow($0) }
            ?? options.first(where: isAllow)
    }

    private var reject: PermissionOption? {
        options.first { isReject($0) }
    }

    /// Everything that is not the straight yes or no — kept, because an adapter
    /// may offer options this client has never heard of and swallowing them
    /// would make an answer unreachable.
    private var secondary: [PermissionOption] {
        options.filter { $0.id != allow?.id && $0.id != reject?.id }
    }

    private func isAllow(_ option: PermissionOption) -> Bool {
        let kind = option.kind.lowercased()
        return kind.contains("allow") || kind.contains("accept")
    }

    private func isReject(_ option: PermissionOption) -> Bool {
        let kind = option.kind.lowercased()
        return kind.contains("reject") || kind.contains("deny")
    }
}

private struct ApprovalCard: View {
    let pending: PendingPermission
    let onChoose: (String) -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: PaneMetrics.card) {
            // The one amber left in this file, with the ring on a gated tool
            // call: an agent is waiting on you, which is the only thing amber
            // says anywhere in this app.
            Label("Needs your approval", systemImage: "hand.raised.fill")
                .font(.subheadline.weight(.semibold))
                .foregroundStyle(.orange)

            ApprovalControls(options: pending.options, onChoose: onChoose)
        }
        // Radius 22 and no inset of its own — one of `composerStack`'s
        // surfaces, and that stack draws the edge now. This was 12, curving at
        // half the rate of the composer directly beneath it.
        .padding(PaneMetrics.edge)
        .background(
            TranscriptFill.attention,
            in: RoundedRectangle(cornerRadius: PaneMetrics.surfaceRadius))
        .overlay(
            RoundedRectangle(cornerRadius: PaneMetrics.surfaceRadius)
                .strokeBorder(TranscriptFill.attentionEdge))
    }


}

// MARK: - Diff

/// A unified diff, computed client-side from the two full texts a tool call
/// carries.
///
/// No syntax highlighting — cut in the spec. What earns the pixels here is
/// which lines changed, not what language they are in. This duplicates the
/// Mac's `DiffView` rather than sharing it: `AgentKit` holds the decode and
/// the reduce, which both platforms must agree on bit-for-bit, but layout is
/// each platform's own job by design (see the plan's architecture note), and
/// a diff view is layout, not derivation.
struct DiffView: View {
    let diff: Diff

    /// Beyond this many lines, the diff opens collapsed. A four-line edit is
    /// worth seeing on arrival; a four-hundred-line rewrite is not something
    /// to scroll past to reach the message after it — doubly so on a screen
    /// this narrow.
    private static let collapseThreshold = 20

    @State private var expanded = false

    private var lines: [DiffComputation.Line] {
        DiffComputation.compute(old: diff.oldText ?? "", new: diff.newText)
    }

    var body: some View {
        let rows = lines
        VStack(alignment: .leading, spacing: PaneMetrics.step) {
            header(for: rows)

            if rows.count > Self.collapseThreshold && !expanded {
                Button {
                    expanded = true
                } label: {
                    Text("Show \(rows.count) lines")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
                .buttonStyle(.plain)
            } else {
                diffBody(rows)
            }
        }
    }

    private func header(for rows: [DiffComputation.Line]) -> some View {
        let added = rows.filter { $0.kind == .added }.count
        let removed = rows.filter { $0.kind == .removed }.count
        return HStack(spacing: PaneMetrics.step) {
            Text(diff.path)
                .font(.caption.weight(.medium).monospaced())
                .lineLimit(1)
            Spacer(minLength: 8)
            if added > 0 {
                Text("+\(added)").foregroundStyle(.green)
            }
            if removed > 0 {
                Text("-\(removed)").foregroundStyle(.red)
            }
        }
        .font(.caption2.monospaced())
        .foregroundStyle(.secondary)
    }

    private func diffBody(_ rows: [DiffComputation.Line]) -> some View {
        // A narrower gutter than the Mac's: a phone has far fewer points to
        // spend on line numbers before the code column itself is squeezed
        // unreadable, and diffs here scroll horizontally besides (see the
        // wrapping `ScrollView` below).
        let gutterWidth: CGFloat = 26

        return ScrollView(.horizontal, showsIndicators: false) {
            VStack(alignment: .leading, spacing: 0) {
                ForEach(rows) { line in
                    HStack(spacing: 0) {
                        Text(line.oldNumber.map(String.init) ?? "")
                            .frame(width: gutterWidth, alignment: .trailing)
                        Text(line.newNumber.map(String.init) ?? "")
                            .frame(width: gutterWidth, alignment: .trailing)
                        Text(line.kind.marker)
                            .frame(width: 12, alignment: .center)
                        Text(line.text.isEmpty ? " " : line.text)
                            .fixedSize(horizontal: true, vertical: false)
                    }
                    .font(.caption2.monospaced())
                    .foregroundStyle(line.kind == .context ? .secondary : .primary)
                    .padding(.horizontal, PaneMetrics.tight)
                    // A point, not a step: this is leading between two lines of
                    // one listing, not a gap between two things. Anything on
                    // the scale above turns a diff into a list of rows.
                    .padding(.vertical, 1)
                    .background(line.kind.background)
                }
            }
            .padding(.vertical, PaneMetrics.tight)
        }
        .textSelection(.enabled)
        // A ground inside a ground: this box is already inside a tool card, so
        // it takes the tier below the one that card is drawn at.
        .background(TranscriptFill.recessed, in: RoundedRectangle(cornerRadius: 6))
    }
}


extension DiffComputation.Kind {
    fileprivate var marker: String {
        switch self {
        case .context: return ""
        case .added: return "+"
        case .removed: return "-"
        }
    }

    fileprivate var background: Color {
        switch self {
        case .context: return .clear
        case .added: return .green.opacity(0.15)
        case .removed: return .red.opacity(0.15)
        }
    }
}

// MARK: - Composer

/// The prompt field, plus everything that hangs off it: slash commands, file
/// mentions, image attachments, and the agent-mode switcher. One row, the
/// same rule the constraint file states for the Mac: no second header, no
/// footer — whatever this pane needs to say lives here or nowhere.
private struct AgentComposer: View {
    /// The modes the agent offers, with their human names.
    ///
    /// `[AgentChoice]`, not `[String]`. This was `[String]` and had been
    /// failing to compile since modes gained names on the Mac — the phone's
    /// picker was listing wire identifiers like `acceptEdits` before that, and
    /// nothing at all after.
    let availableModes: [AgentChoice]
    let agentMode: String?
    /// Every selector the agent advertises — mode, model, effort, subagent and
    /// whatever an adapter adds next.
    ///
    /// The phone had only `availableModes`, so a session offering five
    /// selectors showed one of them. The Mac has rendered the generic list
    /// since ACP config options landed; this is the same data, drawn for a
    /// screen with no room to lay them side by side.
    let configOptions: [ConfigOption]
    let onSetConfig: (String, String) -> Void
    /// Which agent this is — "Claude", "Codex". The conversation's own name is
    /// already in the title bar; what a fleet of several harnesses needs is to
    /// tell them apart.
    let harness: String
    let availableCommands: [AgentChoice]
    let workspaceID: String?
    let core: ClientCore
    let onSend: (String, [(mime: String, data: Data)]) -> Void
    let onSetMode: (String) -> Void

    @State private var text = ""
    @State private var cursor = 0
    @State private var mentionResults: [String] = []
    @State private var mentionSearch: Task<Void, Never>?
    @State private var attachments: [ComposerAttachment] = []
    @State private var photoPickerItem: PhotosPickerItem?
    /// Why the last attachment did not attach. Shown in the composer.
    @State private var attachmentError: String?
    @State private var fieldHeight: CGFloat = UIFont.preferredFont(forTextStyle: .body).lineHeight

    private var token: ComposerToken { activeToken(in: text, cursor: cursor) }

    /// A picture on its own is a message.
    ///
    /// This was text-only, which disagreed with `send` — that guard has always
    /// accepted either — so the one case where the disagreement showed was a
    /// screenshot with nothing typed: the thumbnail sat in the strip with the
    /// send button greyed out beside it, and the only way forward was to type
    /// something you did not mean.
    private var canSend: Bool {
        !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || !attachments.isEmpty
    }

    var body: some View {
        // A card that floats over the conversation, not a bar welded beneath it.
        //
        // This used to be a full-width strip on `.bar`, drawn to read as the
        // keyboard's top edge. That was the right instinct for iOS 18 and is
        // the wrong shape now: the platform's own messaging surfaces float a
        // rounded, glass field above scrolling content, and a squared-off slab
        // spanning edge to edge reads as a control from two releases ago.
        //
        // The transcript scrolls behind it — see `transcriptBody`'s
        // `safeAreaInset` — which is what the glass is for. A material with
        // nothing passing under it is just a grey rectangle.
        VStack(alignment: .leading, spacing: 0) {
            suggestions

            if !attachments.isEmpty {
                attachmentStrip
            }

            // Red, like every other failure on this screen. It was amber,
            // which in this app means an agent is waiting on you.
            //
            // Tap to dismiss, so it is a control and gets a control's height:
            // it was a caption you had to hit within about 15 points.
            if let attachmentError {
                Text(attachmentError)
                    .font(.caption)
                    .foregroundStyle(.red)
                    .frame(
                        maxWidth: .infinity, minHeight: PaneMetrics.target,
                        alignment: .leading)
                    .contentShape(.rect)
                    .onTapGesture { self.attachmentError = nil }
            }

            // Two rows, the same shape the Mac settled on: what you are
            // writing, then everything that acts on it. One row put a growing
            // field between four fixed controls, so the buttons moved as you
            // typed and the field never had the width it needed.
            // Settings on top, the message and its send button beneath.
            //
            // The other way round left the field stranded at the top of the
            // card with a band of dead space under it, and put the send button
            // at the end of a row of small secondary controls rather than
            // beside the thing it sends. This is the shape every messaging app
            // on the platform uses, for the same reason.
            VStack(alignment: .leading, spacing: PaneMetrics.step) {
                // Every control in this row is 44 points tall and none of them
                // LOOKS 44 points tall — the glyph and the capsules keep the
                // size they had, and the band around each is what a thumb
                // actually gets. Before this the row was a 22-point strip of
                // targets, which is half the floor.
                HStack(spacing: PaneMetrics.step) {
                    PhotosPicker(selection: $photoPickerItem, matching: .images) {
                        Image(systemName: "photo.badge.plus")
                            .font(.system(size: 15))
                            .frame(
                                minWidth: PaneMetrics.chip, minHeight: PaneMetrics.target)
                            .contentShape(.rect)
                    }
                    .onChange(of: photoPickerItem) { _, item in loadPickedPhoto(item) }

                    ForEach(inlineOptions) { option in
                        inlineSelector(option)
                    }

                    settingsMenu

                    Spacer(minLength: 0)
                }
                // Grey, not accent.
                //
                // A `Menu` and a `PhotosPicker` tint their labels with the app's
                // accent color, and `.foregroundStyle` on the label inside does
                // not survive it. So the whole row came out blue and read as a
                // line of links — things that navigate — rather than as controls
                // that change what the next message costs. Tinting the row is
                // what actually reaches them.
                .tint(.secondary)

                HStack(alignment: .bottom, spacing: PaneMetrics.step) {
                    fieldWithPlaceholder

                    Button(action: send) {
                        Image(systemName: "arrow.up.circle.fill")
                            .font(.system(size: 27))
                            .symbolRenderingMode(.hierarchical)
                            // The glyph stays 27; the target is 44. Send is the
                            // control this screen exists to press.
                            .frame(width: PaneMetrics.target, height: PaneMetrics.target)
                            .contentShape(.circle)
                    }
                    .disabled(!canSend)
                }
            }
            .padding(.horizontal, PaneMetrics.edge)
            .padding(.vertical, PaneMetrics.card)
        }
        // Taps land ON the card, not through it.
        //
        // A glass surface is a background, and a background is not a hit target
        // — so a tap in the gap between the chips and the field went straight
        // through to whatever the conversation had scrolled under there. Tapping
        // dead space in an input box must never activate something you cannot
        // see.
        .background(
            // BEHIND the content, not in front of it.
            //
            // Absorbing taps on the card itself risks winning them from the
            // buttons inside — the photo picker and the selectors live in this
            // rectangle. As a background it hit-tests after its children, so it
            // catches only what they did not want: the dead space between them,
            // which used to fall through the glass to whatever the conversation
            // had scrolled underneath.
            Color.clear
                .contentShape(Rectangle())
                .onTapGesture {}
        )
        // Radius 22, and no horizontal inset here: `composerStack` hoisted it,
        // so this card and everything stacked above it share one left edge.
        .modifier(GlassSurface())
        .padding(.bottom, PaneMetrics.step)
        .onChange(of: text) { _, _ in scheduleMentionSearch() }
        .onChange(of: cursor) { _, _ in scheduleMentionSearch() }
    }

    // MARK: Field

    private var fieldWithPlaceholder: some View {
        // `.topLeading`, not `.leading`: centered, the placeholder sat halfway
        // down a box that was itself too tall, which read as a text field that
        // had lost its text rather than one waiting for some.
        ZStack(alignment: .topLeading) {
            // `.secondary`, not `.tertiary`. A placeholder is the field's
            // label until you type — the only thing saying which agent this
            // message goes to — and tertiary on glass is a third level of
            // grey over a surface that is already sampling whatever scrolled
            // under it.
            if text.isEmpty {
                Text("Message \(harness)")
                    .foregroundStyle(.secondary)
                    // 2 is `ComposerTextView`'s own `textContainerInset`, so
                    // the placeholder sits exactly where the first typed
                    // character will land rather than a point above it.
                    .padding(.top, 2)
            }
            ComposerTextView(text: $text, cursor: $cursor, measuredHeight: $fieldHeight)
                .frame(height: fieldHeight)
        }
    }

    // MARK: Mode switcher

    /// The selectors worth a permanent place on a phone.
    ///
    /// Mode, model and effort: what the agent is allowed to do without asking,
    /// what it costs, and how hard it tries. All three get reached for
    /// mid-session — mode most of all, since it is what you change when the
    /// approvals start getting tedious. Subagent and fast mode are set once and
    /// forgotten, so they fold into the menu beside them.
    ///
    /// Values only: "Manual", "Sonnet", "High" say what they are, and a phone
    /// has no room to print the question as well. The menu spells both out.
    private static let inlineIDs = ["mode", "model", "effort"]

    private var inlineOptions: [ConfigOption] {
        Self.inlineIDs.compactMap { id in configOptions.first { $0.id == id } }
    }

    private var foldedOptions: [ConfigOption] {
        configOptions.filter { !Self.inlineIDs.contains($0.id) }
    }

    /// One selector, shown in full.
    @ViewBuilder
    private func inlineSelector(_ option: ConfigOption) -> some View {
        Menu {
            ForEach(option.options) { choice in
                Button {
                    onSetConfig(option.id, choice.id)
                } label: {
                    if choice.id == option.currentValue {
                        Label(choice.name, systemImage: "checkmark")
                    } else {
                        Text(choice.name)
                    }
                }
            }
        } label: {
            // A bordered chip rather than plain tinted text, which read as a
            // link — something that navigates — rather than a control that
            // changes what the next message costs.
            //
            // A fixed 26-point capsule rather than padding around a caption:
            // this chip and the overflow beside it were sized by two different
            // fonts plus two different paddings and came out two points apart.
            Text(option.options.first { $0.id == option.currentValue }?.name ?? option.name)
                .font(.caption)
                .lineLimit(1)
                .foregroundStyle(.secondary)
                .padding(.horizontal, PaneMetrics.step)
                .frame(height: PaneMetrics.chip)
                .background(Capsule().fill(.quaternary))
                .frame(minHeight: PaneMetrics.target)
                .contentShape(.capsule)
        }
    }

    /// Everything not worth a permanent place, behind one control.
    ///
    /// Nested submenus rather than a flat list: these are several separate
    /// questions, and flattening them would put "High" and "Opus" and "Plan
    /// Mode" side by side with nothing saying which question each answers. The
    /// Mac folds its overflow the same way; a phone has no room to show any of
    /// them inline, so everything folds.
    @ViewBuilder
    private var settingsMenu: some View {
        if !configOptions.isEmpty {
            Menu {
                ForEach(foldedOptions.isEmpty ? configOptions : foldedOptions) { option in
                    Menu(menuTitle(option)) {
                        ForEach(option.options) { choice in
                            Button {
                                onSetConfig(option.id, choice.id)
                            } label: {
                                if choice.id == option.currentValue {
                                    Label(choice.name, systemImage: "checkmark")
                                } else {
                                    Text(choice.name)
                                }
                            }
                        }
                    }
                }
            } label: {
                Image(systemName: "ellipsis")
                    .font(.system(size: 15))
                    .foregroundStyle(.secondary)
                    .padding(.horizontal, PaneMetrics.step)
                    .frame(height: PaneMetrics.chip)
                    .background(Capsule().fill(.quaternary))
                    .frame(minHeight: PaneMetrics.target)
                    .contentShape(.capsule)
            }
        } else if availableModes.count > 1 {
            // An older daemon that only reports modes still gets a picker.
            Menu {
                ForEach(availableModes) { mode in
                    Button {
                        onSetMode(mode.id)
                    } label: {
                        if mode.id == agentMode {
                            Label(mode.name, systemImage: "checkmark")
                        } else {
                            Text(mode.name)
                        }
                    }
                }
            } label: {
                Image(systemName: "ellipsis")
                    .font(.system(size: 15))
                    .foregroundStyle(.secondary)
                    .padding(.horizontal, PaneMetrics.step)
                    .frame(height: PaneMetrics.chip)
                    .background(Capsule().fill(.quaternary))
                    .frame(minHeight: PaneMetrics.target)
                    .contentShape(.capsule)
            }
        }
    }

    /// "Model · Sonnet" — the question and its answer, so a menu of five reads
    /// as five settings rather than five words.
    private func menuTitle(_ option: ConfigOption) -> String {
        let current = option.options.first { $0.id == option.currentValue }?.name
        guard let current else { return option.name }
        return "\(option.name) · \(current)"
    }

    // MARK: Slash / mention suggestions

    @ViewBuilder
    private var suggestions: some View {
        switch token {
        case .none:
            EmptyView()
        case let .slash(prefix, range):
            let matches = availableCommands.filter {
                prefix.isEmpty || $0.name.lowercased().hasPrefix(prefix.lowercased())
            }
            if !matches.isEmpty {
                // Not a chevron: these rows do not expand, and an affordance
                // that promises otherwise is one nothing keeps.
                SuggestionList(items: matches, icon: "terminal") { command in
                    apply(range: range, replacement: "/\(command) ")
                }
            }
        case let .mention(_, range):
            if !mentionResults.isEmpty {
                SuggestionList(
                    items: mentionResults.map { AgentChoice(id: $0, name: $0, description: "") },
                    icon: "doc"
                ) { path in
                    apply(range: range, replacement: "@\(path) ")
                }
            }
        }
    }

    /// Replace the active token with a chosen completion, and put the caret
    /// right after what was inserted.
    ///
    /// Recomputed from character counts rather than reusing `range`'s indices
    /// after the mutation: a `String.Index` captured before `replaceSubrange`
    /// is not guaranteed valid after it, and the failure mode for guessing
    /// wrong is silent — a crash or a caret in the wrong place — not a
    /// compiler error.
    private func apply(range: Range<String.Index>, replacement: String) {
        let prefixCount = text.distance(from: text.startIndex, to: range.lowerBound)
        text.replaceSubrange(range, with: replacement)
        cursor = prefixCount + replacement.count
        mentionResults = []
    }

    private func scheduleMentionSearch() {
        mentionSearch?.cancel()
        guard case let .mention(prefix, _) = token, let workspaceID else {
            mentionResults = []
            return
        }
        mentionSearch = Task {
            // Debounced rather than fired on every keystroke: each search is
            // an ssh round trip, and a search whose result arrives after the
            // next keystroke already superseded it is nothing but cost.
            try? await Task.sleep(for: .milliseconds(200))
            guard !Task.isCancelled else { return }
            guard
                let data = try? await core.call(
                    "worktree.file_search",
                    ["workspace": workspaceID, "query": prefix, "limit": 20]),
                let decoded = try? JSONDecoder().decode(FileSearchResult.self, from: data)
            else { return }
            guard !Task.isCancelled else { return }
            mentionResults = decoded.paths
        }
    }

    private struct FileSearchResult: Decodable { let paths: [String] }

    // MARK: Attachments
    //
    // Delivered, end to end: the picker fills `attachments`, `send` hands them
    // to `AgentStream.send`, which base64s them into `terminal.agent_prompt`;
    // `ffi.rs` decodes them into `ImageBlock`s and the daemon passes them to
    // the shim. The comment that used to sit here said the wire could not carry
    // an image and that sending deliberately dropped it — that was true of the
    // Claude backend and of nothing else, and it stayed here long after the
    // protocol grew the block.

    private var attachmentStrip: some View {
        VStack(alignment: .leading, spacing: PaneMetrics.tight) {
            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: PaneMetrics.step) {
                    ForEach(attachments) { attachment in
                        ZStack(alignment: .topTrailing) {
                            Image(uiImage: attachment.image)
                                .resizable()
                                .scaledToFill()
                                .frame(width: 56, height: 56)
                                .clipShape(RoundedRectangle(cornerRadius: 8))
                            Button {
                                attachments.removeAll { $0.id == attachment.id }
                            } label: {
                                Image(systemName: "xmark.circle.fill")
                                    .font(.system(size: 15))
                                    .foregroundStyle(.white, .black.opacity(0.6))
                            }
                            .offset(x: 5, y: -5)
                        }
                    }
                }
                .padding(.horizontal, PaneMetrics.card)
                .padding(.top, PaneMetrics.step)
            }
        }
    }

    private func loadPickedPhoto(_ item: PhotosPickerItem?) {
        guard let item else { return }
        Task {
            // Loudly, not silently.
            //
            // This used to `try?` the load and drop the result on failure, so a
            // photo that could not be read — an iCloud asset not on the device
            // is the usual reason — looked exactly like a picker that did
            // nothing. "Doesn't seem possible to upload images" is what that
            // failure mode sounds like from outside.
            let loaded = try? await item.loadTransferable(type: Data.self)
            guard let data = loaded, let image = UIImage(data: data) else {
                attachmentError = "That photo could not be read. If it lives in "
                    + "iCloud, open it in Photos first so it downloads."
                photoPickerItem = nil
                return
            }
            // PNG only when it really is one — a picker hands back HEIC as
            // often as anything else, and telling the agent the wrong type
            // fails at the far end.
            let mime = data.starts(with: [0x89, 0x50, 0x4E, 0x47]) ? "image/png" : "image/jpeg"
            // Shrunk to fit ONE control envelope, which is what a prompt's
            // image has to fit inside — see `PromptImageBudget`. Sending the
            // original bytes is what a picked photo used to do, and the
            // protocol refused every one of them over a megabyte.
            guard let (payload, payloadMime) = PromptImageBudget.fit(
                image, original: data, mime: mime)
            else {
                attachmentError = "That photo couldn’t be prepared to send."
                photoPickerItem = nil
                return
            }
            attachments.append(
                ComposerAttachment(image: image, data: payload, mime: payloadMime))
            attachmentError = nil
            photoPickerItem = nil
        }
    }

    private func send() {
        let message = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !message.isEmpty || !attachments.isEmpty else { return }
        // The attachments GO now. They used to be cleared on the next line
        // without ever being sent: the picker worked, the thumbnail appeared,
        // and the image was dropped on the floor.
        onSend(message, attachments.compactMap(\.payload))
        text = ""
        cursor = 0
        attachments = []
        mentionResults = []
    }
}

private struct ComposerAttachment: Identifiable {
    let id = UUID()
    let image: UIImage
    /// The bytes to send — the original when it already fits inside one control
    /// envelope, a resized JPEG when it did not. See `PromptImageBudget`.
    ///
    /// The `image` above stays the FULL-size one, because it is what the
    /// thumbnail is drawn from and shrinking that would show a worse picture
    /// than was actually sent.
    /// Formerly the original bytes, kept so the agent gets the picture the user picked
    /// rather than one re-encoded from a `UIImage` for display.
    let data: Data
    let mime: String

    var payload: (mime: String, data: Data)? { (mime, data) }
}

/// The floating list a slash command or an `@` mention pops open, tap to
/// accept.
private struct SuggestionList: View {
    let items: [AgentChoice]
    let icon: String
    let onChoose: (String) -> Void

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 0) {
                ForEach(items.prefix(8), id: \.id) { item in
                    Button {
                        onChoose(item.name)
                    } label: {
                        HStack(spacing: PaneMetrics.step) {
                            Image(systemName: icon)
                                .font(.footnote)
                                .foregroundStyle(.secondary)
                                .frame(width: 14)
                            VStack(alignment: .leading, spacing: 2) {
                                Text(item.name)
                                    .font(.footnote.monospaced())
                                // What it does. The adapter has always sent
                                // this and it was thrown away, so the list
                                // named commands without saying what any of
                                // them were for.
                                if !item.description.isEmpty {
                                    Text(item.description)
                                        .font(.caption2)
                                        .foregroundStyle(.secondary)
                                        .lineLimit(2)
                                }
                            }
                            Spacer(minLength: 0)
                        }
                        .frame(
                            maxWidth: .infinity, minHeight: PaneMetrics.target,
                            alignment: .leading)
                        .padding(.horizontal, PaneMetrics.card)
                        .padding(.vertical, PaneMetrics.step)
                    }
                    .buttonStyle(.plain)
                    Divider().padding(.leading, PaneMetrics.card)
                }
            }
        }
        .frame(maxHeight: 220)
        .background(.thickMaterial)
    }
}

/// A plain multi-line text field that also reports where the caret is.
///
/// `activeToken(in:cursor:)` needs a caret position to decide whether the `/`
/// or `@` under it should open a picker, and no SwiftUI `TextField` or
/// `TextEditor` on this deployment target hands one back for a plain `String`
/// binding. `UITextViewDelegate.textViewDidChangeSelection` does — which is
/// why this is a representable rather than a built-in control, the same
/// reasoning that put a `UIViewRepresentable` under `TerminalView`'s keyboard
/// input, just for a different reason: that one needed raw keystrokes with no
/// text field at all, and this one needs an ordinary text field that also
/// exposes its selection.
private struct ComposerTextView: UIViewRepresentable {
    @Binding var text: String
    @Binding var cursor: Int
    /// How tall the typed text is, reported after it changes rather than
    /// negotiated during layout.
    ///
    /// A `UITextView` has no useful intrinsic height, so given a flexible frame
    /// it simply took the largest one offered — a one-line message rendered as
    /// a box three lines deep with the placeholder floating in the middle of
    /// it. The Mac hit this too; measuring during layout is what froze it
    /// there, so this goes the same way round: text changes, then height
    /// changes, then layout happens.
    @Binding var measuredHeight: CGFloat

    func makeUIView(context: Context) -> UITextView {
        let view = UITextView()
        view.font = .preferredFont(forTextStyle: .body)
        view.backgroundColor = .clear
        view.isScrollEnabled = true
        // Tight: the card around it already provides the padding, and a text
        // view that adds its own on top is what made a one-line field look
        // like a three-line one.
        view.textContainerInset = UIEdgeInsets(top: 2, left: 0, bottom: 2, right: 0)
        view.textContainer.lineFragmentPadding = 0
        view.delegate = context.coordinator
        view.text = text
        DispatchQueue.main.async { context.coordinator.report(view) }
        return view
    }

    /// Only pushes `text` into the view when it changed from OUTSIDE the
    /// view's own typing — a slash-command completion, say, or `send()`
    /// clearing the field. When the user is simply typing, `text` already
    /// equals `uiView.text` because `textViewDidChange` just set it, and
    /// overwriting `.text` here on every SwiftUI render pass would reset
    /// `UITextView`'s own undo stack and marked (IME composition) text out
    /// from under whatever is mid-composition.
    func updateUIView(_ uiView: UITextView, context: Context) {
        // Measured here as well as on change: at `makeUIView` the view has no
        // width yet, so the first measurement is worthless and an empty field
        // would keep whatever height it was born with.
        context.coordinator.report(uiView)
        guard uiView.text != text else { return }
        uiView.text = text
        let location = ComposerTextView.utf16Offset(forCharacterOffset: cursor, in: text)
        uiView.selectedRange = NSRange(location: location, length: 0)
    }

    func makeCoordinator() -> Coordinator { Coordinator(self) }

    final class Coordinator: NSObject, UITextViewDelegate {
        var parent: ComposerTextView
        init(_ parent: ComposerTextView) { self.parent = parent }

        func textViewDidChange(_ textView: UITextView) {
            parent.text = textView.text
            report(textView)
        }

        /// The height of what has been typed, clamped to what the composer
        /// will show.
        func report(_ textView: UITextView) {
            let width = textView.bounds.width
            guard width > 0 else { return }
            let fitted = textView.sizeThatFits(
                CGSize(width: width, height: .greatestFiniteMagnitude)
            ).height
            // The floor is one line of the body font, not a guess: an empty
            // field that reserves three lines reads as a box that has lost its
            // text rather than one waiting for some.
            let oneLine = UIFont.preferredFont(forTextStyle: .body).lineHeight
            let clamped = min(max(fitted, oneLine), 110)
            guard abs(clamped - parent.measuredHeight) > 0.5 else { return }
            let binding = parent.$measuredHeight
            DispatchQueue.main.async { binding.wrappedValue = clamped }
        }

        func textViewDidChangeSelection(_ textView: UITextView) {
            parent.cursor = ComposerTextView.characterOffset(
                forUTF16Offset: textView.selectedRange.location, in: textView.text)
        }
    }

    /// `UITextView.selectedRange` is UTF-16 code units; `activeToken` counts
    /// `Character`s. The two agree for plain ASCII commands and paths — the
    /// only content these pickers ever match against — and diverge only
    /// inside a multi-scalar grapheme cluster (an emoji, say), where landing
    /// mid-cluster falls back to the nearest end rather than crashing.
    fileprivate static func characterOffset(forUTF16Offset utf16Offset: Int, in text: String) -> Int {
        guard
            let utf16Index = text.utf16.index(
                text.utf16.startIndex, offsetBy: utf16Offset, limitedBy: text.utf16.endIndex),
            let index = String.Index(utf16Index, within: text)
        else { return text.count }
        return text.distance(from: text.startIndex, to: index)
    }

    fileprivate static func utf16Offset(forCharacterOffset characterOffset: Int, in text: String) -> Int {
        guard let index = text.index(text.startIndex, offsetBy: characterOffset, limitedBy: text.endIndex)
        else { return (text as NSString).length }
        return text.utf16.distance(from: text.utf16.startIndex, to: index.samePosition(in: text.utf16) ?? text.utf16.endIndex)
    }
}


/// The composer's surface: Liquid Glass, because that is what a control resting
/// ON scrolling content is on this platform.
///
/// No fallback. This app's minimum is iOS 26, so the material-and-hairline
/// approximation that used to sit behind an availability check was dead code
/// pretending to be portability.
/// Putting the keyboard away without owning the field that raised it.
///
/// The composer's text view is a `UIViewRepresentable` several layers down, so
/// there is no `FocusState` up here to set false. Asking the responder chain is
/// the framework's own answer to "whatever is focused, stop being focused" and
/// does not require this view to know what that is.
@MainActor
enum KeyboardDismissal {
    static func now() {
        UIApplication.shared.sendAction(
            #selector(UIResponder.resignFirstResponder), to: nil, from: nil, for: nil)
    }
}

/// The steps this screen's layout is allowed to take.
///
/// Written down because it had stopped being a scale. These two files held 3,
/// 4, 5, 6, 7, 8, 9, 10, 11, 12 and 14 as gaps and insets, in a tree that
/// argues for every other number it uses — and with eleven values in play
/// nothing is a step above anything, it is just the number that looked right
/// the day the view was written. Four values, and the reason each exists.
///
/// The two surface constants are the phone's, not this screen's:
/// `TerminalTabStrip` and `TerminalKeyRow` already draw at 22 and 10, and a
/// composer floating two points inside a tab strip with corners curving at
/// half the rate is what made three surfaces on one screen read as three
/// different objects.
enum PaneMetrics {
    /// Inside one control — a glyph and the word next to it.
    static let tight: CGFloat = 4
    /// Between two things in one group. The default; reach for another only
    /// with a reason.
    static let step: CGFloat = 8
    /// A transcript card's inner edge, and the gap between cards.
    static let card: CGFloat = 12
    /// A radius-22 surface's inner edge. Larger than a card's because the
    /// corner is: text run to 12 points of a 22-point curve reads as text
    /// falling out of the rounding.
    static let edge: CGFloat = 16

    /// Every floating surface's corner, and the inset it sits at — the tab
    /// strip's pair and the terminal key row's, shipped there first.
    ///
    /// The inset is deliberately not a step on the scale above. It is where
    /// this platform's floating bars sit, and matching the strip 44 points
    /// below the composer matters more than matching the padding inside it.
    static let surfaceRadius: CGFloat = 22
    static let surfaceInset: CGFloat = 10

    /// The smallest a thing you tap is allowed to be, which is the
    /// guideline's floor and not a preference.
    ///
    /// Almost nothing on this screen cleared it: the approve row was 28, the
    /// composer's chips 22, the queue's actions 16. The pattern throughout is
    /// the one the tab strip shipped — the visible control keeps the size it
    /// had, a 44-point frame goes around it, and a `contentShape` makes the
    /// band between the two live rather than merely occupied.
    static let target: CGFloat = 44

    /// The visible height of a composer chip, under a 44-point target.
    ///
    /// One number for both kinds of chip. The selectors were 3 points of
    /// vertical padding on `.caption` and the overflow was 4 points on a
    /// 15-point glyph, so two capsules sitting side by side in one row ended
    /// up two points different in height — close enough to look like a
    /// mistake and not close enough to look like anything else.
    static let chip: CGFloat = 26
}

/// The fills the transcript draws on, as one scale.
///
/// There were eight hand-mixed opacities across these files — 0.035, 0.04,
/// 0.07 twice, 0.08, 0.10, 0.12, 0.15 — beside `.quaternary` used semantically
/// a few hundred lines away, so half the file was on the platform's hierarchy
/// and half was on numbers. Worse than untidy: the user's bubble and every
/// tool card were the SAME 0.07, directly under a comment saying the bubble
/// has to read as a different speaker at a glance.
///
/// The neutral tiers are the platform's, so they follow the appearance the
/// theme puts this pane in. The tinted ones are not on that hierarchy — they
/// are a color that means something — so they stay explicit, and are named
/// here rather than mixed again at each call site.
enum TranscriptFill {
    /// The reader's own words. One full step above anything the agent draws,
    /// which is what "a different speaker" has to be to survive a glance.
    static let speaker = AnyShapeStyle(.tertiary)
    /// A container around the agent's output: a tool call, a subagent still
    /// working, a gap the transcript is reporting.
    static let container = AnyShapeStyle(.quaternary)
    /// A container that has finished, and a ground drawn inside another
    /// ground — the diff's own bed.
    static let recessed = AnyShapeStyle(.quinary)

    /// An agent is waiting on you. The one thing amber means in this app.
    static let attention = AnyShapeStyle(Color.orange.opacity(0.08))
    static let attentionEdge = AnyShapeStyle(Color.orange.opacity(0.35))
    /// The stronger edge, for the request drawn ON the call it is about.
    static let attentionRing = AnyShapeStyle(Color.orange.opacity(0.45))
    /// Something went wrong. Red, because amber is spoken for.
    static let alarm = AnyShapeStyle(Color.red.opacity(0.12))
}

struct GlassSurface: ViewModifier {
    var radius: CGFloat = PaneMetrics.surfaceRadius

    func body(content: Content) -> some View {
        content.glassEffect(.regular, in: .rect(cornerRadius: radius))
    }
}

/// The agent's reasoning: streaming while it happens, folded away after.
///
/// The Mac's `ThoughtRow`, ported. A phone watching a long turn used to see one
/// collapsed word and no sign of movement, which reads as an agent that has
/// hung rather than one that is thinking.
private struct ThoughtRow: View {
    let text: String
    let isLive: Bool

    @State private var expanded = false

    var body: some View {
        VStack(alignment: .leading, spacing: PaneMetrics.step) {
            Button {
                withAnimation(.spring(response: 0.22, dampingFraction: 0.82)) { expanded.toggle() }
            } label: {
                // A 16-point caption was the entire target for folding a
                // thought open. The words stay a caption; the band around them
                // is 44 and runs the width of the row, because there is
                // nothing else on this line to hit by accident.
                HStack(spacing: PaneMetrics.step) {
                    Image(systemName: "chevron.right")
                        .font(.caption2)
                        .rotationEffect(.degrees(showing ? 90 : 0))
                    Text(isLive ? "Thinking…" : "Thought")
                        .font(.caption.weight(.medium))
                    Spacer(minLength: 0)
                }
                .foregroundStyle(.secondary)
                .frame(minHeight: PaneMetrics.target)
                .contentShape(.rect)
            }
            .buttonStyle(.plain)

            if showing {
                // While it is being written, only the last few lines — enough
                // to see it moving, which is the whole point. One collapsed
                // word looks stuck; the whole thing pushes the conversation off
                // the screen.
                MarkdownText(text: isLive && !expanded ? Self.tail(of: text) : text, secondary: true)
            }
        }
        .animation(.spring(response: 0.22, dampingFraction: 0.82), value: isLive)
    }

    /// Open while live unless the reader has closed it; closed after unless the
    /// reader has opened it.
    private var showing: Bool { expanded || isLive }

    private static func tail(of text: String, lines: Int = 5) -> String {
        let all = text.split(separator: "\n", omittingEmptySubsequences: false)
        guard all.count > lines else { return text }
        return all.suffix(lines).joined(separator: "\n")
    }
}

/// A turn in progress, said where the work is appearing.
///
/// The Mac's `WorkingRow`, ported — including its clock: phase from a
/// `TimelineView` and a start captured on appear, because a `repeatForever`
/// animation restarted from zero by every streamed event never visibly moves,
/// and phase taken from the wall clock starts the sweep mid-word.
/// Reduce Motion turns the sweep off, and `TimelineView(.animation)` with it.
///
/// Required rather than polite: this is a repeating decorative animation with
/// no information in it — the row says "Working…" whether it shimmers or not —
/// which is exactly what the setting exists to stop. It also runs at DISPLAY
/// RATE for the entire length of a turn, so on a phone the check doubles as
/// the escape hatch from 120 layout passes a second while an agent thinks.
private struct WorkingRow: View {
    private static let period: TimeInterval = 1.1

    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    @State private var start: Date?

    @ViewBuilder
    var body: some View {
        if reduceMotion {
            // Secondary rather than the gradient's midpoint: the sweep was the
            // only thing marking this row as unfinished, so without it the row
            // has to say so by being quieter than the words above it.
            Text("Working…")
                .font(.callout)
                .foregroundStyle(.secondary)
        } else {
            TimelineView(.animation) { context in
                Text("Working…")
                    .font(.callout)
                    .foregroundStyle(
                        LinearGradient(
                            stops: stops(at: phase(now: context.date)),
                            startPoint: .leading,
                            endPoint: .trailing))
            }
            .onAppear { if start == nil { start = Date() } }
        }
    }

    private func phase(now: Date) -> Double {
        guard let start else { return 0 }
        return now.timeIntervalSince(start)
            .truncatingRemainder(dividingBy: Self.period) / Self.period
    }

    private func stops(at phase: Double) -> [Gradient.Stop] {
        let center = -0.35 + phase * 1.7
        let width = 0.3
        return [
            .init(color: .secondary, location: min(max(center - width, 0), 1)),
            .init(color: .primary, location: min(max(center, 0), 1)),
            .init(color: .secondary, location: min(max(center + width, 0), 1)),
        ]
    }
}
