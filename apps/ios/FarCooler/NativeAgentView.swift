import SwiftUI

/// A terminal-mode claude or codex pane with its conversation beside it
/// (ov-373, ov-416): both
/// mounted, one shown, switched on the phone alone.
///
/// The terminal is always the first child and never leaves the tree, so a
/// switch never respawns the pane, never restarts its stream and works
/// mid-turn and mid-dialog; the conversation keeps its draft and rows in its
/// `NativePaneModel`, which outlives this view. The switch, in the pane's
/// bar, is there only where the conversation is offered: the runner serves
/// rows and compose (`AgentConversation.served`), and the pane is claude in
/// a terminal, or codex where the runner says so (`codex_view`). Anywhere
/// else this is the terminal and nothing more. Never a blank pane.
struct NativeSwitch<Content: View>: View {
    let terminal: Terminal
    @ObservedObject var connection: Connection
    /// Whether this pane is the one on screen.
    let isVisible: Bool
    /// The conversation was switched to: the terminal gives up the keyboard.
    let toConversation: () -> Void
    @ViewBuilder let content: () -> Content

    @Environment(\.scenePhase) private var scenePhase

    /// The pane's model, where the conversation is offered.
    ///
    /// Gated on the build the LAYOUT reads (`daemon ?? lastDaemon`), never
    /// on `daemon` alone: every reconnect clears `daemon` until the new
    /// link's `host` answers, and a view gated on it came down for that round
    /// trip, showed the terminal and raised its keyboard, then came back
    /// (ov-373 review 1). Sends still go through the runner's own gate.
    private var model: NativePaneModel? {
        guard AgentConversation.served(by: build),
            AgentConversation.isAgentInATerminal(paneMode: terminal.paneMode, preset: agent, build: build),
            AgentConversation.isRunning(state: terminal.state)
        else { return nil }
        return NativePanes.shared.model(for: terminal.id, core: connection.core, preset: agent)
    }

    /// The agent the pane runs, by which the view is offered: never
    /// `preset`, which for a claude that named its session is the title
    /// (ov-443).
    private var agent: String {
        AgentConversation.agent(running: terminal.runningAgent, program: terminal.program, preset: terminal.preset)
    }

    /// Why a Claude or Codex pane isn't offered the view, for its dimmed
    /// switch (ov-443); nil where it is, and for any other pane.
    private var unavailable: AgentConversation.Unavailable? {
        let reason = AgentConversation.unavailable(
            paneMode: terminal.paneMode, agent: agent, running: AgentConversation.isRunning(state: terminal.state),
            offered: build?.capabilities, projectorOn: connection.projectorOn)
        return reason?.isAboutTheRunner == true ? reason : nil
    }

    /// Whether this device may turn the runner's setting on: the reason says so
    /// when it may not.
    private var maySetSetting: Bool { build?.grantedScope == "host_admin" }

    /// The build the layout reads: see `model`.
    private var build: DaemonBuild? { connection.daemon ?? connection.lastDaemon ?? connection.knownBuild }

    var body: some View {
        let model = model
        NativeSwitchBody(
            terminal: terminal.id, model: model, unavailable: model == nil ? unavailable?.sentence(maySetSetting: maySetSetting) : nil, offer: NativeOffer(build),
            isOnScreen: isVisible && scenePhase == .active, isVisible: isVisible, toConversation: toConversation, content: content)
    }
}

/// What the runner's hello offers the composer beyond rows and a way to send
/// (ov-404): line breaks, photos and commands with `compose`; Stop and Send
/// Now with `terminal_interrupt`.
struct NativeOffer: Equatable {
    var rich = false
    var interrupts = false
    /// Bring Here, with `bring_draft` (ov-369).
    var bring = false
    /// A subagent opened to its own rows, with `subagent_rows` (ov-453).
    var agents = false
    /// A prompt's images, with `agent_images` (ov-458).
    var images = false

