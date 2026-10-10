import AgentKit
import SwiftUI

/// The native view of a terminal-mode claude or codex pane (ov-372, ov-416):
/// its rows as the
/// runner's projector folds them, newest at the bottom, and a box to type
/// into.
///
/// A lazy list keyed by row id, so a long session builds only the rows on
/// screen and a row's change re-renders that row alone. Older rows are paged
/// in as the top comes into view.
struct NativeAgentView: View {
    @ObservedObject var model: NativePaneModel
    let isFocused: Bool
    let showTerminal: () -> Void

    var body: some View {
        // A subagent opened from the tray or its row (ov-453) takes the
        // conversation's place, in the same style, with the way back above.
        let opened = model.drill.opened
        let store = opened?.store ?? model.store
        NativeTranscript(model: model, store: store, isPane: opened == nil, showTerminal: showTerminal)
            // Clear of the switch that floats in the pane's top corner.
            .contentMargins(.top, opened == nil ? 36 : Spacing.group, for: .scrollContent)
            .overlay {
                if store.shownIds.isEmpty { emptyState(store.phase, agent: opened != nil) }
            }
            // Rows held and the runner not answering: said over them, so stale
            // rows never pass for live ones, and the box waits (`canSend`).
            .overlay(alignment: .top) {
                if !store.shownIds.isEmpty, store.isStale { staleBanner(store.phase) }
            }
            .safeAreaInset(edge: .top, spacing: 0) {
                if let opened {
                    let row = model.store.box(opened.id)?.row
                    NativeAgentHeader(subagent: row.flatMap(Self.subagent), isFocused: isFocused, back: model.closeAgent)
                        .padding(.leading, Spacing.section)
                        // Clear of the switch in the top trailing corner.
                        .padding(.trailing, 72)
                        .padding(.top, Spacing.group)
                }
            }
            .safeAreaInset(edge: .bottom, spacing: 0) {
                VStack(spacing: Spacing.group) {
                    NativeAgentTray(
                        store: model.store, drill: model.drill, opens: model.opensAgents,
                        open: { model.openAgent(row: $0.id, agentId: $0.agentId) }, back: model.closeAgent)
                    if opened == nil {
                        NativeComposer(model: model, isFocused: isFocused, showTerminal: showTerminal)
                    }
                }
                .padding(Spacing.inset)
            }
            .environment(\.nativeOpenAgent, model.opensAgents ? { model.openAgent(row: $0, agentId: $1) } : nil)
            .background(WorkspaceStyle.paper)
            .environment(\.promptImages, model.promptImages)
            .onChange(of: model.store.ids.count) { _, _ in model.settleQueued() }
            .identified("native-agent-view")
    }

    private static func subagent(_ row: AgentRow) -> AgentRow.Subagent? {
        if case .subagent(let sub) = row.kind { sub } else { nil }
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
        .padding(.top, 40)
        .identified("native-stale")
    }

    @ViewBuilder
    private func emptyState(_ phase: AgentRowStore.Phase, agent: Bool) -> some View {
        switch phase {
        case .loading, .cached: ProgressView().controlSize(.small)
        case .live: Text(agent ? "This agent hasn’t written anything yet." : "Nothing in this session yet.").foregroundStyle(.secondary)
        case .unavailable where agent: Text(AgentTray.unopenable).foregroundStyle(.secondary)
        case .unavailable: Text("This pane’s session can’t be shown here. Use the terminal.").foregroundStyle(.secondary)
        case .trouble: Text("Can’t reach the runner. Trying again…").foregroundStyle(.secondary)
        }
    }
}

/// The rows of a conversation, newest at the bottom: the pane's own, or a
/// subagent's opened in its place (ov-453), drawn the same way. Only the
/// pane's take Send Now and a held ask's answers, and show what the
/// composer queued.
private struct NativeTranscript: View {
    @ObservedObject var model: NativePaneModel
    let store: AgentRowStore
    let isPane: Bool
    let showTerminal: () -> Void

    var body: some View {
        ScrollView {
            LazyVStack(alignment: .leading, spacing: Spacing.inset) {
                if store.moreBefore {
                    ProgressView()
                        .controlSize(.small)
                        .frame(maxWidth: .infinity)
                        .onAppear { if isPane { model.loadOlder() } }
                }
                let last = store.items.last?.id
                let sendNow: (() -> Void)? = isPane && model.offersSendNow ? { Task { await model.sendNow() } } : nil
                let answer = isPane ? model.nativeAnswer : nil
                // A run of tool calls is one line (ov-452).
                ForEach(store.items) { item in
                    switch item {
                    case .row(let id):
                        if let box = store.box(id) {
                            NativeRowView(box: box, isLast: id == last, showTerminal: showTerminal, sendNow: sendNow, answer: answer)
                        }
                    case .tools(let id, let rows):
                        ToolGroupRow(id: id, boxes: rows.compactMap(store.box))
                    }
                }
                if isPane {
                    ForEach(model.queued.indices, id: \.self) { i in
                        QueuedLine(text: model.queued[i], sendNow: sendNow)
                    }
                    if model.issue == .handoff {
                        HandoffRow(reason: AgentConversation.handoff(model.agent), showTerminal: showTerminal)
                    } else if model.issue == .panel {
                        HandoffRow(reason: AgentConversation.panel(model.agent), showTerminal: showTerminal)
                    }
                }
            }
            .padding(Spacing.section)
        }
        .defaultScrollAnchor(.bottom)
        .scrollEdgeEffectStyle(.soft, for: .bottom)
        // A subagent opened is a list of its own, at its own tail.
        .id(store.key)
    }
}

