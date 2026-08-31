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
    // MARK: What a conversation's scrolling has to do
    //
    // Rebuilt from these, rather than grown one guard at a time. The behavior
    // this replaced had three pieces of state, a generation counter and a
    // five-step timer ladder all trying to answer one question — WHO moved the
    // scroll view — by inference from geometry, which cannot answer it.
    //
    // 1. FOLLOWING THE TAIL IS A MODE THE READER OWNS. It turns off when the
    //    reader scrolls away and on when they come back, and nothing else may
    //    touch it. Every mistake this surface made was some layout event —
    //    the keyboard, the composer growing, a lazy row resolving its height —
    //    being read back as the reader having scrolled.
    // 2. SO ONLY A FINGER COUNTS. `onScrollPhaseChange` says whether the
    //    scroll in progress is the reader's or ours; the at-bottom test is
    //    consulted only while it is theirs. That one distinction is what the
    //    generation counter was standing in for.
    // 3. PINNED MEANS RE-ANCHORED WHENEVER EITHER SIDE MOVES. Content growing
    //    is only half of it: the viewport shrinks too — keyboard, a message
    //    growing to three lines, a plan panel appearing — and reserving room
    //    for that WITHOUT re-anchoring is exactly how the tail ended up
    //    underneath the composer. See `obstruction`.
    // 4. UNPINNED IS INVIOLABLE. Nothing scrolls a reader who has scrolled
    //    away. Not a streamed token, not the keyboard, not a finished turn.
    // 5. THE WAY BACK IS OFFERED, NOT TAKEN. See `jumpToLatest`.
    // 6. FOLLOWING IS NOT ANIMATED. A reply arrives several events a second
    //    and an eased scroll per event is the jitter. The content grows at the
    //    bottom edge, so an unanimated re-anchor is invisible. The one scroll
    //    worth animating is the one the reader asked for.
    // 7. IT OPENS AT THE TAIL. First mount and every return.

    /// Whether the transcript is following its own tail.
    @State private var pinnedToTail = true
    /// Whether the scroll in progress is one the READER started. See 2 above.
    @State private var readerIsDriving = false
    /// Whether anything has arrived since the reader scrolled away.
    @State private var arrivedWhileAway = false
    /// The re-anchor that outlives the keyboard's own animation. See
    /// `anchorToTail(animated:settling:)`.
    @State private var settleTask: Task<Void, Never>?
    /// The row the scroll view holds still while heights around it resolve.
    @State private var scrollPosition = ScrollPosition(idType: Int.self)
    /// The bottom content inset actually in force, for the probe below — this
    /// is the number the regression was about and it was not observable.
    @State private var appliedInset: CGFloat = 0
    /// The end of the content. `Int` like every row id, because
    /// `scrollPosition(id:)` binds one type.
    private static let endOfTranscript = Int.max
    /// How far off the bottom still counts as the bottom. A lazy row resolving
    /// its height moves the tail by a few points at a time, so "at the bottom"
    /// after a redraw is rarely exact.
    private static let tailSlack: CGFloat = 40
    /// When to look again after the keyboard has been asked to move.
    ///
    /// `keyboardWillChangeFrame` reports the END rectangle, and iOS 26 then
    /// animates to it over several layout passes — so a single re-anchor lands
    /// against geometry still in motion and leaves the last rows underneath the
    /// keyboard. Kept from the behavior this replaced, which found the same
    /// thing the hard way; only the bookkeeping around it is new.
    private static let settleLadder = [80, 180, 300, 440]

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

    /// Whether the daemon is serving a session for this pane RIGHT NOW.
    ///
    /// `.starting` and nothing else, which is narrower than `hasSession`: that
    /// phase is the daemon having ANSWERED and said it holds no session — see
    /// `AgentStream.answered`. `.opening` and `.failing` are not knowing, and
    /// they keep Send live on purpose, because the retryable failure a send
    /// gets from a link that is down is more use than a greyed button, and the
    /// message is not lost.
    private var hasAgent: Bool { stream.phase != .starting }

    private var isWorking: Bool {
        guard let workspaceID else { return false }
        return connection.terminal(terminalID, in: workspaceID)?.agent == .working
    }

    #if DEBUG
    /// The canned conversation `AgentLayoutHarness` stands this pane on.
    ///
    /// A type and a STATIC, where this was five instance properties the harness
    /// set on an `AgentView` it constructed itself. Constructing the pane is
    /// what it may no longer do: the harness mounts the shell now, so the pane
    /// is built by `ShellPaneRealView` and `TerminalView` several levels down
    /// and there is nothing for an outside caller to reach into. A fixture that has to survive that
    /// trip is a static, and one value rather than five keeps "there is a
    /// fixture" a single question.
    ///
    /// `phase`, `waited` and `trouble` are here because the states the harness
    /// could not reach are precisely the ones that were wrong: `emptyState`
    /// draws only when the transcript has no rows, so every screen the "could
    /// not load" report is about needs a fixture with nothing in it and a phase
    /// set by hand. `isWorking` is NOT here — it rides the fleet the harness
    /// stands up, the way the app reads it.
    struct Fixture {
        var events: [Sequenced]
        var phase: AgentStream.Phase = .live
        var waited: AgentStream.Waited = .aMoment
        var trouble: AgentStream.Trouble?
    }

    /// Set only by `AgentLayoutHarness`; `nil` everywhere else.
    static var fixture: Fixture?
    #endif

    // MARK: Following the tail

    /// How much of the transcript is covered by something resting on it.
    ///
    /// The larger of the two, because each is right in one state and blind in
    /// the other. With the keyboard DOWN the accessory is simply on screen and
    /// posts no keyboard-frame notification at all, so only its measured height
    /// knows it is there. With the keyboard UP the reported frame already
    /// includes the accessory and is the taller number, so it wins — and
    /// nothing is counted twice.
    ///
    /// Both are measured in the keyboard's window, which runs to the bottom of
    /// the SCREEN, so this number already contains the home-indicator strip.
    /// That is why the transcript below ignores its own bottom safe area
    /// rather than adding to it.
    ///
    /// Nothing reserved by a pane that is not on screen: its bar is not docked,
    /// so there is nothing down there to clear.
    private var obstruction: CGFloat {
        isVisible ? max(keyboard.height, barHeight) : 0
    }


    /// Whether the scroll view is parked at the end of the conversation.
    ///
    /// `visibleRect` already accounts for every safe-area inset. Adding
    /// `containerSize` to `contentOffset` does not, which makes a bottom-inset
    /// scroll view look permanently short of its tail even after the user has
    /// reached it.
    ///
    /// `>=` rather than a band around the tail, deliberately: a finger holding
    /// the transcript rubber-banded past its end has not scrolled away from
    /// anything. What that leniency cannot do is notice a transcript left
    /// overscrolled by a stale inset — which is why the inset is now correct at
    /// the source instead of being compensated for here.
    private static func isAtTail(_ geometry: ScrollGeometry) -> Bool {
        geometry.visibleRect.maxY >= geometry.contentSize.height - Self.tailSlack
    }

    /// The one writer of the mode. Reaching the tail is also how "you missed
    /// something" stops being true — the reader has now seen it.
    private func setPinned(_ atTail: Bool) {
        pinnedToTail = atTail
        if atTail { arrivedWhileAway = false }
    }

    /// Put the end of the conversation back against the bottom of the viewport.
    ///
    /// `settling` is for the changes that do not finish in one layout pass —
    /// the keyboard above all. See `settleLadder`. Each pass re-checks the
    /// mode and the finger, so a reader who starts scrolling mid-animation
    /// takes the transcript from it rather than fighting it.
    private func anchorToTail(animated: Bool, settling: Bool) {
        settleTask?.cancel()
        settleTask = nil
        if animated {
            withAnimation(.easeOut(duration: 0.25)) {
                scrollPosition.scrollTo(id: Self.endOfTranscript, anchor: .bottom)
            }
        } else {
            scrollPosition.scrollTo(id: Self.endOfTranscript, anchor: .bottom)
        }
        guard settling else { return }
        settleTask = Task { @MainActor in
            for delay in Self.settleLadder {
                try? await Task.sleep(for: .milliseconds(delay))
                guard !Task.isCancelled, pinnedToTail, !readerIsDriving else { return }
                scrollPosition.scrollTo(id: Self.endOfTranscript, anchor: .bottom)
            }
        }
    }

    /// The way back, offered rather than taken.
    ///
    /// A transcript that yanks the reader to the bottom is the single
    /// most-hated behavior a chat surface has. A transcript that strands them
    /// with no way back is the second, and it is the one this screen had: the
    /// only route to the end of a long conversation was to scroll there by
    /// hand. So the tail is never taken by force and is always one tap away.
    ///
    /// NO COUNT ON IT. A streamed reply coalesces into the row already on
    /// screen — see `Transcript` — so "3 new" would say one for a four-minute
    /// answer and nothing at all for a turn that was only tool calls. The
    /// honest question a count is trying to answer is whether anything has
    /// happened since you looked away, and a dot answers exactly that and
    /// claims nothing more.
    /// The way-back button's diameter, named because two places need it: the
    /// button itself, and the offset that lifts it clear of the composer's
    /// strip. A literal in both is a literal that drifts in one.
    private static let jumpDiameter: CGFloat = 38

    private var jumpToLatest: some View {
        Button {
            setPinned(true)
            // The one scroll worth animating: the reader asked for it, and the
            // movement is what tells them the tap did something.
            anchorToTail(animated: true, settling: false)
        } label: {
            Image(systemName: "chevron.down")
                .font(.system(size: 15, weight: .semibold))
                // Said explicitly, because a `Button` tints its label and
                // `.foregroundStyle` inside one does not survive it — the
                // blue-link reading half the controls on this screen have
                // already been corrected for.
                .foregroundStyle(.primary)
                .frame(width: Self.jumpDiameter, height: Self.jumpDiameter)
                .background(.regularMaterial, in: Circle())
                .overlay {
                    Circle().strokeBorder(Color.primary.opacity(0.12))
                }
                .overlay(alignment: .topTrailing) {
                    if arrivedWhileAway {
                        Circle()
                            .fill(Color.accentColor)
                            .frame(width: 10, height: 10)
                            .overlay { Circle().strokeBorder(TerminalPalette.background, lineWidth: 2) }
                            .offset(x: 1, y: -1)
                    }
                }
        }
        .buttonStyle(.plain)
        .accessibilityIdentifier("jump-to-latest")
        .accessibilityLabel(arrivedWhileAway ? "Jump to Latest, new messages" : "Jump to Latest")
        .transition(.scale(scale: 0.6).combined(with: .opacity))
    }

    var body: some View {
        VStack(spacing: 0) {
            transcriptBody
                .modifier(
                    AgentLayoutProbe(
                        keyboardHeight: keyboard.height, barHeight: barHeight,
                        appliedInset: appliedInset, followingTail: pinnedToTail,
                        measuring: !transcript.rows.isEmpty))
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
                // one number that is correct in both states — `obstruction`,
                // the keyboard's own overlap with the accessory included — is
                // used for both.
                .safeAreaInset(edge: .bottom, spacing: 0) {
                    // The reserved strip, and the way back RIDING ON IT.
                    //
                    // The button used to be an `.overlay(alignment: .bottom)`
                    // on the whole pane, padded up by `obstruction` — the same
                    // number said a second time, and correct only for as long
                    // as the pane's frame ended at the bottom of the screen.
                    // It stopped: the pane is inside a `NavigationStack` now
                    // (`ShellScreen.ShellPaneRealView.body`), whose content is
                    // laid out inside the framework's own keyboard inset, so
                    // "the bottom of the frame" is no longer where the composer
                    // is. Measured with a keyboard up: the button landed at
                    // y = −97, off the top of the display, and the test that
                    // taps it could not reach it.
                    //
                    // Hung off the strip instead, which IS the composer's room
                    // — one number, in one place, and the button cannot drift
                    // from the thing it is meant to sit above however the pane
                    // is hosted. The alignment guide puts the button's own
                    // BOTTOM one card above the strip's top edge; the strip
                    // stays exactly `obstruction` tall, so what the transcript
                    // reserves is unchanged.
                    Color.clear
                        .frame(height: obstruction)
                        .overlay(alignment: .top) {
                            if !pinnedToTail {
                                jumpToLatest
                                    .offset(y: -(Self.jumpDiameter + PaneMetrics.card))
                            }
                        }
                }
                // THE HOME INDICATOR, COUNTED ONCE.
                //
                // This is what left the last line under the composer even when
                // everything else was right. An input accessory is laid out in
                // the KEYBOARD's window, which runs to the bottom of the SCREEN
                // — so the height it measures already contains the
                // home-indicator strip below the card, and this pane's own
                // container safe area contains that same strip again. The two
                // added up reserved 196 points where the composer covers 162,
                // so the transcript came to rest 34 points too low and its last
                // row sat under the glass.
                //
                // Applied OUTSIDE the inset above, which is the whole trick:
                // the transcript is laid out against the bottom of the SCREEN
                // with a bottom safe area of nothing, and `obstruction` is then
                // the only thing put back. One number, taken from the one
                // window that knows where the composer actually is.
                //
                // `.all` rather than `.container`, and that half is load-
                // bearing too: SwiftUI's own keyboard avoidance is a bottom
                // safe area as well, and leaving it in place put the keyboard's
                // height on top of a number that already contained it — 645
                // points of inset for a keyboard covering 497.
                //
                // What it costs is that the transcript now draws into the strip
                // below the card, so a conversation being scrolled shows a band
                // of itself under the composer rather than clean ground. That
                // is what a floating composer does on this platform — it is a
                // card over content, not a bar with a floor — but it is a real
                // change and nobody has looked at it on a device.
                .ignoresSafeArea(.all, edges: .bottom)
                .animation(.spring(response: 0.28, dampingFraction: 0.86), value: pinnedToTail)
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
            #if DEBUG
            if let fixture = Self.fixture {
                stream.loadFixture(
                    fixture.events, phase: fixture.phase, waited: fixture.waited,
                    trouble: fixture.trouble)
                return
            }
            #endif
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
                        onSteer: { Task { await stream.steerQueued(queued.id) } },
                        hasAgent: hasAgent)
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
                        // Grey, beside an accent "Retry".
                        //
                        // Two accent controls in one banner is the row-of-blue
                        // pattern in miniature: Retry is what this row is FOR
                        // and keeps the color, and closing the row without
                        // sending anything is the incidental half. `.tint`
                        // rather than a foreground style on the glyph, because
                        // a `Button` tints its own label and a style set inside
                        // it does not survive — the finding `4458dcb` made in
                        // the composer and `af7d229` made again on the fleet's
                        // `…` menu.
                        Button {
                            stream.sendFailure = nil
                        } label: {
                            Image(systemName: "xmark")
                                .frame(
                                    width: PaneMetrics.target, height: PaneMetrics.target)
                                .contentShape(.rect)
                        }
                        .tint(.secondary)
                        .accessibilityLabel("Dismiss")
                    }
                    .padding(.horizontal, PaneMetrics.edge)
                    .padding(.vertical, PaneMetrics.tight)
                    .modifier(GlassSurface())
                }

                // The session under a conversation that is still on screen has
                // gone, and the pane says so where the eye already is.
                //
                // `emptyState` is where this fact is told with no rows, and it
                // cannot be where it is told with them: a transcript is the one
                // condition under which that view never draws. Keeping the rows
                // is deliberate — the daemon has forgotten this conversation,
                // so what is on screen is the only copy of it left, and
                // replacing it with an empty state would take a conversation
                // somebody is reading away to say something about it.
                //
                // What does not stay is the invitation. `AgentSupervisor::send`
                // drops a message for a pane with no shim registered and the
                // RPC still answers OK, so a Send that stayed live would echo
                // the message into the transcript looking sent and nothing
                // would ever answer it. Off, and said out loud, is the pair.
                //
                // The queue above is the same fact one surface up: its three
                // actions are three more `AgentSupervisor::send` calls, so
                // `QueuedRow` takes `hasAgent` too and this sentence is the
                // explanation for both.
                if !hasAgent, !transcript.rows.isEmpty {
                    HStack(spacing: PaneMetrics.step) {
                        // The mark `emptyState` gives this same fact, and
                        // secondary like it: a session that ended is not a
                        // failure, and red in this stack already means the
                        // message you just sent did not go.
                        Image(systemName: "bubble.left.and.text.bubble.right")
                            .foregroundStyle(.secondary)
                            // The sentence beside it says all of this, and a
                            // symbol read out as well would say it twice.
                            .accessibilityHidden(true)
                        Text("This pane no longer has an agent. The conversation stays here to read.")
                            .font(.footnote)
                            .foregroundStyle(.secondary)
                            .accessibilityIdentifier("agent-session-ended")
                        Spacer(minLength: 0)
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
                    // Only once a session has actually said. `Transcript`
                    // defaults `backend` to `acp` — correct for the
                    // transcripts written before the field existed, wrong for
                    // a pane nobody has heard from — so the badge is gated on
                    // the daemon's own epoch rather than on that default. See
                    // `AgentStream.hasSession`.
                    backend: stream.hasSession ? transcript.backend : nil,
                    hasAgent: hasAgent,
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
            // AND ROOM FOR THE SHELL'S BAR, WHICH THIS COMPOSER CANNOT SEE.
            //
            // The composer is an `inputAccessoryView`, so it is laid out in the
            // KEYBOARD's window (`DockedBar`); the shell's bar is laid out in
            // the app's. Neither window knows the other exists, and with the
            // keyboard DOWN they both want the same strip at the bottom of the
            // display. Measured on an iPhone 17: the composer's Send button
            // came out at y 776…820 and `shell-bar` at 784…828 — the "Message
            // Claude" box drawn straight over the workspace slider, which is
            // the owner's report.
            //
            // The bar is the half that must not move. `ShellRootView` keeps the
            // whole shell full height precisely so the bar and the track never
            // travel under a keyboard, and a bar that hopped whenever a
            // composer docked would be the one surface in the app you are meant
            // to put a thumb on moving out from under it. So the composer
            // yields: it clears the bar and the gap the bar floats in, and the
            // order is then fixed — bar at the bottom, composer above it,
            // always, in every pane and every state.
            //
            // Only while there is no keyboard behind it. With the keyboard up
            // the bar is already covered by the keyboard — that is the design,
            // not a bug — so the same padding there would open a 56-point strip
            // of nothing between the composer and the keys.
            //
            // `keyboardBehindTheBar` and NOT `keyboard.height <= barHeight`,
            // which is the version that was written first and hung the app.
            // The accessory posts its own keyboard-frame notification whenever
            // it changes height, so `keyboard.height` CONTAINS this padding —
            // a test against `barHeight` is a test against a number this line
            // moves, and the two settled into an oscillation 56 points wide
            // that never converged. `testTypingAMultiLineMessageMakesRoomForIt`
            // stopped finishing at all: the runner was killed rather than
            // failed, which is what a layout loop looks like from outside.
            .padding(.bottom, keyboardBehindTheBar ? 0 : Self.barClearance)
        }
    }

    /// How far the docked composer sits above the bottom of the display, so
    /// that the shell's bar is never underneath it.
    ///
    /// The bar's own height plus the gap it floats in — `ShellRootView`'s
    /// `safeArea.bottom + barGap` minus the safe area, which is the only part
    /// of that sum the keyboard's window already has. Named here rather than
    /// taken from the shell because a composer in the keyboard's window has no
    /// way to ask the shell anything; `AgentTranscriptScrollTests
    /// .testTheDockedComposerClearsTheShellsBar` is what keeps the two numbers
    /// honest, by measuring both rectangles rather than trusting this one.
    private static let barClearance: CGFloat = ShellMetrics.barRow + PaneMetrics.card

    /// Whether there is a real software keyboard under the composer, as
    /// opposed to the composer simply being docked.
    ///
    /// **A difference, and that is the whole point.** `keyboard.height` is the
    /// keyboard's overlap with the screen WITH the accessory inside it, and
    /// `barHeight` is that accessory — so both carry `barClearance` and the
    /// subtraction cancels it. What is left is the keys alone, which is the
    /// one quantity here that no layout on this screen can move. Any condition
    /// written on either number by itself is a condition that feeds back into
    /// the padding that produced it.
    ///
    /// The 100 points are slack, not a measurement. The two numbers are
    /// published by different notifications and can be one frame apart while
    /// the keyboard animates; the gap they have to tell apart is between
    /// nothing at all and the ~290 points of the shortest iPhone keyboard, so
    /// anything in the middle does.
    private var keyboardBehindTheBar: Bool {
        max(0, keyboard.height - barHeight) > 100
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
            // Start at the conversation's tail even when the transcript is too
            // long for the lazy stack to finish laying out before `onAppear`.
            // `.initialOffset` names the ONE role this is for: where the view
            // opens. It used to be set for every role, which handed the
            // framework a standing opinion about the bottom on top of the one
            // this view maintains deliberately below.
            .defaultScrollAnchor(.bottom, for: .initialOffset)
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
            // What the scroll view holds still while content around it changes
            // height — see the lazy stack above.
            .scrollPosition($scrollPosition, anchor: .bottom)
            // WHO is moving the scroll view — the question everything else
            // here depends on, asked of the framework rather than guessed at
            // from geometry. See principle 2 at the top of this file.
            .onScrollPhaseChange { _, phase, context in
                switch phase {
                // A finger, in one of its three states: down and still,
                // dragging, or coasting after a fling. Only these are the
                // reader.
                case .tracking, .interacting, .decelerating:
                    readerIsDriving = true
                // Ours. `anchorToTail` produces exactly this, and reading it
                // back as a decision the reader made is the feedback loop the
                // generation counter existed to break.
                case .animating:
                    break
                // The end of whichever it was. A fling that coasts to the
                // bottom finishes here rather than in the geometry callback,
                // so the verdict is taken here too.
                case .idle:
                    guard readerIsDriving else { break }
                    readerIsDriving = false
                    setPinned(Self.isAtTail(context.geometry))
                @unknown default:
                    break
                }
            }
            // Live while the finger is down, so the button appears the moment
            // the reader leaves the tail rather than when they let go — and
            // GATED on the finger, because this fires for every layout change
            // as well and those are not the reader scrolling.
            .onScrollGeometryChange(for: Bool.self) { Self.isAtTail($0) } action: { _, atBottom in
                guard readerIsDriving else { return }
                setPinned(atBottom)
            }
            // What the transcript actually inset itself by, published for the
            // probe. The regression this screen shipped was a wrong number
            // here that nothing on the outside could read.
            .onScrollGeometryChange(for: CGFloat.self) { $0.contentInsets.bottom } action: { _, inset in
                appliedInset = inset
            }
            // Keyed on the CURSOR, not the row count. A streamed reply
            // coalesces into the row already on screen, so the count does
            // not change while the text grows off the bottom.
            .onChange(of: transcript.cursor) { _, _ in
                guard pinnedToTail else {
                    // Not a nudge, not a flash — one dot on a control that is
                    // already on screen. See `jumpToLatest`.
                    arrivedWhileAway = true
                    return
                }
                // Unanimated: see principle 6. This fires several times a
                // second while a reply streams, and an eased scroll per event
                // is the jitter, not the smoothness.
                anchorToTail(animated: false, settling: false)
            }
            // THE VIEWPORT MOVED. The half that was missing.
            //
            // Only the keyboard was watched, and only for growth — so the
            // composer growing to three lines, an attachment strip appearing, a
            // plan panel arriving, or the bar simply finishing its first
            // measurement all changed how much of the transcript was covered
            // with nothing re-anchoring it. That last one is the reported bug:
            // the tail is parked against a bare viewport on the frame before
            // the accessory has measured, the inset then arrives, and the
            // scroll view keeps the offset it had — leaving the last rows
            // exactly one composer underneath the composer.
            //
            // Shrinking counts as much as growing: the keyboard going away
            // gives the tail-follower a hundred points of new room and it must
            // fill them rather than leave a gap it will never close.
            .onChange(of: obstruction) { _, _ in
                guard pinnedToTail else { return }
                anchorToTail(animated: false, settling: true)
            }
            // Deliberately nothing for the UNPINNED case, and it is a choice
            // rather than an omission. A reader parked mid-conversation keeps
            // their content offset, so every line they were reading stays
            // exactly where it was on screen and the keyboard rises over what
            // was already below the fold. Shifting the content up instead —
            // which is what a re-anchor would do — moves words under the eye of
            // somebody who is reading them, to save a scroll they can perform
            // themselves. Principle 4: unpinned is inviolable.
            .onAppear { anchorToTail(animated: false, settling: true) }
            .onDisappear { settleTask?.cancel() }
        }
    }

    /// A full-screen state, sized like one. `status` below owns the
    /// proportions, which are `FleetView.failure`'s — they were written out
    /// here when there was one state to draw and now there are several.
    ///
    /// The failure mark is red rather than amber. A session that would not
    /// load is a failure; amber in this app means an agent is waiting on you,
    /// and a screen that cannot show you an agent at all is not that.
    ///
    /// A LOADING SESSION MUST NOT WEAR A DEAD ONE'S SENTENCE, which is what
    /// this used to do for every state it had. There was one question here —
    /// is `connectionError` set — and two answers, so the screen went
    /// "Say something to begin." → red triangle → transcript, and was wrong at
    /// both of the first two. Before the first poll came back it invited you
    /// to type into a session it knew nothing about; a round trip later it
    /// called a shim that was still coming up a failure, and kept calling it
    /// one for as long as the shim took. `AgentStream.Phase` says why nothing
    /// on this screen is ever actually dead.
    ///
    /// Four honest states, and the two that are still trying change with how
    /// long they have been trying:
    ///
    /// - **Loading.** Nothing has come back. A spinner, and no claim either
    ///   way.
    /// - **Still trying.** Past `patience`, the spinner stops — a spinner that
    ///   never ends is its own bug — and a quiet mark takes over with a
    ///   sentence naming what is being waited on. Not red: this is a pane the
    ///   daemon has no agent for, which is what a pane that is not in agent
    ///   mode looks like too, and neither is a failure.
    /// - **Empty.** There IS a session and it has said nothing yet. The one
    ///   state "Say something to begin." was ever true for.
    /// - **Failed.** Only from `.failing`, and only past `alarm` — thirty
    ///   seconds of a poll that runs twice a second. This is where the red
    ///   triangle and the headline finally belong, and everything below the
    ///   headline is exactly what it always was.
    ///
    /// Android has the identical screen and the identical bug — the same
    /// sentence, under the same one-bit condition, at
    /// `apps/android/.../ui/AgentScreen.kt:170`. Not touched from here.
    @ViewBuilder
    private var emptyState: some View {
        VStack(spacing: 0) {
            Spacer()
            switch stream.phase {
            case .opening:
                // One round trip, usually. Deliberately says nothing about
                // whether a session exists, because nothing knows yet.
                status(spinner: true, title: "Loading this session…")

            case .starting:
                if stream.waited == .aMoment {
                    // The Mac's words, not a second set: `AgentComposer`
                    // draws "Starting the agent…" for a chat with no rows and
                    // no config options, which is this exact fact.
                    status(spinner: true, title: "Starting the agent…")
                } else {
                    // Still true, still not a failure, and no longer spinning.
                    //
                    // It promises nothing about what to do, because there is
                    // nothing honest to promise: `terminal.agent_prompt` hands
                    // the daemon a message for a shim, and `AgentSupervisor::
                    // send` drops it when no shim is connected — so "send
                    // something to start it" would be advice that does not
                    // work. What starts one is the pane going into agent mode.
                    status(
                        symbol: "bubble.left.and.text.bubble.right", mark: .secondary,
                        title: "No agent on this pane yet",
                        message:
                            "This pane hasn’t started one. The conversation "
                            + "appears here as soon as it does.")
                }

            case .failing:
                let trouble = stream.connectionError
                if stream.waited == .tooLong {
                    status(
                        symbol: "exclamationmark.triangle", mark: .red,
                        title: "Could not load this session",
                        // The core's own words, below a sentence rather than
                        // standing in for one. Under this headline, in this
                        // face, they used to read as Far Cooler's account of
                        // the runner; they are not, and they are also the only
                        // account anybody debugging an unreachable runner is
                        // going to get, so they stay.
                        message: trouble?.sentence, transcript: trouble?.transcript)
                } else if stream.waited == .aWhile {
                    status(
                        symbol: "arrow.clockwise", mark: .secondary,
                        title: "Still trying", message: trouble?.sentence)
                } else {
                    // A poll that did not come back is not news yet — there is
                    // another one 700ms behind it. The sentence goes under the
                    // spinner so the screen is not silent about it either.
                    status(
                        spinner: true, title: "Loading this session…",
                        message: trouble?.sentence)
                }

            case .live:
                Text("Say something to begin.")
                    .font(.callout)
                    .foregroundStyle(.secondary)
                    // The same identifier `status` gives its headline. This
                    // sentence IS this state's headline, and one name for
                    // "what is this screen claiming" is what lets the states
                    // be asserted as a set rather than one at a time.
                    .accessibilityIdentifier("agent-empty-title")
            }
            Spacer()
        }
        .padding(.horizontal, 32)
        .frame(maxWidth: .infinity)
    }

    /// One full-screen state, composed the one way this app composes them.
    ///
    /// Lifted verbatim from `TerminalView.status`, which took it from
    /// `FleetView.failure`: a 42-point thin mark or a `.large` spinner, 22
    /// points, a `.title2` headline, 8 points, a `.callout` sentence capped at
    /// 320. `message` is prose this app wrote; `transcript` is what the host
    /// said, and the two are drawn as different kinds of thing on purpose —
    /// a lowercase fragment from an ssh channel set in a callout under a
    /// headline reads as Far Cooler's own account of the pane.
    ///
    /// It was already this composition here, written out inline for the one
    /// state that existed. Now several states draw it, and several copies of
    /// it is how the proportions drift apart.
    private func status(
        spinner: Bool = false, symbol: String? = nil, mark: Color = .secondary, title: String,
        message: String? = nil, transcript: String? = nil
    ) -> some View {
        VStack(spacing: 0) {
            if spinner {
                // The spinner is the mark in this state, so it is sized like
                // one rather than left at the 20pt a row would use.
                ProgressView()
                    .controlSize(.large)
                    .padding(.bottom, 22)
            } else if let symbol {
                Image(systemName: symbol)
                    .font(.system(size: 42, weight: .thin))
                    .foregroundStyle(mark)
                    .padding(.bottom, 22)
            }
            Text(title)
                .font(.title2.weight(.semibold))
                .multilineTextAlignment(.center)
                .padding(.bottom, PaneMetrics.step)
                // Named rather than combined into one element. Combining
                // swallowed the identifier — a synthesized element does not
                // keep it — and it would also have folded the host's own words
                // in the box below into this app's sentence, which is the one
                // thing everything about this composition exists to keep apart.
                .accessibilityIdentifier("agent-empty-title")
            if let message {
                Text(message)
                    .font(.callout)
                    .foregroundStyle(.secondary)
                    .multilineTextAlignment(.center)
                    .frame(maxWidth: 320)
                    .accessibilityIdentifier("agent-empty-message")
            }
            if let transcript, !transcript.isEmpty {
                DetailBox(text: transcript)
                    .frame(maxWidth: 320)
                    .padding(.top, 14)
            }
        }
    }
}