    init(_ build: DaemonBuild?) {
        rich = build?.can(.compose) == true
        interrupts = build?.can(.terminalInterrupt) == true
        bring = build?.can(.bringDraft) == true
        agents = build?.can(.subagentRows) == true
        images = build?.can(.agentImages) == true
    }
}

/// `NativeSwitch` with its model resolved, observed where there is one.
private struct NativeSwitchBody<Content: View>: View {
    let terminal: String
    let model: NativePaneModel?
    /// Why the conversation isn't offered, where a dimmed switch says so.
    let unavailable: String?
    let offer: NativeOffer
    let isOnScreen: Bool
    let isVisible: Bool
    let toConversation: () -> Void
    let content: () -> Content

    /// The model's `showing`, mirrored so this view, which holds the
    /// terminal, redraws when it changes. Seeded from the model, so the
    /// first frame already knows the terminal is covered (`ArrivalFocus`).
    @State private var showing: Bool
    /// The model this view last followed through, so one that stops being
    /// offered (the setting turned off, claude exited) stops its follow and
    /// starts afresh when it's offered again.
    @State private var held: NativePaneModel?
    /// The dimmed switch's reason is on screen.
    @State private var explaining = false

    init(
        terminal: String, model: NativePaneModel?, unavailable: String? = nil, offer: NativeOffer,
        isOnScreen: Bool, isVisible: Bool, toConversation: @escaping () -> Void, content: @escaping () -> Content
    ) {
        self.terminal = terminal
        self.model = model
        self.unavailable = unavailable
        self.offer = offer
        self.isOnScreen = isOnScreen
        self.isVisible = isVisible
        self.toConversation = toConversation
        self.content = content
        _showing = State(initialValue: model?.showing ?? false)
        _held = State(initialValue: model)
    }

