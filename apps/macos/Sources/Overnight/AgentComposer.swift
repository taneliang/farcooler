import AgentKit
import AppKit
import SwiftUI
import UniformTypeIdentifiers

/// The composer row: where a prompt is written, `/` and `@` are picked from,
/// images are attached, and the agent's own mode is chosen.
///
/// One row, not three panes of chrome. `TerminalPane`'s doc comment explains
/// why a pane grew no header or footer of its own — mode and attachments
/// belong here instead, which is what keeps an agent pane exactly as
/// chrome-free as a terminal one.
struct AgentComposer: View {
    @ObservedObject var stream: AgentStream
    let terminal: Terminal
    let isFocused: Bool
    /// The worktree file search, injected rather than reached for directly —
    /// the same reason `CommandPalette` is handed a `screen:` closure instead
    /// of a `DaemonClient`: this view can look things up and send a prompt,
    /// and nothing else.
    let searchFiles: (String) async -> [String]
    /// How much room the pane has, which decides how many selectors are shown
    /// inline and how many move into the overflow menu.
    let width: CGFloat

    @State private var text = ""
    @State private var cursor = 0
    @State private var attachments: [ComposerAttachment] = []
    @State private var suggestions: [String] = []
    @State private var highlight = 0
    @State private var isTargetedForDrop = false
    /// The field's height, reported by it rather than negotiated with it.
    @State private var fieldHeight: CGFloat = 20

    private var token: ComposerToken { activeToken(in: text, cursor: cursor) }
    private var pickerOpen: Bool { token != .none && !suggestions.isEmpty }

