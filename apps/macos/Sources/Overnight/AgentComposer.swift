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

    @State private var text = ""
    @State private var cursor = 0
    @State private var attachments: [ComposerAttachment] = []
    @State private var suggestions: [String] = []
    @State private var highlight = 0
    @State private var isTargetedForDrop = false

    private var token: ComposerToken { activeToken(in: text, cursor: cursor) }
    private var pickerOpen: Bool { token != .none && !suggestions.isEmpty }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            if pickerOpen { picker }
            if !attachments.isEmpty { attachmentStrip }
            Divider()
            VStack(alignment: .leading, spacing: 7) {
                activity
                // The field, in a bordered well. Bare on a material it read as
                // a caption floating in the chrome rather than as somewhere to
                // type — there was nothing to say where the input was.
                HStack(alignment: .bottom, spacing: 8) {
                    AgentComposerField(
                        text: $text,
                        cursor: $cursor,
                        placeholder: "Message \(terminal.label)",
                        isFocused: isFocused,
                        pickerOpen: pickerOpen,
                        onNavigate: navigate,
                        onAccept: acceptHighlighted,
                        onSubmit: send,
                        onDismissPicker: { suggestions = [] },
                        onPasteImage: { attach(image: $0) }
                    )
                    .frame(minHeight: 20, maxHeight: 140)
                    sendButton
                }
                .padding(.horizontal, 10)
                .padding(.vertical, 8)
                .background(Color(nsColor: .textBackgroundColor), in: RoundedRectangle(cornerRadius: 10))
                .overlay {
                    RoundedRectangle(cornerRadius: 10)
                        .strokeBorder(.quaternary, lineWidth: 1)
                }

                HStack(spacing: 10) {
                    attachButton
                    modeMenu
                    modelMenu
                    Spacer(minLength: 0)
                }
                .foregroundStyle(.secondary)
            }
            .padding(.horizontal, 10)
            .padding(.vertical, 9)
        }
        .background(.thinMaterial)
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

    // MARK: - Mode

    /// The agent's own mode — plan vs act, or whatever the agent calls its
    /// equivalent — never the pane's terminal/chat mode. Three distinct
    /// names for three distinct things; see the plan's global constraints.
    @ViewBuilder
    private var modeMenu: some View {
        choiceMenu(
            choices: stream.transcript.availableModes,
            current: stream.transcript.agentMode,
            fallback: "Mode"
        ) { id in await stream.setMode(id) }
    }

    /// The model picker.
    ///
    /// `session/set_model`, verified working against a live adapter. ACP has
    /// since stabilised `session/set_config_option` as the general form, but
    /// the adapter in use answers `Method not found` to it — when agents start
    /// advertising `configOptions`, this becomes one selector among several
    /// rather than a special case.
    @ViewBuilder
    private var modelMenu: some View {
        choiceMenu(
            choices: stream.transcript.availableModels,
            current: stream.transcript.model,
            fallback: "Model"
        ) { id in await stream.setModel(id) }
    }

    /// One picker over a list of things an agent offers.
    ///
    /// Shows `name`, sends `id`. Built from ids alone it read `acceptEdits` and
    /// `bypassPermissions` at a user who never chose those words.
    @ViewBuilder
    private func choiceMenu(
        choices: [AgentChoice], current: String?, fallback: String,
        select: @escaping (String) async -> Void
    ) -> some View {
        if !choices.isEmpty {
            Menu {
                ForEach(choices) { choice in
                    Button {
                        Task { await select(choice.id) }
                    } label: {
                        // The description is the useful half — "Opus 4.6 ·
                        // Most capable for complex work" says more than the
                        // name it sits under.
                        if choice.description.isEmpty {
                            Text(choice.name)
                        } else {
                            Text("\(choice.name) — \(choice.description)")
                        }
                    }
                }
            } label: {
                Text(choices.first { $0.id == current }?.name ?? fallback)
                    .font(.caption)
                    .lineLimit(1)
            }
            .menuStyle(.borderlessButton)
            .fixedSize()
        }
    }

    /// What the agent is doing right now, in words.
    ///
    /// The daemon already derives this and every other surface renders it — the
    /// sidebar dot, the fleet row, the notification. A chat that showed nothing
    /// while a turn ran left the user with a blank panel and no way to tell a
    /// thinking agent from a broken one.
    private var activity: some View {
        // Always shown, idle included.
        //
        // `StatusGlyph`'s silence-by-default rule is right for a fleet list,
        // where most rows are quiet and an icon on every one is an icon on
        // none. A chat is the opposite situation: there is exactly one agent
        // here and the whole question is what it is doing right now, so saying
        // nothing reads as the app having lost track of it.
        let state = terminal.agent
        return HStack(spacing: 5) {
            switch state {
            case .working:
                ProgressView().controlSize(.mini)
                Text("Working…")
            case .blocked:
                StatusGlyph(status: .blocked, size: 6)
                Text("Waiting for you")
            case .done:
                StatusGlyph(status: .done, size: 6)
                Text("Done")
            default:
                StatusGlyph(status: .idle, size: 6)
                Text("Idle")
            }
            Spacer(minLength: 0)
        }
        .font(.caption)
        .foregroundStyle(.secondary)
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
        let paths = attachments.map { "@\($0.url.path)" }.joined(separator: " ")
        let body = paths.isEmpty ? trimmed : (trimmed.isEmpty ? paths : "\(paths)\n\(trimmed)")

        text = ""
        cursor = 0
        attachments = []
        suggestions = []
        Task { await stream.send(body) }
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

    func makeCoordinator() -> Coordinator { Coordinator(self) }

    func makeNSView(context: Context) -> NSScrollView {
        let scroll = NSScrollView()
        scroll.drawsBackground = false
        scroll.hasVerticalScroller = false
        scroll.autohidesScrollers = true

        let view = ComposerTextView()
        view.delegate = context.coordinator
        view.isRichText = false
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

    private func apply(_ view: ComposerTextView) {
        view.pickerOpen = pickerOpen
        view.onNavigate = onNavigate
        view.onAccept = onAccept
        view.onSubmit = onSubmit
        view.onDismissPicker = onDismissPicker
        view.onPasteImage = onPasteImage
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
    var placeholder: String = "" { didSet { needsDisplay = true } }

    override func keyDown(with event: NSEvent) {
        if MainActor.assumeIsolated({ PrefixMode.shared.handle(event) }) == .handled {
            return
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