    var body: some View {
        ZStack {
            // The terminal first, always: the conversation coming and going
            // leaves its identity, and so its stream and its grid, alone.
            NativeMountProbe {
                VStack(spacing: 0) { content() }
            }
            // Out of sight and out of VoiceOver's reach while the
            // conversation covers it, and still mounted. That alone keeps
            // VoiceOver in the conversation: the `.isModal` trait this once
            // added made XCUI (and VoiceOver) read the composer and the
            // bars around it as alerts (ov-680).
            .opacity(showing ? 0 : 1)
            .accessibilityHidden(showing)
            .environment(\.terminalCovered, showing)
            if let model {
                NativeAgentView(model: model, showTerminal: { model.wantsConversation = false })
                    .opacity(showing ? 1 : 0)
                    .allowsHitTesting(showing)
                    .accessibilityHidden(!showing)
            }
        }
        #if DEBUG
        // Which of the two shows, for the UI tests: both stay in the
        // accessibility tree XCUITest reads, so neither's presence says.
        // `left` counts the times the conversation stopped showing, and
        // `dropped` the times it stopped being offered.
        .overlay(alignment: .bottomTrailing) {
            Color.clear
                .frame(width: 1, height: 1)  // style-exempt: DEBUG probe: a 1 pt element the UI tests read, nothing drawn
                .accessibilityElement()
                .accessibilityIdentifier("native-showing")
                .accessibilityValue(
                    (model == nil ? "terminal-only" : (showing ? "conversation" : "terminal"))
                        + " left=\(NativeProbe.left[terminal] ?? 0) dropped=\(NativeProbe.dropped[terminal] ?? 0)")
        }
        #endif
        .toolbar {
            if let model, isVisible, !model.unavailable {
                ToolbarItem(placement: .topBarTrailing) {
                    Button {
                        model.wantsConversation.toggle()
                    } label: {
                        Image(systemName: showing ? "terminal" : "text.bubble")
                    }
                    .inBarScheme()
                    .accessibilityLabel(showing ? "Show Terminal" : "Show Conversation")
                    .accessibilityIdentifier("native-switch")
                }
            } else if let unavailable, isVisible {
                // Dimmed, and a tap says why (ov-443).
                ToolbarItem(placement: .topBarTrailing) {
                    Button {
                        explaining = true
                    } label: {
                        Image(systemName: "text.bubble").foregroundStyle(.tertiary)
                    }
                    .inBarScheme()
                    .accessibilityLabel("Conversation Unavailable")
                    .accessibilityValue(unavailable)
                    .accessibilityIdentifier("native-switch-unavailable")
                }
            }
        }
        .alert("Conversation Unavailable", isPresented: $explaining) {
            Button("OK", role: .cancel) {}
        } message: {
            Text(unavailable ?? "")
        }
        .task(id: model.map(ObjectIdentifier.init)) {
            guard let model else {
                showing = false
                return
            }
            for await value in model.$showing.values { showing = value }
        }
        .onChange(of: showing) { was, now in
            NativePanes.shared.cover(terminal, now)
            if now { toConversation() }
            #if DEBUG
            if was, !now { NativeProbe.left[terminal, default: 0] += 1 }
            #endif
        }
        .onChange(of: isOnScreen, initial: true) { _, now in model?.setOnScreen(now) }
        .onChange(of: offer, initial: true) { _, now in
            model?.offer(rich: now.rich, interrupts: now.interrupts, bring: now.bring, agents: now.agents, images: now.images)
        }
        .onChange(of: model.map(ObjectIdentifier.init)) { _, _ in
            #if DEBUG
            if held != nil, model == nil { NativeProbe.dropped[terminal, default: 0] += 1 }
            #endif
            if let held, held !== model { held.release() }
            held = model
            model?.offer(rich: offer.rich, interrupts: offer.interrupts, bring: offer.bring, agents: offer.agents, images: offer.images)
            model?.setOnScreen(isOnScreen)
        }
        .onAppear {
            NativePanes.shared.cover(terminal, showing)
            model?.setOnScreen(isOnScreen)
        }
        // A pane closed, or the shell left: no `isVisible` change says so,
        // and the model outlives the view, so its follow would go on holding
        // a call on the runner.
        .onDisappear {
            NativePanes.shared.cover(terminal, false)
            model?.setOnScreen(false)
        }
    }
}

#if DEBUG
/// How many times the conversation moved to its tail, and why the last time
/// (ov-626): `jumps=N last=<reason>`.
@MainActor
enum NativeScrollProbe {
    static var count = 0
    static var last = "none"

    /// The latest scroll events, `<ms>:<what>`, for finding out what moved
    /// the list (ov-675).
    static var trail: [String] = []
    private static let began = Date()

    static func jumped(_ reason: String) {
        count += 1
        last = reason
        note("J\(reason)")
    }

    /// Rows that changed height by more than a few points after they were
    /// laid out, newest last, `id:was>now` (ov-675): on a row nobody touched,
    /// a second layout pass that moves every row under it.
    static var resizes: [String] = []

    static func resized(_ id: String, from was: CGFloat, to now: CGFloat) {
        guard abs(now - was) > 8 else { return }
        resizes.append("\(id):\(Int(was))>\(Int(now))")
        if resizes.count > 12 { resizes.removeFirst() }
        if abs(now - was) > 50 { note("H\(id):\(Int(was))>\(Int(now))") }
    }

    static func note(_ what: String) {
        trail.append("\(Int(Date().timeIntervalSince(began) * 1000) % 100_000):\(what)")
        if trail.count > 40 { trail.removeFirst() }
    }
}

/// What the UI tests read about a pane's switch.
@MainActor
enum NativeProbe {
    static var left: [String: Int] = [:]
    /// The times the conversation stopped being offered.
    static var dropped: [String: Int] = [:]
}