/// The native view's box (ov-372). Return sends through `terminal.compose`:
/// typed into claude's own box and submitted, or taken by claude's queue
/// while it works (R-29).
///
/// Where the runner has `compose` (ov-400): Shift-Return makes a new line,
/// images come by paste, drop or the paperclip and wait as chips, and a
/// slash command goes to claude's picker. Without it, one line and no images.
///
/// While claude works, Stop sits beside Send, with ⌘. as its shortcut in the
/// focused pane (ov-368); each waiting Queued row has Send Now.
struct NativeComposer: View {
    @ObservedObject var model: NativePaneModel
    let isFocused: Bool
    let showTerminal: () -> Void
    @State private var fieldHeight = ComposerField.lineHeight

    var body: some View {
        VStack(alignment: .leading, spacing: Spacing.group) {
            if let issue = model.issue, issue != .handoff, issue != .panel {
                issueLine(issue)
            }
            VStack(alignment: .leading, spacing: Spacing.group) {
                if !model.images.isEmpty || !model.files.isEmpty { chips }
                HStack(alignment: .bottom, spacing: Spacing.group) {
                    if model.rich {
                        Button(action: pickImages) {
                            Image(systemName: "paperclip")
                                .font(.title3)
                                .foregroundStyle(.secondary)
                        }
                        .buttonStyle(.plain)
                        .help(model.takesFiles ? "Attach Files" : "Attach Images")
                        .accessibilityLabel(model.takesFiles ? "Attach Files" : "Attach Images")
                        .identified("native-attach")
                    }
                    field
                    if model.offersStop { stopButton }
                    sendButton
                }
            }
            .padding(.horizontal, Spacing.inset)
            .padding(.vertical, Spacing.group)
            .surface(.floating, in: .floating)
        }
    }

    private var field: some View {
        ComposerField(model: model, isFocused: isFocused, measuredHeight: $fieldHeight)
            .frame(height: fieldHeight)
            .overlay(alignment: .topLeading) {
                if model.draft.isEmpty {
                    if let suggestion = model.suggestion {
                        suggestionLine(suggestion)
                    } else {
                        Text(model.hint ?? "Message \(model.agent)")
                            .font(Font(ComposerField.font))
                            .lineLimit(1)
                            .foregroundStyle(.tertiary)
                            .allowsHitTesting(false)
                            .accessibilityHidden(true)
                    }
                }
            }
            .identified("native-composer")
    }

