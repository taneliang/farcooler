import SwiftUI

/// A terminal-mode claude pane with its conversation beside it (ov-373): both
/// mounted, one shown, switched on the phone alone.
///
/// The terminal is always the first child and never leaves the tree, so a
/// switch never respawns the pane, never restarts its stream and works
/// mid-turn and mid-dialog; the conversation keeps its draft and rows in its
/// `NativePaneModel`, which outlives this view. The switch, in the pane's
/// bar, is there only where the conversation is offered: the runner serves
/// rows and compose (`AgentConversation.served`), and the pane is claude in
/// a terminal. Anywhere else this is the terminal and nothing more. Never a
/// blank pane.
struct NativeSwitch<Content: View>: View {
    let terminal: Terminal
    @ObservedObject var connection: Connection
    /// Whether this pane is the one on screen.
    let isVisible: Bool
    /// The conversation was switched to: the terminal gives up the keyboard.
    let toConversation: () -> Void
    @ViewBuilder let content: () -> Content

    @Environment(\.scenePhase) private var scenePhase

    private var model: NativePaneModel? {
        guard AgentConversation.served(by: connection.knownBuild),
            AgentConversation.isClaudeInATerminal(paneMode: terminal.paneMode, preset: terminal.preset)
        else { return nil }
        return NativePanes.shared.model(for: terminal.id, core: connection.core)
    }

    var body: some View {
        let model = model
        NativeSwitchBody(
            model: model, isOnScreen: isVisible && scenePhase == .active, isVisible: isVisible,
            toConversation: toConversation, content: content)
    }
}

/// `NativeSwitch` with its model resolved, observed where there is one.
private struct NativeSwitchBody<Content: View>: View {
    let model: NativePaneModel?
    let isOnScreen: Bool
    let isVisible: Bool
    let toConversation: () -> Void
    let content: () -> Content

    /// The model's `showing`, mirrored so this view, which holds the
    /// terminal, redraws when it changes.
    @State private var showing = false

    var body: some View {
        ZStack {
            // The terminal first, always: the conversation coming and going
            // leaves its identity, and so its stream and its grid, alone.
            NativeMountProbe {
                VStack(spacing: 0) { content() }
            }
            // Out of sight and out of VoiceOver's reach while the
            // conversation covers it, and still mounted.
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
        .overlay(alignment: .bottomTrailing) {
            Color.clear
                .frame(width: 1, height: 1)  // style-exempt: DEBUG probe: a 1 pt element the UI tests read, nothing drawn
                .accessibilityElement()
                .accessibilityIdentifier("native-showing")
                .accessibilityValue(model == nil ? "terminal-only" : (showing ? "conversation" : "terminal"))
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
                    .accessibilityLabel(showing ? "Show Terminal" : "Show Conversation")
                    .accessibilityIdentifier("native-switch")
                }
            }
        }
        .task(id: model.map(ObjectIdentifier.init)) {
            guard let model else {
                showing = false
                return
            }
            for await value in model.$showing.values { showing = value }
        }
        .onChange(of: showing) { _, now in
            if now { toConversation() }
        }
        .onChange(of: isOnScreen, initial: true) { _, now in model?.setOnScreen(now) }
        .onChange(of: model.map(ObjectIdentifier.init)) { _, _ in model?.setOnScreen(isOnScreen) }
    }
}

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

    private static let end = "native-end"
    private static let tailSlack: CGFloat = 40
    /// When to look again after the room changed: the keyboard animates over
    /// several layout passes (`AgentView.settleLadder`).
    private static let settleLadder = [80, 180, 300, 440]

    var body: some View {
        let store = model.store
        ScrollView {
            LazyVStack(alignment: .leading, spacing: Spacing.inset) {
                if store.moreBefore {
                    ProgressView()
                        .frame(maxWidth: .infinity)
                        .onAppear { model.loadOlder() }
                }
                let last = store.ids.last
                ForEach(store.ids, id: \.self) { id in
                    if let box = store.box(id) {
                        NativeRowView(box: box, isLast: id == last, showTerminal: showTerminal)
                    }
                }
                ForEach(model.queued.indices, id: \.self) { i in
                    QueuedLine(text: model.queued[i])
                }
                if model.issue == .handoff {
                    HandoffRow(reason: AgentConversation.handoff, showTerminal: showTerminal)
                }
                // An anchor rather than the last row's id: the last row
                // changes in place while it streams.
                Color.clear.frame(height: 1).id(Self.end)  // style-exempt: a 1 pt scroll anchor, nothing drawn
            }
            .padding(Spacing.section)
        }
        .defaultScrollAnchor(.bottom, for: .initialOffset)
        .scrollDismissesKeyboard(.interactively)
        .scrollPosition($position, anchor: .bottom)
        .onScrollPhaseChange { _, phase, context in
            switch phase {
            case .tracking, .interacting, .decelerating:
                readerIsDriving = true
            case .idle:
                guard readerIsDriving else { break }
                readerIsDriving = false
                pinned = Self.isAtTail(context.geometry)
            default:
                break
            }
        }
        .onScrollGeometryChange(for: Bool.self) { Self.isAtTail($0) } action: { _, atTail in
            guard readerIsDriving else { return }
            pinned = atTail
        }
        // The content grew (a row arrived, a reply streamed on) or the room
        // changed (the keyboard, the composer growing): a pinned list stays
        // on its tail. Unpinned, nothing moves.
        .onScrollGeometryChange(for: CGFloat.self) { $0.contentSize.height } action: { _, _ in
            guard pinned, !readerIsDriving else { return }
            anchor(settling: false)
        }
        .onScrollGeometryChange(for: CGFloat.self) { $0.containerSize.height } action: { _, _ in
            guard pinned, !readerIsDriving else { return }
            anchor(settling: true)
        }
        // A message just sent comes into view, wherever the reader was, as
        // in Messages (ov-383): what they sent is what they're looking for.
        .onChange(of: model.sent) { _, _ in
            pinned = true
            anchor(settling: true)
        }
        .onAppear { anchor(settling: true) }
        .onDisappear { settle?.cancel() }
        .accessibilityIdentifier("native-transcript")
        .accessibilityValue(String("tail=\(pinned) rows=\(store.ids.count) following=\(model.following)"))
        .overlay {
            if store.ids.isEmpty { emptyState(store.phase) }
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
                .padding(Spacing.section)
            }
        }
        .safeAreaInset(edge: .top, spacing: 0) {
            // Rows held and the runner not answering: said over them, so
            // stale rows never pass for live ones, and the box waits
            // (`canSend`).
            if !store.ids.isEmpty, store.isStale { staleBanner(store.phase) }
        }
        .safeAreaInset(edge: .bottom, spacing: 0) {
            NativeComposer(model: model, showTerminal: showTerminal)
        }
        .background(Surface.contentFill.ignoresSafeArea())
        .onChange(of: store.ids.count) { _, _ in model.settleQueued() }
        .onChange(of: store.phase) { _, _ in model.phaseChanged() }
    }

    private static func isAtTail(_ geometry: ScrollGeometry) -> Bool {
        geometry.visibleRect.maxY >= geometry.contentSize.height - tailSlack
    }

    private func anchor(settling: Bool) {
        settle?.cancel()
        settle = nil
        position.scrollTo(id: Self.end, anchor: .bottom)
        guard settling else { return }
        settle = Task { @MainActor in
            for delay in Self.settleLadder {
                try? await Task.sleep(for: .milliseconds(delay))
                guard !Task.isCancelled, pinned, !readerIsDriving else { return }
                position.scrollTo(id: Self.end, anchor: .bottom)
            }
        }
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
        .padding(.horizontal, Spacing.section)
        .padding(.top, Spacing.group)
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("native-stale")
    }

    @ViewBuilder
    private func emptyState(_ phase: AgentRowStore.Phase) -> some View {
        switch phase {
        case .loading, .cached: ProgressView()
        case .live: Text("Nothing in this session yet.").foregroundStyle(.secondary)
        case .unavailable: Text("This pane’s session can’t be shown here. Use the terminal.").foregroundStyle(.secondary)
        case .trouble: Text("Can’t reach the runner. Trying again…").foregroundStyle(.secondary)
        }
    }
}