/// The times the composer's field had its own non-empty text replaced by a
/// different non-empty draft, for the UI tests: a typed key lost to a draft
/// one key behind would count here, where a key the field never got would
/// not (CI's third-letter drops, ov-470 investigation). A Bring Here
/// counts too: it puts the box's words ahead of what was typed.
@MainActor
final class NativeComposerProbe: ObservableObject {
    static let shared = NativeComposerProbe()
    @Published private(set) var overwrites = 0

    /// Counted after the update that saw it, so publishing never happens
    /// inside one.
    func overwrote() {
        DispatchQueue.main.async { self.overwrites += 1 }
    }
}
#endif

extension EnvironmentValues {
    /// The pane's conversation covers its terminal (ov-373).
    @Entry var terminalCovered = false
}

/// A terminal's keyboard on arrival, and on its return from under the
/// conversation, but never while the conversation covers it: keys typed
/// there would go to a terminal nobody can see.
struct ArrivalFocus: ViewModifier {
    let focus: () -> Void
    @Environment(\.terminalCovered) private var covered

    func body(content: Content) -> some View {
        content
            .onAppear { if !covered { focus() } }
            .onChange(of: covered) { _, now in if !now { focus() } }
    }
}

/// Which mount of the terminal under the switch this is, for a UI test to
/// tell a terminal kept mounted from one built again (`native-terminal-mount`).
private struct NativeMountProbe<Content: View>: View {
    @ViewBuilder let content: () -> Content
    #if DEBUG
    @StateObject private var mount = NativeMount()
    #endif

    var body: some View {
        content()
            #if DEBUG
            .overlay(alignment: .bottomLeading) {
                Color.clear
                    .frame(width: 1, height: 1)  // style-exempt: DEBUG probe: a 1 pt element the UI tests read, nothing drawn
                    .accessibilityElement()
                    .accessibilityIdentifier("native-terminal-mount")
                    .accessibilityValue("mount=\(mount.serial)")
            }
            #endif
    }
}

#if DEBUG
private final class NativeMount: ObservableObject {
    private static var built = 0
    let serial: Int

    init() {
        Self.built += 1
        serial = Self.built
    }
}
#endif

/// The conversation view of a terminal-mode claude pane (ov-373): its rows
/// as the runner's projector folds them, newest at the bottom, and a box to
/// type into.
///
/// A lazy list keyed by row id, so a long session builds only the rows on
/// screen and a row's change re-renders that row alone. Older rows are paged
/// in as the top comes into view.
///
/// It follows like Messages, on the rules `AgentView` set out for ov-383:
/// following the tail is the reader's mode, only a finger changes it, a
/// pinned list re-anchors whenever its content or its room changes, an
/// unpinned one is never moved except by the reader's own send, and the way
/// back is a button.
struct NativeAgentView: View {
    @ObservedObject var model: NativePaneModel
    let showTerminal: () -> Void

    /// Whether the list is following its own tail.
    @State private var pinned = true
    /// Whether the scroll in progress is one the reader started.
    @State private var readerIsDriving = false
    @State private var position = ScrollPosition(idType: String.self)
    @State private var settle: Task<Void, Never>?
    /// What keeps the reader's row where it is while older rows load above
    /// it (ov-629), and whether the rows are reporting their places: while
    /// the store has older rows to come (`moreBefore`), and for the moment
    /// after they arrive, until the row held has been still.
    @State private var place = PlaceHold()
    @State private var watchesPlace = false
    #if DEBUG
    /// Where the visible part of the list starts, for the UI tests (ov-626).
    @State private var shownTop: CGFloat = 0
    /// How far the visible part of the list ends above the content's end,
    /// however it got there, for the UI tests (ov-675).
    @State private var shownGap: CGFloat = 0
    #endif
    /// The screen's one margin (ov-611): rows, tray and composer share it.
    @Environment(\.screenMargin) private var margin
    @Environment(\.scenePhase) private var scenePhase
    /// The height between the bars above and the keyboard (or the screen's
    /// foot) below: what the tray and the composer share (ov-614).
    @State private var room: CGFloat = .infinity
    /// The list's height from the bars above to the screen's foot (its
    /// visible height and its bottom inset, which holds the tray, the
    /// composer and the keyboard), and the keyboard's: their difference is
    /// `room`. The tray and the composer move height between the list and
    /// its inset, never changing the sum, so their sizes can't feed back.
    @State private var underBars: CGFloat = 0
    @State private var keyboard: CGFloat = 0
    /// The window this view is in, for the keyboard's overlap.
    @State private var hostWindow = WindowReader.Holder()