    /// claude's suggested next prompt in the empty box's place (ov-409), with
    /// the key that takes it into the draft. One line, cut at the end with an
    /// ellipsis; the whole of it is what Tab brings in.
    private func suggestionLine(_ suggestion: String) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: Spacing.group) {
            Text(suggestion)
                .font(Font(ComposerField.font))
                .lineLimit(1)
                .truncationMode(.tail)
            Spacer(minLength: 0)
            Text("Tab")
                .font(.caption)
                .layoutPriority(1)
        }
        .foregroundStyle(.tertiary)
        .allowsHitTesting(false)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("Suggested message: \(suggestion). Press Tab to use it.")
        .identified("native-suggestion")
    }

    /// The width of the chip row's trailing fade.
    static let fade: CGFloat = 32

    /// The images waiting to go, each with a button to take it out.
    private var chips: some View {
        ScrollView(.horizontal) {
            HStack(spacing: Spacing.group) {
                ForEach(model.images) { image in
                    chip(image)
                }
                ForEach(model.files) { file in
                    ComposeFileChip(file: file) { model.detach(file.id) }
                }
            }
            .padding(.top, Spacing.tight)
        }
        .scrollIndicators(.never)
        // A fade at the trailing edge says the row goes on past it (ov-454):
        // a chip there is cut softly, never hard. Nothing reaches it while
        // the chips fit.
        .mask {
            HStack(spacing: 0) {
                Color.black
                LinearGradient(colors: [.black, .clear], startPoint: .leading, endPoint: .trailing)
                    .frame(width: Self.fade)
            }
        }
        .identified("native-image-chips")
    }

    private func chip(_ image: ComposeImage) -> some View {
        Group {
            if let thumbnail = model.thumbnails[image.id] {
                Image(nsImage: thumbnail).resizable().aspectRatio(contentMode: .fill)
            } else {
                Image(systemName: "photo").foregroundStyle(.secondary)
            }
        }
        .frame(width: 48, height: 48)
        .surface(.inset, in: .control)
        .clipShape(.control)
        .overlay(alignment: .topTrailing) {
            Button {
                model.detach(image.id)
            } label: {
                Image(systemName: "xmark.circle.fill")
                    .symbolRenderingMode(.palette)
                    // style-exempt: a badge over a photo, white on dark to read on any picture
                    .foregroundStyle(.white, .black.opacity(0.6))
            }
            .buttonStyle(.plain)
            .help("Remove Image")
            .accessibilityLabel("Remove Image")
            .offset(x: Spacing.tight, y: -Spacing.tight)
        }
        .accessibilityElement(children: .contain)
        .accessibilityLabel("Image")
        .identified("native-image-chip")
    }

    /// The paperclip's picker: images from disk, and any file where the
    /// runner takes them (ov-454), added as chips.
    private func pickImages() {
        let panel = NSOpenPanel()
        panel.allowedContentTypes = model.takesFiles ? [.item] : [.image]
        panel.allowsMultipleSelection = true
        panel.canChooseDirectories = false
        panel.prompt = "Attach"
        let model = model
        panel.begin { response in
            guard response == .OK else { return }
            let images = ComposeImage.from(urls: panel.urls)
            let files = panel.urls.filter(ComposeFile.isFile)
            Task { @MainActor in
                if !images.isEmpty { model.attach(images) }
                if !files.isEmpty { model.attach(fileURLs: files) }
            }
        }
    }

    /// Stop: one Esc in the terminal, pressed by the runner only while
    /// claude works and nothing is asking (ov-368).
    private var stopButton: some View {
        Button {
            Task { await model.stop() }
        } label: {
            Image(systemName: "stop.circle.fill")
                .font(.title2)
                .foregroundStyle(.secondary)
        }
        .buttonStyle(.plain)
        .disabled(model.pressing != nil)
        .keyboardShortcut(isFocused ? KeyboardShortcut(".", modifiers: .command) : nil)
        .help("Stop")
        .accessibilityLabel("Stop")
        .identified("native-stop")
    }

    private var sendButton: some View {
        Button {
            Task { await model.send() }
        } label: {
            // A filled circle either way, so it reads on light paper
            // when it can't send yet as well as when it can.
            Image(systemName: "arrow.up.circle.fill")
                .font(.title2)
                .foregroundStyle(model.canSend ? AnyShapeStyle(.tint) : AnyShapeStyle(.tertiary))
        }
        .buttonStyle(.plain)
        .disabled(!model.canSend)
        .help("Send")
        .accessibilityLabel("Send")
    }

    /// Bring Here and Show Terminal, for a draft in claude's box (R-28).
    @ViewBuilder
    private var bringButtons: some View {
        Button("Bring Here") { Task { await model.bringHere() } }
            .disabled(model.bringing)
            .identified("native-bring-here")
        Button("Show Terminal", action: showTerminal)
    }

    @ViewBuilder
    private func issueLine(_ issue: NativePaneModel.SendIssue) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: Spacing.group) {
            switch issue {
            case .draftInTerminal where model.offersBringHere:
                // R-28: refused, and the draft offered here or the terminal.
                // In a narrow tile the words go above the buttons, not to a column.
                ViewThatFits(in: .horizontal) {
                    HStack(alignment: .firstTextBaseline, spacing: Spacing.group) {
                        Text(AgentConversation.draftInTerminalBring).fixedSize()
                        Spacer(minLength: Spacing.group)
                        bringButtons
                    }
                    VStack(alignment: .leading, spacing: Spacing.tight) {
                        Text(AgentConversation.draftInTerminalBring)
                        HStack(spacing: Spacing.group) {
                            Spacer(minLength: 0)
                            bringButtons
                        }
                    }
                }
            case .draftInTerminal:
                Text(AgentConversation.draftInTerminal)
                Spacer(minLength: Spacing.group)
                Button("Show Terminal", action: showTerminal)
            case .draftLeftInTerminal(let words):
                Text(words)
                Spacer(minLength: Spacing.group)
                Button("Show Terminal", action: showTerminal)
            case .said(let words):
                Text(words)
                Spacer(minLength: Spacing.group)
            case .handoff, .panel:
                EmptyView()
            }
            Button("Dismiss") { model.issue = nil }
                .buttonStyle(.link)
        }
        .font(.callout)
        .padding(.horizontal, Spacing.inset)
        .identified("native-send-issue")
    }
}