    var body: some View {
        // One floating card, with everything that belongs to composing inside
        // it — the field, what it will do, and the selectors that change how.
        //
        // It used to be a strip welded to the bottom edge, with its own divider
        // and its own material, and the field in a well inside that. Three
        // nested surfaces to say "type here". The transcript now scrolls behind
        // this card instead of stopping above it, which is what makes the pane
        // read as one conversation with a control resting on top rather than
        // two stacked regions.
        // Two rows, and everything sits on one of them.
        //
        // Before this the card held a status line, a field, a send button
        // vertically centred against a field of changing height, and a wrapping
        // strip of selectors — four things at four different alignments, which
        // is why it read as crowded with no structure. Now: what you are
        // writing, then everything that acts on it, left to right, on one
        // baseline. The send button ends that row rather than floating beside
        // the text.
        VStack(alignment: .leading, spacing: 10) {
            if pickerOpen {
                picker
                Divider()
            }
            if !attachments.isEmpty { attachmentStrip }

            AgentComposerField(
                text: $text,
                cursor: $cursor,
                // The HARNESS, not the pane's label.
                //
                // The label is the conversation's own name now, which made the
                // placeholder read "Message Complete D17 Authorization Decision
                // For Overnight". You are not messaging the conversation, you
                // are messaging the agent having it.
                placeholder: "Message \(Terminal.name(of: terminal.preset).capitalized)",
                isFocused: isFocused,
                pickerOpen: pickerOpen,
                onNavigate: navigate,
                onAccept: acceptHighlighted,
                onSubmit: send,
                onDismissPicker: { suggestions = [] },
                onPasteImage: { attach(image: $0) },
                onApprove: approval.map { pending in
                    { answer(pending, preferring: "allow") }
                },
                onReject: approval.map { pending in
                    { answer(pending, preferring: "reject") }
                },
                measuredHeight: $fieldHeight
            )
            .frame(height: fieldHeight)

            HStack(alignment: .center, spacing: 10) {
                attachButton
                configControls

                // Send is anchored to the trailing edge, always.
                //
                // Without this the row simply ends wherever the selectors
                // happen to end, so the send button sat in the middle of the
                // card with empty space beside it — and it moved every time a
                // selector's value changed length. The one control whose
                // position should never be in question is the one you reach for
                // without looking.
                Spacer(minLength: 8)
                activity
                sendButton
            }
            .frame(maxWidth: .infinity)
            .foregroundStyle(.secondary)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(.horizontal, 12)
        .padding(.vertical, 10)
        .modifier(GlassCard())
        // Debounced by `.task(id:)` itself: a token that changes on every
        // keystroke cancels its predecessor for free, which is the whole
        // debounce — see `TileView.panels` for the same trick against a
        // window drag.
        .task(id: token) { await refreshSuggestions(token) }
        .onDrop(of: [.image, .fileURL], isTargeted: $isTargetedForDrop) { providers in
            handleDrop(providers)
            return true
        }
        .overlay {
            if isTargetedForDrop {
                RoundedRectangle(cornerRadius: 8)
                    .strokeBorder(Color.accentColor, lineWidth: 2)
                    .padding(3)
                    .allowsHitTesting(false)
            }
        }
    }

    // MARK: - Picker

    /// The `/` and `@` picker.
    ///
    /// Not `PaletteField`/`CommandPalette` themselves — those own their whole
    /// field, taking every arrow key for their own list, which is right for a
    /// panel that IS a query and wrong here: the query is a token inside a
    /// message still being written, so the composer's own text view has to
    /// keep the keyboard and only lend the vertical arrows to this list while
    /// a token is open. What is reused is the shape they established: a
    /// bordered `.regularMaterial` list, one highlighted row, click-or-accept.
    private var picker: some View {
        VStack(alignment: .leading, spacing: 1) {
            ForEach(Array(suggestions.prefix(8).enumerated()), id: \.offset) { index, item in
                HStack(spacing: 8) {
                    Image(systemName: pickerIcon)
                        .font(.system(size: 10))
                        .foregroundStyle(.secondary)
                        .frame(width: 12)
                    Text(label(for: item))
                        .font(.system(size: 12, design: isSlash ? .monospaced : .default))
                        .lineLimit(1)
                    Spacer(minLength: 0)
                }
                .padding(.horizontal, 8)
                .padding(.vertical, 4)
                .background(
                    index == highlight ? Color.primary.opacity(0.08) : .clear,
                    in: RoundedRectangle(cornerRadius: 5))
                .contentShape(Rectangle())
                .onTapGesture { accept(item) }
            }
        }
        .padding(6)
        .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 8))
        .overlay(RoundedRectangle(cornerRadius: 8).strokeBorder(Color.primary.opacity(0.08)))
        .padding(.horizontal, 10)
        .padding(.top, 8)
    }

    private var isSlash: Bool { if case .slash = token { return true }; return false }

    private var pickerIcon: String { isSlash ? "chevron.right" : "doc.text" }

    private func label(for item: String) -> String { (isSlash ? "/" : "@") + item }

    private func refreshSuggestions(_ token: ComposerToken) async {
        switch token {
        case .none:
            suggestions = []

        case let .slash(prefix, _):
            // Resent every turn (`Transcript.availableCommands`), so filtering
            // is synchronous — no round trip, no debounce needed.
            suggestions = stream.transcript.availableCommands.filter {
                prefix.isEmpty || $0.lowercased().hasPrefix(prefix.lowercased())
            }
            highlight = 0

        case let .mention(prefix, _):
            guard !prefix.isEmpty else {
                suggestions = []
                return
            }
            // A daemon round trip per keystroke would make typing a path feel
            // like it is fighting the network. `.task(id:)` already cancels
            // the previous search when the token changes, so this sleep is
            // the entire debounce.
            try? await Task.sleep(for: .milliseconds(180))
            guard !Task.isCancelled else { return }
            suggestions = await searchFiles(prefix)
            highlight = 0
        }
    }

    private func navigate(_ move: PaletteMove) {
        guard !suggestions.isEmpty else { return }
        let count = min(suggestions.count, 8)
        switch move {
        case .up, .previous: highlight = (highlight - 1 + count) % count
        case .down, .next: highlight = (highlight + 1) % count
        case .left, .right: break
        }
    }

    private func acceptHighlighted() {
        guard suggestions.indices.contains(highlight) else { return }
        accept(suggestions[highlight])
    }

    /// Replace the token's own range with the chosen completion.
    ///
    /// The count is taken BEFORE the mutation and the cursor set from it
    /// afterwards, rather than reusing `range.lowerBound` post-mutation:
    /// `String.Index` is not guaranteed valid across an edit to the string it
    /// indexes, even one that only touches a later range.
    private func accept(_ item: String) {
        let replacement: String
        let range: Range<String.Index>
        switch token {
        case let .slash(_, r): replacement = "/\(item) "; range = r
        case let .mention(_, r): replacement = "@\(item) "; range = r
        case .none: return
        }
        let prefixCount = text.distance(from: text.startIndex, to: range.lowerBound)
        text.replaceSubrange(range, with: replacement)
        cursor = prefixCount + replacement.count
        suggestions = []
    }

    // MARK: - Attachments

    /// Images ride along as `@path` references rather than as a distinct
    /// block, because `AgentStream.send` — this app's one seam to the
    /// daemon's prompt call — takes a single string. The CLI subcommand
    /// behind it (`terminal agent-prompt`, see `AgentStream`'s own deviation
    /// note) has no image-carrying form yet, and a path the agent can open
    /// with its own file tools is what is actually deliverable today. Silently
    /// dropping the picture instead — the alternative to this — would be
    /// worse than sending a reference an agent without file tools ignores.
    private var attachButton: some View {
        Button(action: pickImages) {
            Image(systemName: "paperclip")
                .font(.system(size: 13))
        }
        .buttonStyle(.plain)
        .foregroundStyle(.secondary)
        .help("Attach an image")
    }

    private func pickImages() {
        let panel = NSOpenPanel()
        panel.allowedContentTypes = [.image]
        panel.allowsMultipleSelection = true
        panel.canChooseDirectories = false
        panel.canChooseFiles = true
        guard panel.runModal() == .OK else { return }
        for url in panel.urls { attach(fileURL: url) }
    }

    private var attachmentStrip: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 6) {
                ForEach(attachments) { attachment in
                    ZStack(alignment: .topTrailing) {
                        Image(nsImage: attachment.thumbnail)
                            .resizable()
                            .scaledToFill()
                            .frame(width: 40, height: 40)
                            .clipShape(RoundedRectangle(cornerRadius: 6))
                            .clipped()
                        Button {
                            attachments.removeAll { $0.id == attachment.id }
                        } label: {
                            Image(systemName: "xmark.circle.fill")
                                .font(.system(size: 12))
                                .symbolRenderingMode(.palette)
                                .foregroundStyle(.white, .black.opacity(0.6))
                        }
                        .buttonStyle(.plain)
                        .offset(x: 4, y: -4)
                    }
                }
            }
            .padding(.horizontal, 10)
        }
        .padding(.top, 8)
    }

    private func attach(fileURL: URL) {
        guard let image = NSImage(contentsOf: fileURL) else { return }
        attachments.append(ComposerAttachment(url: fileURL, thumbnail: image))
    }

    /// Pasted or dragged image DATA rather than a file already on disk has to
    /// be given a path before it can ride as an `@` reference at all — the
    /// only thing an outgoing message can carry is text.
    private func attach(image: NSImage) {
        guard let tiff = image.tiffRepresentation,
            let rep = NSBitmapImageRep(data: tiff),
            let png = rep.representation(using: .png, properties: [:])
        else { return }
        let url = FileManager.default.temporaryDirectory
            .appendingPathComponent(UUID().uuidString)
            .appendingPathExtension("png")
        // Best-effort. A failed write drops the attachment rather than
        // surfacing a banner: the user's action here was pasting a picture,
        // not the write, and the message they were composing still sends
        // fine without it.
        guard (try? png.write(to: url)) != nil else { return }
        attachments.append(ComposerAttachment(url: url, thumbnail: image))
    }

    private func handleDrop(_ providers: [NSItemProvider]) {
        for provider in providers {
            if provider.hasItemConformingToTypeIdentifier(UTType.fileURL.identifier) {
                provider.loadItem(forTypeIdentifier: UTType.fileURL.identifier) { item, _ in
                    guard let data = item as? Data,
                        let url = URL(dataRepresentation: data, relativeTo: nil)
                    else { return }
                    Task { @MainActor in attach(fileURL: url) }
                }
            } else if provider.hasItemConformingToTypeIdentifier(UTType.image.identifier) {
                // `NSImage` does not bridge to `_ObjectiveCBridgeable`, so
                // `loadObject(ofClass:)` — the tidier call — is not available
                // for it; raw data plus `NSImage(data:)` is the fallback the
                // framework itself points at instead.
                provider.loadDataRepresentation(forTypeIdentifier: UTType.image.identifier) {
                    data, _ in
                    guard let data, let image = NSImage(data: data) else { return }
                    Task { @MainActor in attach(image: image) }
                }
            }
        }
    }

    /// "Mode" dim, "Manual" full strength, in one string.
    ///
    /// Showing only the value left a row reading "Manual · Default
    /// (recommended) · High · Off · Default", and there is no way to know what
    /// "Off" is off. The name says what the control is; the value says what it
    /// is set to.
    private func caption(for option: ConfigOption) -> AttributedString {
        var name = AttributedString(option.name + "  ")
        name.foregroundColor = .secondary
        var value = AttributedString(
            option.options.first { $0.id == option.currentValue }?.name ?? option.currentValue)
        value.foregroundColor = .primary
        return name + value
    }

    /// The request the composer's shortcuts answer, if one is waiting.
    private var approval: PendingPermission? { stream.transcript.pendingPermission }

    /// Answer with the first option of a kind — `allow` or `reject`.
    ///
    /// The same rule `ApprovalControls` uses for its buttons, so ⌘↩ and the
    /// button it is printed on cannot mean two different things.
    private func answer(_ pending: PendingPermission, preferring kind: String) {
        let option = pending.options.first { $0.kind.hasPrefix(kind) }
            ?? (kind == "allow" ? pending.options.first : nil)
        guard let option else { return }
        Task { await stream.answer(pending.id, option.id) }
    }

    // MARK: - Mode

    /// The agent's own mode — plan vs act, or whatever the agent calls its
    /// equivalent — never the pane's terminal/chat mode. Three distinct
    /// names for three distinct things; see the plan's global constraints.
    /// One control per selector the agent advertises.
    ///
    /// Not a mode menu and a model menu written out by hand. ACP's config
    /// options are deliberately generic, and the payoff is immediate: this
    /// adapter also advertises a SUBAGENT picker that nobody designed a field
    /// for, and it renders here for free. A `thought_level` will do the same.
    /// How many selectors are shown in full before the rest fold away.
    ///
    /// Chosen from the pane's width rather than measured, on purpose. Three
    /// versions of this row have now existed: a plain `HStack`, which was wider
    /// than a tiled pane and pushed the TRANSCRIPT out of its own bounds; a
    /// custom wrapping `Layout`, which fixed that and froze the app by
    /// re-measuring five platform views on every layout pass; and a horizontal
    /// `ScrollView`, which was stable and hid controls behind an edge with no
    /// scrollbar to say so — a control you cannot see is a control you cannot
    /// know is there, which was the argument for wrapping in the first place.
    ///
    /// Arithmetic on a number the pane already knows is none of those things.
    private var inlineCount: Int {
        // Each selector is roughly 130pt with its name and value, and the row
        // also carries the attach button, the activity line and send.
        let room = width - 150
        return max(0, min(stream.transcript.configOptions.count, Int(room / 130)))
    }

    private var inlineOptions: [ConfigOption] {
        Array(stream.transcript.configOptions.prefix(inlineCount))
    }

    private var overflowOptions: [ConfigOption] {
        Array(stream.transcript.configOptions.dropFirst(inlineCount))
    }

    @ViewBuilder
    private var configControls: some View {
        ForEach(inlineOptions) { option in
            selector(option)
        }

        // Everything that did not fit, nested one level down.
        //
        // Nested rather than flattened into a single list: these are several
        // separate choices, and flattening them would put "High" and "Opus" and
        // "Plan Mode" side by side in one menu with nothing saying which
        // question each answers.
        if !overflowOptions.isEmpty {
            Menu {
                ForEach(overflowOptions) { option in
                    Menu(option.name) {
                        ForEach(option.options) { choice in
                            Button {
                                Task { await stream.setConfig(option.id, choice.id) }
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
                Image(systemName: "ellipsis.circle")
                    .font(.caption)
            }
            .menuStyle(.borderlessButton)
            .fixedSize()
            .help(overflowOptions.map(\.name).joined(separator: ", "))
        }
    }

    /// One selector, shown in full.
    @ViewBuilder
    private func selector(_ option: ConfigOption) -> some View {
        if option.isBoolean {
            Toggle(
                option.name,
                isOn: Binding(
                    get: { option.isOn },
                    set: { on in Task { await stream.setConfig(option.id, on ? "true" : "false") } }
                )
            )
            .toggleStyle(.checkbox)
            .font(.caption)
        } else if !option.options.isEmpty {
            Menu {
                ForEach(option.options) { choice in
                    Button {
                        Task { await stream.setConfig(option.id, choice.id) }
                    } label: {
                        // The description is the useful half — "Opus 5 · Best
                        // for everyday, complex tasks" says more than the word
                        // it sits under.
                        if choice.description.isEmpty {
                            Text(choice.name)
                        } else {
                            Text("\(choice.name) — \(choice.description)")
                        }
                    }
                }
            } label: {
                // ONE `Text`, not a stack of two.
                //
                // A borderless menu is an `NSPopUpButton` underneath, and it
                // takes its title from a single string — hand it an `HStack`
                // and everything after the first view is silently dropped,
                // which is how the row came to read "Mode  Model  Effort" with
                // not a value in sight.
                Text(caption(for: option))
                    .font(.caption)
                    .lineLimit(1)
            }
            .menuStyle(.borderlessButton)
            .fixedSize()
        }
    }

    /// What the agent is doing, on the two occasions it is not obvious.
    ///
    /// This used to report every state including `Done` and `Idle`, on the
    /// theory that silence reads as the app having lost track. In a chat it
    /// does not: the transcript IS the state, and a finished turn ending in
    /// "Done" is the interface saying what the reader has just watched happen.
    ///
    /// What survives is the two states the transcript cannot show — waiting for
    /// an answer, and not started yet, where an empty pane with no selectors is
    /// genuinely indistinguishable from a broken one.
    @ViewBuilder
    private var activity: some View {
        if terminal.agent == .blocked {
            HStack(spacing: 5) {
                StatusGlyph(status: .blocked, size: 6)
                Text("Waiting for you")
            }
            .font(.caption)
        } else if stream.transcript.configOptions.isEmpty && stream.transcript.rows.isEmpty {
            HStack(spacing: 5) {
                ProgressView().controlSize(.mini)
                Text("Starting the agent…")
            }
            .font(.caption)
        }
    }

    /// How full the context window is.
    ///
    /// Quiet until it matters. Below half there is nothing to decide, and a
    /// number on screen at all times is a number nobody reads; past that it is
    /// the honest answer to "why has it started forgetting things".
    @ViewBuilder
    private var context: some View {
        if let fraction = stream.transcript.contextFraction, fraction >= 0.5 {
            HStack(spacing: 4) {
                ProgressView(value: fraction)
                    .progressViewStyle(.linear)
                    .frame(width: 34)
                Text("\(Int(fraction * 100))% context")
            }
            .foregroundStyle(fraction >= 0.85 ? AnyShapeStyle(.orange) : AnyShapeStyle(.secondary))
            .help("Context window used")
        }
    }

    /// Return sends; this is for the people who look for a button.
    @ViewBuilder
    private var sendButton: some View {
        let empty = text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            && attachments.isEmpty
        Button(action: send) {
            Image(systemName: "arrow.up.circle.fill")
                .font(.system(size: 18))
                .foregroundStyle(empty ? AnyShapeStyle(.tertiary) : AnyShapeStyle(Color.accentColor))
        }
        .buttonStyle(.plain)
        .disabled(empty)
        .help("Send (return)")
    }

    // MARK: - Send

    private func send() {
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty || !attachments.isEmpty else { return }

        // The BYTES, not the path.
        //
        // This used to write `@/Users/you/x.png` into the message. That names a
        // file on the machine that picked it, and the agent runs on the host —
        // so it worked when those were the same machine and silently referred
        // to nothing when they were not, which is every remote host.
        let images = attachments.compactMap { attachment -> ComposerImage? in
            guard let data = try? Data(contentsOf: attachment.url) else { return nil }
            return ComposerImage(mime: mimeType(for: attachment.url), data: data)
        }
        let body = trimmed

        text = ""
        cursor = 0
        attachments = []
        suggestions = []
        // Whether this is going out now or joining the queue is the daemon's
        // decision, but the composer already knows the answer and the echo
        // depends on it — see `AgentStream.send`.
        let working = terminal.agent == .working
        Task { await stream.send(body, images: images, whileWorking: working) }
    }
}

/// An attached picture, as it goes to the agent.
struct ComposerImage {
    let mime: String
    let data: Data
}

/// The image type, from the file's extension — all a picker gives us, and all
/// the adapter needs in order to decode.
func mimeType(for url: URL) -> String {
    switch url.pathExtension.lowercased() {
    case "png": "image/png"
    case "gif": "image/gif"
    case "webp": "image/webp"
    case "heic": "image/heic"
    default: "image/jpeg"
    }
}

private struct ComposerAttachment: Identifiable {
    let id = UUID()
    let url: URL
    let thumbnail: NSImage
}

// MARK: - Field

/// The composer's own `NSTextView`.
///
/// `Composer` (the task-description field) already establishes the pattern:
/// Return sends, Shift-Return breaks the line. What this adds is the picker's
/// vertical arrows and Tab/Return-to-accept when a token is open, and giving
/// the tiling prefix first refusal on every key — see `ComposerTextView`.
private struct AgentComposerField: NSViewRepresentable {
    @Binding var text: String
    @Binding var cursor: Int
    var placeholder: String
    var isFocused: Bool
    var pickerOpen: Bool
    let onNavigate: (PaletteMove) -> Void
    let onAccept: () -> Void
    let onSubmit: () -> Void
    let onDismissPicker: () -> Void
    let onPasteImage: (NSImage) -> Void
    /// Set only while a permission is waiting. Nil the rest of the time, so
    /// the keystrokes mean nothing when there is nothing to answer.
    var onApprove: (() -> Void)?
    var onReject: (() -> Void)?

    func makeCoordinator() -> Coordinator { Coordinator(self) }

    func makeNSView(context: Context) -> NSScrollView {
        let scroll = NSScrollView()
        scroll.drawsBackground = false
        scroll.hasVerticalScroller = false
        scroll.autohidesScrollers = true

        let view = ComposerTextView()
        view.delegate = context.coordinator
        view.isRichText = false
        // The standard growing-text-view configuration, and it is what makes
        // `Coordinator.report(_:)` mean anything: the text view tracks the
        // width it is given and grows only downward, so its used height IS the
        // height of what has been typed. Without it the view kept whatever
        // height it was first handed and drew its placeholder wherever that
        // left the origin — which is how "Message agent" ended up floating in
        // the middle of the box.
        view.isVerticallyResizable = true
        view.isHorizontallyResizable = false
        view.autoresizingMask = [.width]
        view.minSize = NSSize(width: 0, height: 0)
        view.maxSize = NSSize(
            width: CGFloat.greatestFiniteMagnitude, height: CGFloat.greatestFiniteMagnitude)
        view.textContainer?.widthTracksTextView = true
        view.textContainer?.heightTracksTextView = false
        view.drawsBackground = false
        view.font = .systemFont(ofSize: 13)
        view.textContainerInset = NSSize(width: 2, height: 4)
        view.isAutomaticQuoteSubstitutionEnabled = false
        view.isAutomaticDashSubstitutionEnabled = false
        // A prompt is code as often as prose, same reasoning as `Composer`.
        view.isAutomaticTextReplacementEnabled = false
        view.placeholder = placeholder
        view.string = text

        scroll.documentView = view
        context.coordinator.view = view
        apply(view)
        context.coordinator.report(view)
        return scroll
    }

    func updateNSView(_ scroll: NSScrollView, context: Context) {
        guard let view = scroll.documentView as? ComposerTextView else { return }
        if view.string != text {
            view.string = text
            let location = text.utf16Location(ofCharacterOffset: cursor)
            view.setSelectedRange(NSRange(location: location, length: 0))
        }
        view.placeholder = placeholder
        apply(view)

        // Claimed on becoming focused, never merely on existing — the same
        // rule `TerminalSurface` follows for the same reason: re-asserted on
        // every update because a pane can gain focus long after this field
        // mounted, and never claimed while another pane owns the keyboard.
        if isFocused, context.coordinator.focused != true {
            context.coordinator.focused = true
            DispatchQueue.main.async { view.window?.makeFirstResponder(view) }
        } else if !isFocused {
            context.coordinator.focused = false
        }
    }

    /// Reported by the coordinator after the text changes, NOT measured during
    /// layout.
    ///
    /// This used to be a `sizeThatFits` that laid the text out and returned the
    /// result. That reads as the obvious answer and it deadlocked the app: a
    /// view that measures itself by mutating its own layout, inside a parent
    /// whose size depends on the answer, is a loop with no fixed point. Two
    /// separate freezes came from it, both of them thousands of frames of
    /// `sizeThatFits` on the main thread.
    ///
    /// Text changes, then height changes, then layout happens. One direction
    /// only.
    @Binding var measuredHeight: CGFloat

    private func apply(_ view: ComposerTextView) {
        view.pickerOpen = pickerOpen
        view.onNavigate = onNavigate
        view.onAccept = onAccept
        view.onSubmit = onSubmit
        view.onDismissPicker = onDismissPicker
        view.onPasteImage = onPasteImage
        view.onApprove = onApprove
        view.onReject = onReject
    }

    final class Coordinator: NSObject, NSTextViewDelegate {
        let parent: AgentComposerField
        weak var view: ComposerTextView?
        var focused: Bool?

        init(_ parent: AgentComposerField) { self.parent = parent }

        func textDidChange(_ notification: Notification) {
            guard let view = notification.object as? NSTextView else { return }
            parent.text = view.string
            parent.cursor = view.string.characterOffset(ofUTF16Location: view.selectedRange().location)
            report(view)
        }

        /// How tall the text is now, clamped to what the composer will show.
        ///
        /// Sent up as state rather than answered during layout — see
        /// `measuredHeight`.
        func report(_ view: NSTextView) {
            guard let container = view.textContainer, let manager = view.layoutManager else {
                return
            }
            manager.ensureLayout(for: container)
            let content = manager.usedRect(for: container).height
                + view.textContainerInset.height * 2
            let clamped = min(max(content, 20), 140)
            guard abs(clamped - parent.measuredHeight) > 0.5 else { return }
            DispatchQueue.main.async { [parent] in parent.measuredHeight = clamped }
        }

        func textViewDidChangeSelection(_ notification: Notification) {
            guard let view = notification.object as? NSTextView else { return }
            parent.cursor = view.string.characterOffset(ofUTF16Location: view.selectedRange().location)
        }
    }
}

/// An `NSTextView` that sends on Return, hands its arrows to an open picker,
/// and gives the tiling prefix first refusal on every key it sees.
///
/// That last part is the load-bearing one. `PaletteField`'s own doc comment
/// explains why a plain `NSTextView` cannot be trusted with ⌃B on its own:
/// once this field is first responder, nothing else in the chain ever sees
/// the keystroke before `NSTextView` would insert or ignore it. Without this
/// override, focusing the composer to type a message would silently disable
/// every `⌃B` binding and the prefix-less `⌃HJKL` pane traversal until focus
/// moved elsewhere — exactly the failure `TerminalRenderView.keyDown` already
/// guards against for a terminal, and a chat pane needs the same guard.
final class ComposerTextView: NSTextView {
    var pickerOpen = false
    var onNavigate: ((PaletteMove) -> Void)?
    var onAccept: (() -> Void)?
    var onSubmit: (() -> Void)?
    var onDismissPicker: (() -> Void)?
    var onPasteImage: ((NSImage) -> Void)?
    var onApprove: (() -> Void)?
    var onReject: (() -> Void)?
    var placeholder: String = "" { didSet { needsDisplay = true } }

    override func keyDown(with event: NSEvent) {
        if MainActor.assumeIsolated({ PrefixMode.shared.handle(event) }) == .handled {
            return
        }

        // Answering the agent, from wherever the cursor happens to be.
        //
        // The composer holds first responder for the whole pane, so a shortcut
        // attached only to the buttons would be dead exactly when a user is
        // most likely to reach for it — mid-sentence, with the agent waiting.
        if event.modifierFlags.contains(.command) {
            switch Int(event.keyCode) {
            case 36, 76:  // Return, keypad Enter
                if let onApprove { onApprove(); return }
            case 51:  // Delete
                if let onReject { onReject(); return }
            default: break
            }
        }

        if pickerOpen {
            switch Int(event.keyCode) {
            case 126: onNavigate?(.up); return
            case 125: onNavigate?(.down); return
            case 48: onNavigate?(event.modifierFlags.contains(.shift) ? .previous : .next); return
            case 36, 76: onAccept?(); return  // Return, keypad Enter
            case 53: onDismissPicker?(); return  // Esc closes the picker, not the compose
            default: break
            }
        }

        let isReturn = event.keyCode == 36 || event.keyCode == 76
        if isReturn && !event.modifierFlags.contains(.shift) {
            onSubmit?()
            return
        }
        super.keyDown(with: event)
    }

    /// Belt and braces: some input methods deliver Return through
    /// `insertNewline` rather than `keyDown`, and `Composer`'s own field
    /// overrides this too for the same reason.
    override func insertNewline(_ sender: Any?) {
        // Shift+Return is a line break, and this is where it was being eaten.
        //
        // `keyDown` above deliberately lets it through to `super` — and
        // `super`'s answer to Return, shift or not, is to call this method. So
        // the fallback below swallowed the exact keystroke `keyDown` had just
        // taken care to let past, and a multi-line message was impossible to
        // type.
        if NSApp.currentEvent?.modifierFlags.contains(.shift) == true {
            super.insertNewline(sender)
            return
        }
        if pickerOpen { onAccept?() } else { onSubmit?() }
    }

    override func paste(_ sender: Any?) {
        let pasteboard = NSPasteboard.general
        if let image = NSImage(pasteboard: pasteboard) {
            onPasteImage?(image)
            return
        }
        super.paste(sender)
    }

    override func draw(_ dirtyRect: NSRect) {
        super.draw(dirtyRect)
        guard string.isEmpty, !placeholder.isEmpty else { return }
        effectiveAppearance.performAsCurrentDrawingAppearance {
            placeholder.draw(
                at: NSPoint(x: textContainerInset.width + 5, y: textContainerInset.height),
                withAttributes: [
                    .font: font ?? .systemFont(ofSize: 13),
                    .foregroundColor: NSColor.tertiaryLabelColor,
                ])
        }
    }
}

// MARK: - Character/UTF-16 offset conversion

extension String {
    /// `AgentKit.activeToken` counts characters — the way a person counts
    /// them — but `NSTextView` reports selection in UTF-16 code units. Every
    /// emoji and most non-Latin scripts make those two counts diverge, so the
    /// conversion has to happen at this boundary rather than be assumed away.
    fileprivate func characterOffset(ofUTF16Location location: Int) -> Int {
        guard
            let utf16Index = utf16.index(
                utf16.startIndex, offsetBy: location, limitedBy: utf16.endIndex),
            let index = utf16Index.samePosition(in: self)
        else { return count }
        return distance(from: startIndex, to: index)
    }

    fileprivate func utf16Location(ofCharacterOffset offset: Int) -> Int {
        guard let index = self.index(startIndex, offsetBy: offset, limitedBy: endIndex) else {
            return utf16.count
        }
        guard let utf16Index = index.samePosition(in: utf16) else { return utf16.count }
        return utf16.distance(from: utf16.startIndex, to: utf16Index)
    }
}


/// The composer's surface: Liquid Glass, because that is what a control resting
/// ON scrolling content is on this platform.
///
/// No fallback. macOS 26 is this app's floor, so the material-and-hairline
/// approximation that used to sit behind an availability check was a second
/// implementation nobody ran — the kind that rots quietly and then misleads the
/// next person who reads it as documentation of what the app does.
struct GlassCard: ViewModifier {
    var cornerRadius: CGFloat = 20

    func body(content: Content) -> some View {
        content.glassEffect(.regular, in: .rect(cornerRadius: cornerRadius))
    }
}