    private static let end = "native-end"
    /// How near the end still counts as on it: the list's bottom padding,
    /// which the end marker sits above, and a little for rounding.
    private static let tailSlack: CGFloat = Spacing.section + 8
    /// When to look again after the room changed: the keyboard animates over
    /// several layout passes (`AgentView.settleLadder`).
    private static let settleLadder = [80, 180, 300, 440]

    var body: some View {
        // A subagent opened from the tray or its row (ov-453) takes the
        // conversation's place, in the same style, with the way back above.
        if let opened = model.drill.opened {
            NativeAgentConversation(model: model, opened: opened, showTerminal: showTerminal)
        } else {
            conversation
        }
    }

    private var conversation: some View {
        let store = model.store
        return ScrollView {
            LazyVStack(alignment: .leading, spacing: Spacing.inset) {
                if store.moreBefore {
                    ProgressView()
                        .frame(maxWidth: .infinity)
                        .onAppear { model.loadOlder() }
                }
                let last = store.items.last?.id
                let answer = model.nativeAskAnswer(viewer: connection.talk.viewer)
                let sendNow: (() -> Void)? = model.offersSendNow ? { Task { await model.sendNow() } } : nil
                // A run of tool calls is one line (ov-452).
                ForEach(store.items) { item in
                    switch item {
                    case .row(let id):
                        if let box = store.box(id) {
                            NativeRowView(
                                box: box, isLast: id == last, showTerminal: showTerminal, answer: answer, sendNow: sendNow)
                                .modifier(NativePlaceMarker(id: id, watching: watchesPlace, moved: placeMoved, gone: place.gone))
                        }
                    case .tools(let id, let rows):
                        NativeToolGroupRow(id: id, boxes: rows.compactMap(store.box))
                            .modifier(NativePlaceMarker(id: id, watching: watchesPlace, moved: placeMoved, gone: place.gone))
                    }
                }
                ForEach(model.queued.indices, id: \.self) { i in
                    QueuedLine(text: model.queued[i], sendNow: sendNow)
                }
                if model.issue == .handoff {
                    HandoffRow(reason: AgentConversation.handoff(model.agent), showTerminal: showTerminal)
                } else if model.issue == .panel {
                    HandoffRow(reason: AgentConversation.panel(model.agent), showTerminal: showTerminal)
                }
                // An anchor rather than the last row's id: the last row
                // changes in place while it streams.
                Color.clear.frame(height: 1).id(Self.end)  // style-exempt: a 1 pt scroll anchor, nothing drawn
            }
            .padding(.horizontal, margin)
            .padding(.vertical, Spacing.section)
        }
        .defaultScrollAnchor(.bottom, for: .initialOffset)
        // Rows go on up under the bars above (the navigation bar, and the
        // orchestrator's cards in their `safeAreaBar`) and show through
        // them, softened by the system's edge effect, as in Messages
        // (ov-678). `.hard` read more plainly behind the title but sliced
        // rows at the bar's foot.
        .scrollEdgeEffectStyle(.soft, for: .top)
        // A session shorter than the screen sits on the composer, as in
        // Messages, not at the top with a gap under it.
        .defaultScrollAnchor(.bottom, for: .alignment)
        .scrollDismissesKeyboard(.interactively)
        .scrollPosition($position, anchor: .bottom)
        .onScrollPhaseChange { _, phase, context in
            #if DEBUG
            NativeScrollProbe.note("P\(phase)g\(Int(shownGap))")
            #endif
            switch phase {
            case .tracking, .interacting, .decelerating:
                readerIsDriving = true
                // Older rows that arrive now wait for the scroll to rest, so
                // they never go in under a moving finger (ov-629).
                model.store.scrolling()
                #if DEBUG
                if CommandLine.arguments.contains("-native-live-older-on-scroll") { Task { await NativeLiveRows.olderGate.open() } }
                #endif
                place.release(oldest: model.store.ids.first)
            case .idle:
                model.store.scrollEnded()
                guard readerIsDriving else { break }
                readerIsDriving = false
                pinned = Self.isAtTail(context.geometry)
            default:
                break
            }
        }
        .onScrollGeometryChange(for: [CGFloat].self) { [$0.contentOffset.y + $0.contentInsets.top, $0.contentInsets.top] } action: { _, now in
            // A correction waiting behind the last one goes now it has landed.
            #if DEBUG
            place.inset = now[1]
            #endif
            if model.store.scrollInProgress { model.store.scrolling() }
            if let target = place.landed(at: now[0]) { holdTo(.offset(target)) }
        }
        .onChange(of: store.moreBefore, initial: true) { _, more in
            if more { watchesPlace = true }
        }
        .onScrollGeometryChange(for: Bool.self) { Self.isAtTail($0) } action: { _, atTail in
            guard readerIsDriving else { return }
            pinned = atTail
        }
        // The content grew (a row arrived, a reply streamed on) or the room
        // changed (the keyboard, the composer growing): a pinned list stays
        // on its tail. Unpinned, nothing moves.
        .onScrollGeometryChange(for: CGFloat.self) { $0.contentSize.height } action: { was, now in
            #if DEBUG
            if abs(now - was) > 50 { NativeScrollProbe.note("S\(Int(now - was))") }
            #endif
            guard pinned, !readerIsDriving else { return }
            anchor(settling: false, for: "content")
        }
        .onScrollGeometryChange(for: CGFloat.self) { $0.containerSize.height } action: { _, height in
            place.viewport = height
            guard pinned, !readerIsDriving else { return }
            anchor(settling: true, for: "room")
        }
        // A message just sent comes into view, wherever the reader was, as
        // in Messages (ov-383): what they sent is what they're looking for.
        .onChange(of: model.sent) { _, _ in
            pinned = true
            anchor(settling: true, for: "sent")
        }
        .onAppear { anchor(settling: true, for: "appear") }
        // An app sent to the background mid-drag, or a call or a sheet that
        // cancels the gesture, never gets its `.idle`: rows must not wait on it.
        .onChange(of: scenePhase) { _, now in
            if now != .active {
                readerIsDriving = false
                model.store.scrollEnded()
            }
        }
        .onDisappear {
            settle?.cancel()
            // A scroll that never reports its end mustn't hold rows back.
            model.store.scrollEnded()
        }
        .accessibilityIdentifier("native-transcript")
        #if DEBUG
        // For the UI tests only: VoiceOver mustn't read it.
        .accessibilityValue(probe)
        .onScrollGeometryChange(for: CGFloat.self) { $0.visibleRect.minY.rounded() } action: { _, now in
            shownTop = now
        }
        .onScrollGeometryChange(for: CGFloat.self) {
            ($0.contentSize.height - ($0.visibleRect.maxY - $0.contentInsets.bottom)).rounded()
        } action: { was, now in
            shownGap = now
            // A big move, whoever made it: what pulled a reader down (ov-675).
            if abs(now - was) > 60 { NativeScrollProbe.note("G\(Int(was))>\(Int(now))") }
        }
        #endif
        .overlay {
            if store.shownIds.isEmpty { emptyState(store.phase) }
        }
        .overlay(alignment: .bottomTrailing) {
            if !pinned {
                Button {
                    pinned = true
                    withAnimation(.easeOut(duration: 0.25)) { position.scrollTo(id: Self.end, anchor: .bottom) }
                } label: {
                    Image(systemName: "chevron.down")
                        .fontWeight(.semibold)
                        .frame(width: 38, height: 38)
                }
                .buttonStyle(.glass)
                .accessibilityLabel("Jump to Latest")
                .accessibilityIdentifier("native-jump-to-latest")
                .padding(.horizontal, margin)
                .padding(.vertical, Spacing.section)
            }
        }
        .safeAreaInset(edge: .top, spacing: 0) {
            // Rows held and the runner not answering: said over them, so
            // stale rows never pass for live ones, and the box waits
            // (`canSend`).
            if !store.shownIds.isEmpty, store.isStale { staleBanner(store.phase) }
        }
        .safeAreaInset(edge: .bottom, spacing: 0) {
            // The agents at work are the Agents card's (ov-619), over the
            // conversation, not a tray under it.
            NativeComposer(model: model, showTerminal: showTerminal, room: room)
        }
        .environment(\.promptImages, model.promptImages)
        // A long press on a word selects it, with handles (ov-609).
        .environment(\.selectsRanges, true)
        .environment(\.nativeOpenAgent, model.opensAgents ? { model.openAgent(row: $0, agentId: $1) } : nil)
        .background(Surface.contentFill.ignoresSafeArea())
        .onChange(of: store.ids.count) { _, _ in model.settleQueued() }
        .onChange(of: store.phase) { _, _ in model.phaseChanged() }
        .onScrollGeometryChange(for: CGFloat.self) { $0.containerSize.height + $0.contentInsets.bottom } action: { _, now in
            underBars = now
            settleRoom()
        }
        .background { WindowReader(holder: hostWindow).frame(width: 0, height: 0).accessibilityHidden(true) }
        .onReceive(NotificationCenter.default.publisher(for: UIResponder.keyboardWillChangeFrameNotification)) { note in
            // In this view's own window's coordinates, so an iPad's Split View,
            // Slide Over or Stage Manager window, or a second scene, measures
            // its own overlap; a floating keyboard, clear of the window's
            // foot, covers nothing.
            guard let end = note.userInfo?[UIResponder.keyboardFrameEndUserInfoKey] as? CGRect,
                let window = hostWindow.window
            else { return }
            let frame = window.convert(end, from: window.screen.coordinateSpace)
            let docked = frame.maxY >= window.bounds.maxY - 1
            keyboard = docked ? max(window.bounds.maxY - frame.minY, 0) : 0
            settleRoom()
        }
    }