/// Debug-only layout telemetry for the real-device keyboard regression.
/// Release builds keep the stable identifier but do not expose implementation
/// measurements as a VoiceOver value.
private struct AgentLayoutProbe: ViewModifier {
    let keyboardHeight: CGFloat
    let barHeight: CGFloat
    /// What the transcript actually inset itself by. The regression this probe
    /// was written for turned out to be a wrong number HERE, which nothing
    /// outside the view could read — the two heights above were both correct.
    let appliedInset: CGFloat
    let followingTail: Bool
    /// Whether there is a transcript here at all.
    ///
    /// OFF OVER THE EMPTY STATE, and that is an accessibility fix rather than
    /// tidiness. An accessibility VALUE on a container makes the container
    /// itself the element, which hides everything inside it — so with no rows,
    /// the headline, the sentence and the host's own output collapsed into one
    /// node whose entire exposed text was `keyboard=0;bar=162;inset=162;
    /// tail=true`. VoiceOver could not read the screen, and neither could a UI
    /// test: `agent-empty-title` was findable under `simctl` and invisible to
    /// XCUITest, which is how this was found.
    ///
    /// There is also nothing to measure there. Every number this publishes is
    /// a scroll view's, and the empty state is not one.
    let measuring: Bool

    @ViewBuilder
    func body(content: Content) -> some View {
        if measuring {
            #if DEBUG
            content
                .accessibilityIdentifier("agent-transcript")
                .accessibilityValue(
                    Text(verbatim:
                        "keyboard=\(Int(keyboardHeight.rounded()));bar=\(Int(barHeight.rounded()));inset=\(Int(appliedInset.rounded()));tail=\(followingTail ? "true" : "false")"))
            #else
            content.accessibilityIdentifier("agent-transcript")
            #endif
        } else {
            content
        }
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
    /// Whether there is an agent on the other end — see `AgentView.hasAgent`.
    ///
    /// All THREE of this row's actions need it, which was checked in the Rust
    /// rather than assumed. `terminal.agent_steer_queued`,
    /// `terminal.agent_edit_queued` and `terminal.agent_cancel_queued` are one
    /// `svc.agents().send(…)` each in `crates/daemon/src/rpc.rs`, and that call
    /// looks the terminal up in the supervisor's writer table and returns
    /// having done nothing when it is not there — the same floor
    /// `terminal.agent_prompt` drops a message onto.
    ///
    /// Remove is the one worth naming, because a queue you can still empty
    /// would be a fair thing to leave live and it is not one. The queue is not
    /// this app's; it lives in the shim, beside the backend, and reaches a
    /// client only as `promptQueue` — which `Transcript` applies WHOLESALE,
    /// never editing the list itself. So with the shim gone the row is a
    /// picture of a queue that no longer exists anywhere: Remove would send
    /// into the same floor, no `promptQueue` would ever answer, and the row it
    /// was meant to delete would still be sitting there.
    let hasAgent: Bool

    @State private var editing = false
    @State private var draft = ""

    /// Editing needs somewhere to send the result, so a pane that loses its
    /// agent mid-edit leaves the field rather than keeping a `Save` that
    /// saves nothing. Read instead of `editing` everywhere, because `@State`
    /// set before the session went is still `true`.
    private var isEditing: Bool { editing && hasAgent }

    var body: some View {
        HStack(alignment: .top, spacing: PaneMetrics.step) {
            Spacer(minLength: 40)
            VStack(alignment: .trailing, spacing: PaneMetrics.tight) {
                // `.body` in all three arms. A queued message is the reader's
                // own words waiting to be sent, so it is set at the size the
                // transcript sets them — and this row used to hold `.callout`
                // and `.body` in the SAME row, one step apart, depending on
                // whether what you queued had any words in it.
                if isEditing {
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
                //
                // The correction to THAT was a full accent on all three, and
                // it is what the owner photographed: "Send now", "Edit" and
                // "Remove" side by side in bright blue on a dark card, which
                // is the same failure with the contrast turned up. Three
                // equally loud words say nothing about which one you want,
                // and `FleetList` already states the rule this row broke —
                // accent is spent per screen, not per control.
                //
                // So the weight carries "these do something" — it is what
                // separates them from the `.caption` regular "Queued" beside
                // them — and the color carries "this is the one worth
                // finding", which is `onSteer` and only `onSteer`. Editing a
                // draft and dropping a draft are both things you came here
                // already meaning to do; interrupting a running turn is the
                // one this card exists to offer. See `QueuedActionStyle`.
                //
                // And all three GO when the pane's agent does. Not greyed:
                // greying is what the composer does with Send, and it earns it
                // — the field beside it still holds your draft, so a dead
                // button is the thing keeping your words on screen. Nothing
                // here is holding anything. Three unreadable words in a row
                // are the "grey on grey" complaint in miniature, and they
                // would be saying, at their most legible, exactly what the
                // notice a few points below already says.
                //
                // What replaces them is one true word. "Queued" is a promise
                // about the future — this goes next — and on a pane with no
                // shim it is a promise nothing can keep, so the label says
                // what actually happened to the message instead. See
                // `hasAgent` for why none of the three would have worked.
                HStack(spacing: PaneMetrics.card) {
                    Text(hasAgent ? "Queued" : "Not sent")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .accessibilityIdentifier("agent-queued-state")
                    if hasAgent {
                        Button("Send now", action: onSteer)
                            .buttonStyle(QueuedActionStyle(prominent: true))
                        Button(isEditing ? "Save" : "Edit") {
                            if isEditing {
                                commit()
                            } else {
                                draft = queued.text
                                editing = true
                            }
                        }
                        Button("Remove", action: onCancel)
                    }
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
        // `hasAgent` asked again here rather than only where the button is
        // drawn, for the reason the composer's `send()` re-asks its own
        // condition: a hardware Return reaches `onSubmit` without passing a
        // button at all.
        guard hasAgent, !trimmed.isEmpty, trimmed != queued.text else { return }
        onEdit(trimmed)
    }
}

/// One of the queue's actions: a word you can tap, drawn as one.
///
/// Semibold, which is what this file already uses for an action standing in
/// prose — the composer's "Retry" — rather than a fifth grey word in a row of
/// grey words. The 44-point band is the hit target a 16-point caption never
/// had, and the `contentShape` is what makes it live: padding around a
/// `Button`'s label is layout only.
///
/// **Weight says "control"; color says "this one".** This style used to paint
/// every label `.tint`, which on a queued card is three accent words in a row
/// — the thing the owner photographed. `prominent` is now what spends the
/// accent, and exactly one of the three asks for it.
///
/// `Color.accentColor` and `Color.secondary` rather than `.tint` and a
/// hierarchical style, for the reason `FleetList` gives about its own `+`: a
/// hierarchical style resolves against whatever foreground is in force, so
/// `.secondary` under a tinted control is a paler accent and not grey at all.
/// A custom `ButtonStyle` does not tint its label, so both of these reach —
/// and if a future edit puts a tint back over this row, it goes grey rather
/// than blue, which is the safe direction to fail in.
private struct QueuedActionStyle: ButtonStyle {
    /// Whether this is the one action on the card worth finding by color.
    var prominent = false

    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(.caption.weight(.semibold))
            .foregroundStyle(prominent ? Color.accentColor : Color.secondary)
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
///
/// **And it had drifted a second time, in the half nobody could see.** That
/// fix put `.frame(maxWidth: .infinity)` on the buttons, which widened the
/// boxes and not the capsules — so what shipped was still two intrinsic pills,
/// merely centred in the halves instead of bunched at the left. Reasoning from
/// the code found neither round of this. `AgentLayoutHarness` now draws the
/// card under `-approval`, which is where both were finally seen.
struct ApprovalControls: View {
    let options: [PermissionOption]
    let onChoose: (String) -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: PaneMetrics.step) {
            HStack(spacing: PaneMetrics.step) {
                // `maxWidth` on the LABEL, not on the button.
                //
                // Seen at last, in the harness: the sentence above about
                // full-width was still not true. A frame around a `Button`
                // whose style draws its own capsule stretches the button's
                // box; the capsule inside it keeps its intrinsic width and
                // sits centred in the space, which is what shipped — two
                // ordinary pills floating in a row twice their width, on the
                // one card where a thumb has to hit the right half the first
                // time. Stretching the label is what stretches the capsule.
                if let allow {
                    Button { onChoose(allow.id) } label: {
                        Text(allow.name).frame(maxWidth: .infinity)
                    }
                    .buttonStyle(.borderedProminent)
                    .frame(maxWidth: .infinity, minHeight: PaneMetrics.target)
                }
                if let reject {
                    // NOT a second blue.
                    //
                    // `.bordered` paints its label in the tint, so Reject was
                    // accent text on a translucent grey capsule directly
                    // beside Allow's solid accent fill: two blues, one of them
                    // the hardest thing on the card to read, in the one place
                    // this app cannot afford a misread. Allow carries the
                    // color because it is the answer that lets something
                    // happen; Reject carries the same capsule and the ordinary
                    // label color, which is how a decision reads as two equal
                    // choices rather than as one recommendation and one link.
                    //
                    // Both said, in this order, on purpose. `.tint` is what
                    // reliably reaches a system button style; the foreground
                    // style is what lifts the label off the greyed tint to
                    // full label contrast. If the second one is ever
                    // overridden the button goes grey, not back to blue.
                    Button { onChoose(reject.id) } label: {
                        Text(reject.name).frame(maxWidth: .infinity)
                    }
                    .buttonStyle(.bordered)
                    .tint(.secondary)
                    .foregroundStyle(.primary)
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

    @State private var diffRowWidth: CGFloat = 0

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
                    .background(
                        GeometryReader { geometry in
                            Color.clear.preference(
                                key: DiffRowWidth.self, value: geometry.size.width)
                        }
                    )
                    // As wide as the widest row, so a run of added lines is one
                    // band rather than a staircase. See `DiffRowWidth`.
                    .frame(minWidth: diffRowWidth, alignment: .leading)
                    // `in: .rect`, stated rather than defaulted, for the reason
                    // `ChangesView.DiffLineRow` states it: a bare
                    // `.background(_ style:)` takes its shape from the container
                    // it sits in, and this one sits in a 6pt rounded box — so
                    // every row was drawing that box's corners at its own
                    // bottom edge.
                    .background(line.kind.background, in: .rect)
                }
            }
            .onPreferenceChange(DiffRowWidth.self) { diffRowWidth = $0 }
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
    /// Which protocol is carrying this conversation — `acp`, `claude` or
    /// `codex` — or nil until a session has said which.
    ///
    /// Straight off `Transcript.backend`, which is `SessionStarted.backend` on
    /// the wire. Transcribed from the producers rather than assumed: the field
    /// is declared in `crates/agent-core/src/event.rs` as a plain `String`
    /// with `#[serde(default = "acp_backend")]` and NO `skip_serializing_if`,
    /// so it is always present going out; the three writers are
    /// `crates/acp/src/session.rs`, `crates/claude/src/backend.rs` and
    /// `crates/codex/src/backend.rs`, each passing `BackendKind::as_str()`,
    /// which is exactly `"acp"`, `"claude"` or `"codex"`. It reaches a phone
    /// verbatim: `ffi.rs` copies `payload_json` into the `payloadJson` string
    /// this app decodes and touches nothing inside it.
    let backend: String?
    /// Whether there is an agent on the other end to send to.
    ///
    /// False only where the daemon has SAID there is none — see
    /// `AgentView.hasAgent`. `AgentSupervisor::send` drops a message for a pane
    /// with no shim registered and `terminal.agent_prompt` answers OK anyway,
    /// so a live Send there is a button that files the message into the
    /// transcript and nothing else. The row above the composer says why it is
    /// off; this is the half that keeps the draft in the field.
    let hasAgent: Bool
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
        guard hasAgent else { return false }
        return !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || !attachments.isEmpty
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
                    // A chip, like everything else in this row.
                    //
                    // It was a bare glyph centered in a 26-point box, which
                    // put its ink four and a half points inside the left edge
                    // every other thing in this card starts on — visible
                    // precisely because the message field sits directly below
                    // it — and made the one control that adds something to a
                    // message read as a different KIND of thing from the four
                    // capsules beside it. Same capsule, same height, same
                    // 44-point band around it as `settingsMenu`.
                    PhotosPicker(selection: $photoPickerItem, matching: .images) {
                        Image(systemName: "photo.badge.plus")
                            .font(.system(size: 15))
                            .foregroundStyle(.secondary)
                            .padding(.horizontal, PaneMetrics.step)
                            .frame(height: PaneMetrics.chip)
                            .background(Capsule().fill(.quaternary))
                            .frame(minHeight: PaneMetrics.target)
                            .contentShape(.capsule)
                    }
                    .onChange(of: photoPickerItem) { _, item in loadPickedPhoto(item) }

                    ForEach(inlineOptions) { option in
                        inlineSelector(option)
                    }

                    settingsMenu

                    Spacer(minLength: PaneMetrics.step)

                    adapterBadge
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

                // Bottom-aligned, so a field grown to four lines keeps Send
                // beside its LAST line rather than floating halfway up the
                // box. What that alone could not do is the resting state: an
                // empty field is one 26-point line and Send is a 44-point
                // target, so bottom-aligning them put the glyph's centre nine
                // points above the text's. The field carries the same
                // 44-point minimum now, and a single line sits centred in it,
                // which is what puts the two on one line.
                HStack(alignment: .bottom, spacing: PaneMetrics.step) {
                    fieldWithPlaceholder
                        .frame(minHeight: PaneMetrics.target, alignment: .leading)

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
                    // Named, for VoiceOver and for the tests: a glyph-only
                    // button is read out as its symbol otherwise, and "arrow up
                    // circle fill" is not what this does.
                    .accessibilityLabel("Send")
                    .accessibilityIdentifier("agent-send")
                }
            }
            .padding(.vertical, PaneMetrics.card)
        }
        // ONE left edge inside the card, said once — the same correction
        // `composerStack` already made for the surfaces stacked above it, and
        // it had to be made twice because it was three different numbers in
        // here: 16 on the two control rows, 12 on the suggestion list and the
        // attachment strip, and nothing at all on the attachment error, which
        // therefore ran into a 22-point corner. Four things in one card at
        // four left edges is most of what "everything is not aligned" was.
        .padding(.horizontal, PaneMetrics.edge)
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

    // MARK: Which protocol this is

    /// Which protocol is carrying this chat, at the far end of the row that
    /// says what the next message costs.
    ///
    /// HERE because this surface has no header to put it in. The Mac draws it
    /// in the pane's title bar beside the pane's name (`TileView.headerContent`)
    /// and a phone's agent pane deliberately has no such bar — "Nothing here is
    /// new permanent chrome" is the constraint at the top of this file, and it
    /// is the reason the mode and the attachments live in the composer at all.
    /// So the nearest true equivalent of "beside the name" is the composer:
    /// the placeholder directly below this reads "Message Claude", and the
    /// badge above it finishes that sentence with which protocol Claude is on.
    /// It is also the one piece of chrome that is on screen for the whole life
    /// of this surface, including while the transcript is still empty — which
    /// is exactly when somebody asking "why is this behaving oddly" is looking.
    ///
    /// PAST THE SPACER, and with no capsule, because it is not a control. Every
    /// other thing in this row is: a bordered chip here would be the fifth in a
    /// line of four tappable ones and would promise a menu it does not have.
    ///
    /// The WORDS are the Mac's, and the rule for choosing between them is the
    /// Mac's: anything that is not `acp` is a native backend. "ACP" stays
    /// capitalised — it is an acronym, Agent Client Protocol, and lowercasing
    /// it makes a proper noun look like a status word.
    ///
    /// The COLOR is not the Mac's, and that is a platform decision rather than
    /// drift. `TileView` tints Native with the accent color; on this row that
    /// would be blue text on dark glass, in the one row this file explicitly
    /// de-blued — see the `.tint(.secondary)` below it, which exists because a
    /// `Menu` tinting its own label made this line read as a row of links.
    /// Same emphasis, one step up the platform's own hierarchy instead: Native
    /// is the only primary text in the row, ACP sits with the chips.
    ///
    /// The Mac's `.help` text is a real explanation and a phone has no hover,
    /// so it becomes the VoiceOver label. That is the only place it fits
    /// without turning a label into a button.
    @ViewBuilder
    private var adapterBadge: some View {
        if let backend, !backend.isEmpty {
            let native = backend != "acp"
            Text(native ? "Native" : "ACP")
                .font(.caption2.weight(.medium))
                .foregroundStyle(native ? AnyShapeStyle(.primary) : AnyShapeStyle(.secondary))
                // Never the thing that gets truncated: the selectors beside it
                // carry agent-chosen names of any length, and this is four to
                // six characters.
                .fixedSize()
                .layoutPriority(1)
                .accessibilityLabel(
                    native
                        ? "Native: driven through \(backend)’s own protocol, with no adapter"
                        : "ACP: driven through an Agent Client Protocol adapter")
                .accessibilityIdentifier("adapter-badge")
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
                // No horizontal inset of its own: the card says the edge
                // once now. The vertical gap stays, because it is the space
                // between the thumbnails and whatever is above them.
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
        // The button's own condition, asked again here rather than only on
        // `.disabled`: a hardware Return also lands in this function, and it
        // reaches it past a greyed button. See `hasAgent`.
        guard canSend else { return }
        let message = text.trimmingCharacters(in: .whitespacesAndNewlines)
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
                        // Horizontal inset comes from the card, so a
                        // suggestion's glyph starts on the same column as the
                        // message being typed under it.
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
/// The two surface constants are the phone's, not this screen's: `ShellBar`
/// and `TerminalKeyRow` already draw at 22 and 10, and a composer floating two
/// points inside a glass bar with corners curving at half the rate is what made
/// three surfaces on one screen read as three different objects.
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

/// One floating surface's glass, in one place.
///
/// Every floating thing in this app goes through here rather than reaching for
/// `glassEffect` itself, so there is exactly one opinion about what glass is
/// and a shape can never end up backed by a slightly different material than
/// the one beside it.
struct GlassSurface: ViewModifier {
    var radius: CGFloat = PaneMetrics.surfaceRadius
    /// Whether this surface is a thing you TOUCH, and should react like one.
    ///
    /// iOS 26's glass has a reaction built into it — it scales, bounces and
    /// lights up at the point of contact — and it is off unless a surface asks
    /// for it, because it is a statement about affordance rather than a
    /// finish: glass that flexes under a finger is glass the finger can do
    /// something to. A strip you drag says yes; a panel that merely floats
    /// over a transcript says no, and turning it on there would promise a
    /// control that is not there.
    ///
    /// Not for `Button`s. The standard button styles already carry the same
    /// reaction, and a button drawn on an interactive surface reacts twice.
    var interactive: Bool = false

    func body(content: Content) -> some View {
        content.glassEffect(
            interactive ? .regular.interactive() : .regular, in: .rect(cornerRadius: radius))
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

#if DEBUG
/// The agent pane, with a canned conversation and no runner behind it.
///
/// Exists because this screen could not be LOOKED at: reaching it needs an
/// enrolled runner, a workspace, a chat-capable pane and a turn in flight, so
/// every judgement about its layout had been made by reading the code. Launch
/// the app with `-agent-layout-harness` to get this instead of `RootView`.
///
/// ## It mounts the pane where the app mounts it
///
/// This used to construct an `AgentView` and put it on screen BARE — no shell,
/// no bar, no ribbon — and the app has never once shown the pane that way.
/// What the harness drew instead was a transcript running up under the status
/// bar, with a tool-call row crossing the clock, and screenshots from here are
/// how this app's iOS layout gets judged: a harness that is wrong about the TOP
/// of the screen is a trap for the next person measuring something at the
/// bottom.
///
/// So the route is the app's route, top to bottom — and the app's route is the
/// shell now. `ShellScreen` over a canned `Connection`, `ShellRootView`,
/// `ShellPaneTrack`, the real `TerminalView`, the real `AgentView`. That is a
/// stronger claim than the pushed `WorkspaceView` this used to mount, because
/// the shell is what tells a pane where the furniture is: `ShellPaneSlot.chrome`
/// is the top and bottom inset the conversation has to clear, and it is
/// measured from the bar rather than from a navigation bar that no longer
/// exists. A harness that skipped it would be measuring the wrong two numbers.
///
/// Everything above the pane is the shipping code reading a canned fleet, which
/// is the only part that is fixture — `Connection.standIn(on:)` and
/// `AgentView.fixture` are the two seams, and neither of them draws anything.
///
/// ## What is not the app, and says so
///
/// The words in the bar. It carries the workspace's name, so the fixture names
/// itself there — "Layout harness" — rather than borrowing a plausible worktree
/// name and leaving somebody to find out later that no runner was involved. The
/// glass around it is real; only the words are canned.
///
/// The runner menu in the overview's toolbar stands on an empty `RunnerStore`
/// and says "No Runner", which is the truth about this launch.
///
/// The Diff tab shows no unread ring: that comes from `Connection.inbox`, which
/// is a separate round trip this stands nothing in for.
struct AgentLayoutHarness: View {
    static var isRequested: Bool {
        CommandLine.arguments.contains("-agent-layout-harness")
    }

    @StateObject private var connection = Connection()
    /// No runners, and that is the fixture. The menu is real and its sheets
    /// open; there is simply nothing on this device to switch to.
    @StateObject private var hosts = RunnerStore()

    /// The pane to open on, as the shell's own deep-link request.
    ///
    /// Set once `stand()` has filled the fixture in. The shell would land on
    /// this pane anyway — `PaneFocus.rule` picks the top-ranked agent and the
    /// fixture has one — but going through the request is the app's own path
    /// for "open exactly this pane", so the harness cannot start passing
    /// because the rule happened to agree.
    @State private var open: String?

    var body: some View {
        ShellScreen(connection: connection, hosts: hosts, pendingTerminal: $open)
            .task { stand() }
    }

    /// Fill the fixture in, then point the shell at the pane.
    ///
    /// In this order, and in a `task` rather than in `body`: the fixture has to
    /// exist before `AgentView` mounts and reads it, and writing to an
    /// `ObservableObject` from inside a view update is a change published into
    /// the update that is reading it.
    private func stand() {
        AgentView.fixture = Self.fixture
        connection.standIn(
            on: Fleet(runtimeHealthy: true, livePanes: 2, workspaces: [Self.workspace]))
        open = Self.agentPane.id
    }

    /// The canned pane, and the fleet it lives in.
    ///
    /// A chat-capable agent pane and a shell beside it, because the ribbon and
    /// the column are drawn from the workspace's terminals and the content
    /// swipe walks between them — a one-terminal fixture would have made every
    /// screenshot a workspace with two tabs where the app usually has three or
    /// four, and left the swipe with nowhere to go.
    ///
    /// The activity is the fleet's, not a flag on the pane: `AgentView.isWorking`
    /// reads it from here the way it does in the app, so a fixture that says a
    /// turn is running and a pane drawing "Working…" are one fact rather than
    /// two that can disagree. Only the live conversation is working: a pane
    /// with no session behind it — every `-empty-` screen, and `-ended` — is
    /// not running a turn, and `idle` is the least any of them claims.
    private static var agentPane: Terminal {
        Terminal(
            id: "harness", short: "harness", title: "claude", preset: "claude", state: "running",
            activity: emptyState == nil && !isEnded ? "working" : "idle",
            epoch: 1, paneMode: "agent", chatCapable: true)
    }

    /// The conversation whose session has gone out from under it.
    private static var isEnded: Bool { CommandLine.arguments.contains("-ended") }

    private static var workspace: Workspace {
        Workspace(
            id: "harness-ws", short: "harness",
            // The two words that say what this is, in the two slots the app
            // puts a worktree's name and branch in.
            task: "Layout harness", branch: "fixture · no runner", state: "ready",
            terminals: [
                agentPane,
                Terminal(
                    id: "harness-shell", short: "shell", title: "shell", preset: "shell",
                    state: "running", epoch: 1, paneMode: "terminal"),
            ])
    }

    /// Which empty-transcript screen to stand in, or nil for the conversation.
    ///
    /// One BARE flag each — `-empty-starting` — rather than `-state starting`.
    /// The pair form worked under `simctl launch` and silently did nothing
    /// under `XCUIApplication.launchArguments`, where a leading-dash argument
    /// followed by a value is the NSUserDefaults argument domain's own syntax
    /// and does not arrive as two arguments. `-plain` and `-native` were
    /// already bare for no reason but taste; now there is a reason.
    ///
    /// Every one of these is a screen that used to be reachable only by owning
    /// a runner whose shim was slow or whose link was down, which is most of
    /// why they were all drawn the same and nobody could see that they were.
    private static var emptyState:
        (AgentStream.Phase, AgentStream.Waited, AgentStream.Trouble?)?
    {
        let args = CommandLine.arguments
        if args.contains("-empty-opening") { return (.opening, .aMoment, nil) }
        if args.contains("-empty-starting") { return (.starting, .aMoment, nil) }
        if args.contains("-empty-waiting") { return (.starting, .aWhile, nil) }
        if args.contains("-empty-trying") {
            return (
                .failing, .aWhile,
                AgentStream.Trouble(
                    sentence: "The connection to this runner dropped. Reconnecting…")
            )
        }
        if args.contains("-empty-failed") {
            return (
                .failing, .tooLong,
                AgentStream.Trouble(
                    sentence: "The request that reads it didn’t finish.",
                    transcript: "ssh: connect to host runner port 22: Operation timed out")
            )
        }
        if args.contains("-empty-live") { return (.live, .aMoment, nil) }
        return nil
    }

    /// The conversation, and the state it is in.
    ///
    /// The pane no longer takes the container treatment this used to apply —
    /// `.ignoresSafeArea(.keyboard, edges: .bottom)`, added here to reproduce
    /// what the pane host does so the keyboard was not counted twice.
    /// `ShellPaneRealView` is on screen now and does it itself, which is the
    /// point of mounting the shell: nothing about the container is reproduced,
    /// so nothing about it can drift.
    private static var fixture: AgentView.Fixture {
        if let (phase, waited, trouble) = emptyState {
            return AgentView.Fixture(events: [], phase: phase, waited: waited, trouble: trouble)
        }
        // `-ended` is the conversation whose session has gone: rows on screen
        // and the daemon answering epoch 0, which is what a shim that goes away
        // mid-conversation leaves behind. `loadFixture` gives a non-`.live`
        // phase epoch 0, so this is the daemon's own answer rather than a
        // fourth thing to set by hand — see `AgentStream.answered`.
        let phase: AgentStream.Phase =
            CommandLine.arguments.contains("-ended") ? .starting : .live
        let events = CommandLine.arguments.contains("-plain")
            ? conversation.filter {
                if case .plan = $0.event { return false }
                if case .promptQueue = $0.event { return false }
                return true
            }
            : conversation
        return AgentView.Fixture(events: events, phase: phase)
    }

    private static func choice(_ id: String, _ name: String) -> AgentChoice {
        AgentChoice(id: id, name: name)
    }

    private static func selector(
        _ id: String, _ name: String, _ current: String, _ options: [(String, String)]
    ) -> ConfigOption {
        ConfigOption(
            id: id, name: name, description: "", category: id, kind: "select",
            currentValue: current, options: options.map { choice($0.0, $0.1) })
    }

    /// Long enough to overflow the screen, so the bottom inset is testable.
    private static var conversation: [Sequenced] {
        var events: [AgentEvent] = [
            .sessionStarted(
                sessionID: "harness", agentMode: "manual",
                availableModes: [choice("manual", "Manual"), choice("auto", "Auto")],
                model: "sonnet", availableModels: [],
                configOptions: [
                    selector("mode", "Mode", "manual", [("manual", "Manual"), ("auto", "Auto")]),
                    selector("model", "Model", "sonnet", [("sonnet", "Sonnet"), ("opus", "Opus")]),
                    selector("effort", "Effort", "high", [("high", "High"), ("low", "Low")]),
                    selector(
                        "thought_level", "Thinking", "normal",
                        [("normal", "Normal"), ("deep", "Deep")]),
                ],
                // `-native` to see the other badge. The value is a real
                // `BackendKind::as_str()` string, not a fixture spelling —
                // `crates/agent-core/src/backend.rs` writes exactly these
                // three, and the badge's rule is "anything that is not acp".
                availableCommands: [choice("review", "review")],
                backend: CommandLine.arguments.contains("-native") ? "claude" : "acp"),
            .plan(entries: [
                PlanEntry(content: "Read the failing test", priority: "high", status: "completed"),
                PlanEntry(content: "Fix the inset", priority: "high", status: "in_progress"),
                PlanEntry(content: "Run the build", priority: "medium", status: "pending"),
            ]),
        ]
        for turn in 1...4 {
            events.append(
                .message(
                    role: .user,
                    text: "Turn \(turn): the bottom of this transcript has to stay readable.",
                    parent: nil))
            events.append(
                .message(
                    role: .agent,
                    text: """
                        Answer \(turn). This paragraph exists to push the conversation past \
                        the height of the screen so the last line can be checked against the \
                        composer resting over it. A line that comes to rest underneath the \
                        glass is the bug being looked for.
                        """,
                    parent: nil))
            events.append(
                .toolCall(
                    id: "tool-\(turn)", title: "Read AgentView.swift", kind: "read",
                    status: .completed, locations: [], parent: nil, subagent: false))
        }
        events.append(
            .message(
                role: .agent,
                text: "This is the LAST line of the transcript. It must be fully readable.",
                parent: nil))
        events.append(.promptQueue(items: [QueuedPrompt(id: "q1", text: "And then run the tests")]))
        // `-approval` stands the whole screen on a blocked turn instead.
        //
        // The card is unreachable from every other fixture — a permission
        // arrives from a runner mid-turn and nothing here can ask for one —
        // which is why `ApprovalControls` has twice been reasoned about from
        // the code and twice been wrong about what shipped. One turn rather
        // than four, so the card and the composer are on screen together and
        // the decision row can be read against the ground it actually sits on.
        if CommandLine.arguments.contains("-approval") {
            events = Array(events.prefix(5))
            events.append(
                .permission(
                    id: "perm-1", toolCall: "tool-1",
                    options: [
                        PermissionOption(id: "allow", name: "Allow", kind: "allow_once"),
                        PermissionOption(id: "reject", name: "Reject", kind: "reject_once"),
                        PermissionOption(
                            id: "allow-always", name: "Allow always for this session",
                            kind: "allow_always"),
                    ]))
        }
        return events.enumerated().map { Sequenced(seq: UInt64($0.offset + 1), event: $0.element) }
    }
}
#endif
