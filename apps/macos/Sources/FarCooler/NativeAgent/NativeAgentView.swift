import AgentKit
import SwiftUI

/// The native view of a terminal-mode claude pane (ov-372): its rows as the
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
        let store = model.store
        ScrollView {
            LazyVStack(alignment: .leading, spacing: Spacing.inset) {
                if store.moreBefore {
                    ProgressView()
                        .controlSize(.small)
                        .frame(maxWidth: .infinity)
                        .onAppear { model.loadOlder() }
                }
                let last = store.ids.last
                let sendNow: (() -> Void)? = model.offersSendNow ? { Task { await model.sendNow() } } : nil
                let answer = model.nativeAnswer
                ForEach(store.ids, id: \.self) { id in
                    if let box = store.box(id) {
                        NativeRowView(box: box, isLast: id == last, showTerminal: showTerminal, sendNow: sendNow, answer: answer)
                    }
                }
                ForEach(model.queued.indices, id: \.self) { i in
                    QueuedLine(text: model.queued[i], sendNow: sendNow)
                }
                if model.issue == .handoff {
                    HandoffRow(reason: "Claude is showing something only the terminal can.", showTerminal: showTerminal)
                } else if model.issue == .panel {
                    HandoffRow(reason: "This opens a panel in Claude, so it’s for the terminal.", showTerminal: showTerminal)
                }
            }
            .padding(Spacing.section)
        }
        // Clear of the switch that floats in the pane's top corner.
        .contentMargins(.top, 36, for: .scrollContent)
        .defaultScrollAnchor(.bottom)
        .scrollEdgeEffectStyle(.soft, for: .bottom)
        .overlay {
            if store.ids.isEmpty { emptyState(store.phase) }
        }
        // Rows held and the runner not answering: said over them, so stale
        // rows never pass for live ones, and the box waits (`canSend`).
        .overlay(alignment: .top) {
            if !store.ids.isEmpty, store.isStale { staleBanner(store.phase) }
        }
        .safeAreaInset(edge: .bottom, spacing: 0) {
            NativeComposer(model: model, isFocused: isFocused, showTerminal: showTerminal)
                .padding(Spacing.inset)
        }
        .background(WorkspaceStyle.paper)
        .onChange(of: store.ids.count) { _, _ in model.settleQueued() }
        .identified("native-agent-view")
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
    private func emptyState(_ phase: AgentRowStore.Phase) -> some View {
        switch phase {
        case .loading, .cached: ProgressView().controlSize(.small)
        case .live: Text("Nothing in this session yet.").foregroundStyle(.secondary)
        case .unavailable: Text("This pane’s session can’t be shown here. Use the terminal.").foregroundStyle(.secondary)
        case .trouble: Text("Can’t reach the runner. Trying again…").foregroundStyle(.secondary)
        }
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
                if !model.images.isEmpty { chips }
                HStack(alignment: .bottom, spacing: Spacing.group) {
                    if model.rich {
                        Button(action: pickImages) {
                            Image(systemName: "paperclip")
                                .font(.title3)
                                .foregroundStyle(.secondary)
                        }
                        .buttonStyle(.plain)
                        .help("Attach Images")
                        .accessibilityLabel("Attach Images")
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
                    Text("Message Claude")
                        .font(Font(ComposerField.font))
                        .foregroundStyle(.tertiary)
                        .allowsHitTesting(false)
                        .accessibilityHidden(true)
                }
            }
            .identified("native-composer")
    }

    /// The images waiting to go, each with a button to take it out.
    private var chips: some View {
        ScrollView(.horizontal) {
            HStack(spacing: Spacing.group) {
                ForEach(model.images) { image in
                    chip(image)
                }
            }
            .padding(.top, Spacing.tight)
        }
        .scrollIndicators(.never)
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

    /// The paperclip's picker: images from disk, added as chips.
    private func pickImages() {
        let panel = NSOpenPanel()
        panel.allowedContentTypes = [.image]
        panel.allowsMultipleSelection = true
        panel.canChooseDirectories = false
        panel.prompt = "Attach"
        let model = model
        panel.begin { response in
            guard response == .OK else { return }
            let images = ComposeImage.from(urls: panel.urls)
            Task { @MainActor in model.attach(images) }
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

    @ViewBuilder
    private func issueLine(_ issue: NativePaneModel.SendIssue) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: Spacing.group) {
            switch issue {
            case .draftInTerminal:
                Text("The terminal’s box already holds a draft. Send or clear it there first.")
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