    #if DEBUG
    /// What the list says of itself, for the UI tests.
    private var probe: String {
        let store = model.store
        let head = "tail=\(pinned) rows=\(store.shownIds.count) following=\(model.following) "
        let jumps = "jumps=\(NativeScrollProbe.count) last=\(NativeScrollProbe.last) top=\(Int(shownTop)) gap=\(Int(shownGap)) "
        let hold = "inset=\(Int(place.inset)) hold=\(store.heldCount)/\(place.probe) trace=\(place.trace.joined(separator: ",")) "
        let resized = "resized=\(NativeScrollProbe.resizes.joined(separator: ",")) "
        return head + jumps + hold + resized + "events=\(NativeScrollProbe.trail.joined(separator: ","))"
    }
    #endif

    /// `room`, in whole points, moved only by a change of more than one:
    /// the inset's layout rounds, and a room that followed every half point
    /// would lay the tray and composer out again for each.
    private func settleRoom() {
        let now = max((underBars - keyboard).rounded(), 0)
        if !room.isFinite || abs(now - room) > 1 { room = now }
    }

    /// A row is at `y` in the scroll view: kept, and where older rows came in
    /// above the rows the list had, scrolled back to (ov-629).
    private func placeMoved(_ id: String, _ y: CGFloat) {
        let store = model.store
        let move = place.report(
            id, y: y, oldest: store.ids.first, kept: { store.ids.contains($0) }, following: pinned,
            driving: readerIsDriving)
        if let move { holdTo(move) } else { restartSettle() }
    }