/// The conversation's box (ov-373). Send goes through `terminal.compose`:
/// typed into claude's own box and submitted, or taken by claude's queue
/// while it works (R-29).
///
/// Seams for what comes later, as on the Mac: attachments and slash commands
/// (ov-367), interrupt and Send Now (ov-368).
struct NativeComposer: View {
    @ObservedObject var model: NativePaneModel
    let showTerminal: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: Spacing.group) {
            if let issue = model.issue, issue != .handoff {
                issueLine(issue)
            }
            HStack(alignment: .bottom, spacing: Spacing.group) {
                TextField("Message Claude", text: $model.draft, axis: .vertical)
                    .lineLimit(1...6)
                    .submitLabel(.send)
                    .accessibilityIdentifier("native-composer")
                Button {
                    Task { await model.send() }
                } label: {
                    Image(systemName: "arrow.up.circle.fill")
                        .font(.title)
                        .foregroundStyle(model.canSend ? AnyShapeStyle(.tint) : AnyShapeStyle(.tertiary))
                }
                .buttonStyle(.plain)
                .disabled(!model.canSend)
                .accessibilityLabel("Send")
                .accessibilityIdentifier("native-send")
            }
            .padding(.leading, Spacing.section)
            .padding(.trailing, Spacing.tight)
            .padding(.vertical, Spacing.tight)
            .frame(minHeight: 44)
            .surface(.floating, in: .capsule)
        }
        .padding(.horizontal, Spacing.inset)
        .padding(.vertical, Spacing.group)
        // Its top edge, for `NativeAgentViewTests`: the line the last row
        // has to clear (ov-383's composer report).
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("native-composer-stack")
    }

    @ViewBuilder
    private func issueLine(_ issue: AgentConversation.SendIssue) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: Spacing.group) {
            switch issue {
            case .draftInTerminal:
                Text(AgentConversation.draftInTerminal)
                Spacer(minLength: Spacing.group)
                Button("Show Terminal", action: showTerminal)
            case .said(let words):
                Text(words)
                Spacer(minLength: Spacing.group)
            case .handoff:
                EmptyView()
            }
            Button("Dismiss") { model.issue = nil }
        }
        .font(.callout)
        .padding(.horizontal, Spacing.inset)
        .padding(.vertical, Spacing.group)
        .surface(.floating, in: .card)
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("native-send-issue")
    }
}