    /// Move the list to keep the held row, and wait for it to be still.
    private func holdTo(_ move: PlaceHold.Move) {
        switch move {
        case .offset(let target): position.scrollTo(y: target)
        case .row(let id): position.scrollTo(id: id, anchor: .top)
        }
        restartSettle()
    }

    private func restartSettle() {
        guard place.isHolding else { return }
        let store = model.store
        place.settle(after: 0.8) {
            place.release(oldest: store.ids.first)
            watchesPlace = store.moreBefore
        }
    }

    private static func isAtTail(_ geometry: ScrollGeometry) -> Bool {
        // The visible rect runs on under the tray and the composer (the bottom
        // inset): measured, 141 pt past the content's end at the tail. Counted
        // as visible, a reader had to go up the inset's height and two lines
        // more before the list let them go, and anything less was taken back
        // to the bottom by the next streamed line (ov-626).
        geometry.visibleRect.maxY - geometry.contentInsets.bottom >= geometry.contentSize.height - tailSlack
    }

    private func anchor(settling: Bool, for reason: String) {
        settle?.cancel()
        settle = nil
        jump(reason)
        guard settling else { return }
        settle = Task { @MainActor in
            for delay in Self.settleLadder {
                try? await Task.sleep(for: .milliseconds(delay))
                guard !Task.isCancelled, pinned, !readerIsDriving else { return }
                jump("\(reason)-settle")
            }
        }
    }

    /// Every move to the tail goes through here, and in a debug build says
    /// why, for the UI tests (ov-626).
    private func jump(_ reason: String) {
        #if DEBUG
        NativeScrollProbe.jumped(reason)
        #endif
        position.scrollTo(id: Self.end, anchor: .bottom)
    }

    private func staleBanner(_ phase: AgentRowStore.Phase) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: Spacing.group) {
            Text(phase == .unavailable
                ? "This session isn’t being read anymore. The terminal has it."
                : "Can’t reach the runner, so this may be out of date. Trying again…")
            Spacer(minLength: Spacing.group)
            Button("Show Terminal", action: showTerminal)
        }
        .font(.callout)
        .padding(.horizontal, Spacing.inset)
        .padding(.vertical, Spacing.group)
        .attentionSurface(in: .card)
        .surface(.content, in: .card)
        .padding(.horizontal, margin)
        .padding(.top, Spacing.group)
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("native-stale")
    }

    @ViewBuilder
    private func emptyState(_ phase: AgentRowStore.Phase) -> some View {
        switch phase {
        case .loading, .cached: ProgressView()
        case .live: Text("Nothing in this session yet.").foregroundStyle(.quietInk)
        case .unavailable: Text("This pane’s session can’t be shown here. Use the terminal.").foregroundStyle(.quietInk)
        case .trouble: Text("Can’t reach the runner. Trying again…").foregroundStyle(.quietInk)
        }
    }
}

/// Hands the window its view is in to a holder, weakly: a SwiftUI view has no
/// window of its own to ask.
struct WindowReader: UIViewRepresentable {
    final class Holder {
        weak var window: UIWindow?
    }

    let holder: Holder

    func makeUIView(context: Context) -> Probe {
        let view = Probe()
        view.holder = holder
        return view
    }

    func updateUIView(_ view: Probe, context: Context) { view.holder = holder }

    final class Probe: UIView {
        var holder: Holder?

        override func didMoveToWindow() {
            super.didMoveToWindow()
            holder?.window = window
        }
    }
}
