import PhotosUI
import SwiftUI
import UIKit

/// The conversation's box (ov-373, ov-404). Send goes through
/// `terminal.compose`: typed into claude's own box and submitted, or taken by
/// claude's queue while it works (R-29).
///
/// Where the runner has `compose` (`NativePaneModel.rich`), as on the Mac:
/// Return is a new line and the Send button sends (⌘↩ too, from a hardware
/// keyboard); photos come from the picker or a paste and wait as chips; a
/// slash command goes to claude's picker. While claude works, Stop sits beside
/// Send, and each waiting Queued row has Send Now (ov-368). Without `compose`,
/// one line and no photos.
struct NativeComposer: View {
    @ObservedObject var model: NativePaneModel
    let showTerminal: () -> Void

    /// The side of a button's target, and of its glyph's room.
    private static let targetSide: CGFloat = 44
    /// What the attach glyph (`photo.badge.plus` at `.title3`, about 26 pt wide)
    /// leaves each side of its target: a chip strip is inset by it, so the
    /// first chip's edge and the glyph's ink share a line.
    private static let glyphSlack: CGFloat = 9

    @State private var fieldHeight = NativeComposerField.lineHeight
    @State private var picked: [PhotosPickerItem] = []

    var body: some View {
        VStack(alignment: .leading, spacing: Spacing.group) {
            if let issue = model.issue, issue != .handoff, issue != .panel {
                issueLine(issue)
            }
            VStack(alignment: .leading, spacing: Spacing.group) {
                if !model.images.isEmpty { chips }
                HStack(alignment: .bottom, spacing: Spacing.group) {
                    if model.rich { attachButton }
                    field
                    if model.offersStop { stopButton }
                    sendButton
                }
            }
            .padding(.leading, Spacing.section)
            .padding(.trailing, Spacing.tight)
            .padding(.vertical, Spacing.tight)
            .frame(minHeight: 44)
            .surface(.floating, in: .floating)
        }
        .padding(.horizontal, Spacing.inset)
        .padding(.vertical, Spacing.group)
        // Its top edge, for `NativeAgentViewTests`: the line the last row
        // has to clear (ov-383's composer report).
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("native-composer-stack")
    }

    private var field: some View {
        NativeComposerField(
            text: $model.draft, height: $fieldHeight, label: "Message \(model.agent)",
            onSend: { Task { await model.send() } },
            onPasteImages: model.rich ? { datas in Task { await model.attach(picked: datas) } } : nil,
            claimsKeyboard: model.suggestionsTaken,
            offersTab: { model.suggestion != nil }, onTab: { model.takeSuggestion() }
        )
        .frame(height: fieldHeight)
        .overlay(alignment: .topLeading) {
            if model.draft.isEmpty, model.suggestion == nil {
                // claude's own `Try "…"` example where it shows one (ov-409):
                // read to VoiceOver as the hint it is, and never taken by a
                // tap, which puts the caret in the box.
                Text(model.hint ?? "Message \(model.agent)")
                    .lineLimit(1)
                    .foregroundStyle(.secondary)
                    // `NativeComposerField`'s own `textContainerInset`, so the
                    // placeholder sits where the first typed letter will land.
                    .padding(.top, NativeComposerField.inset)
                    .allowsHitTesting(false)
                    .accessibilityHidden(model.hint == nil)
                    .accessibilityIdentifier(model.hint == nil ? "native-placeholder" : "native-hint")
            }
        }
        // Centered in the row's 44 pt of target when it is one line, so the
        // field's line and the buttons beside it share a middle.
        .frame(minHeight: 44, alignment: .center)
        .overlay(alignment: .leading) { suggestionButton }
    }

    /// claude's suggested next prompt in the empty box's place (ov-409): one
    /// line, cut at the end, with the arrow every suggestion row uses for
    /// "put this in the field". A tap takes it as a draft and brings up the
    /// keyboard to edit it; it is never sent.
    @ViewBuilder
    private var suggestionButton: some View {
        if model.draft.isEmpty, let suggestion = model.suggestion {
            Button {
                model.takeSuggestion()
            } label: {
                HStack(spacing: Spacing.group) {
                    Text(suggestion)
                        .lineLimit(1)
                        .truncationMode(.tail)
                    Image(systemName: "arrow.up.left")
                        .font(.footnote)
                        .foregroundStyle(.tertiary)
                }
                .foregroundStyle(.secondary)
                .frame(maxHeight: .infinity)
                .contentShape(.rect)
            }
            .buttonStyle(.plain)
            // Only as wide as its words, so a tap beside them still puts the
            // caret in the field to type something else.
            .accessibilityLabel("Suggested message: \(suggestion)")
            .accessibilityHint("Puts it in the message to edit.")
            .accessibilityIdentifier("native-suggestion")
        }
    }

    /// The photo picker: photos added as chips, up to what the message takes.
    private var attachButton: some View {
        PhotosPicker(
            selection: $picked, maxSelectionCount: max(1, model.imageRoom), matching: .images
        ) {
            Image(systemName: "photo.badge.plus")
                .font(.title3)
                .foregroundStyle(.secondary)
                .frame(width: Self.targetSide, height: Self.targetSide)
                .contentShape(.rect)
        }
        // A `PhotosPicker` tints its label with the app's accent, and a
        // foreground style on the label doesn't survive it: the glyph came out
        // blue beside a gray Stop (ov-404 review 1).
        .tint(.secondary)
        .disabled(model.imageRoom == 0)
        .accessibilityLabel("Attach Photos")
        .accessibilityIdentifier("native-attach")
        .onChange(of: picked) { _, items in
            guard !items.isEmpty else { return }
            picked = []
            Task {
                var datas: [Data?] = []
                // Loudly: a photo that can't be read says so (an iCloud asset
                // not on the device is the usual reason), rather than the
                // picker looking as though it did nothing.
                for item in items { datas.append(try? await item.loadTransferable(type: Data.self)) }
                await model.attach(picked: datas)
            }
        }
    }

    /// The images waiting to go, each with a button to take it out.
    private var chips: some View {
        ScrollView(.horizontal) {
            HStack(spacing: Spacing.group) {
                ForEach(model.images) { image in chip(image) }
            }
            // Room above for the remove badge, which sits on the chip's corner,
            // so a larger Dynamic Type size can't clip it; the strip's own left
            // inset lines the first chip up with the attach glyph.
            .padding(.top, Spacing.group)
            .padding(.leading, model.rich ? Self.glyphSlack : 0)
            .padding(.trailing, Spacing.group)
        }
        .scrollIndicators(.never)
        .accessibilityIdentifier("native-image-chips")
    }

    private func chip(_ image: OutgoingImage) -> some View {
        Group {
            if let thumbnail = model.thumbnails[image.id] {
                Image(uiImage: thumbnail).resizable().aspectRatio(contentMode: .fill)
            } else {
                Image(systemName: "photo").foregroundStyle(.secondary)
            }
        }
        .frame(width: 56, height: 56)
        .surface(.inset, in: .control)
        .clipShape(.control)
        .overlay(alignment: .topTrailing) {
            Button {
                model.detach(image.id)
            } label: {
                Image(systemName: "xmark.circle.fill")
                    .font(.system(size: 18))
                    .symbolRenderingMode(.palette)
                    // style-exempt: a badge over a photo, white on dark to read on any picture
                    .foregroundStyle(.white, .black.opacity(0.6))
                    .frame(width: 44, height: 44)
                    .contentShape(.rect)
            }
            .buttonStyle(.plain)
            .accessibilityLabel("Remove Photo")
            .accessibilityIdentifier("native-image-remove")
            // The 44 pt target is centered on the badge, which sits on the
            // chip's corner.
            .offset(x: 14, y: -14)
        }
        .accessibilityElement(children: .contain)
        .accessibilityLabel("Photo")
        .accessibilityIdentifier("native-image-chip")
    }

    /// Stop: one Esc in the terminal, pressed by the runner only while claude
    /// works and nothing is asking (ov-368).
    private var stopButton: some View {
        Button {
            Task { await model.stop() }
        } label: {
            Image(systemName: "stop.circle.fill")
                .font(.title)
                .foregroundStyle(.secondary)
                .frame(width: 44, height: 44)
                .contentShape(.circle)
        }
        .buttonStyle(.plain)
        .disabled(model.pressing != nil)
        .accessibilityLabel("Stop")
        .accessibilityIdentifier("native-stop")
    }

    private var sendButton: some View {
        Button {
            Task { await model.send() }
        } label: {
            Image(systemName: "arrow.up.circle.fill")
                .font(.title)
                .foregroundStyle(model.canSend ? AnyShapeStyle(.tint) : AnyShapeStyle(.tertiary))
                // An edge while it can't send, so it still reads on
                // light paper.
                .overlay {
                    if !model.canSend { Circle().strokeBorder(.secondary, lineWidth: 1) }  // style-exempt: the disabled Send's edge on light paper
                }
                .frame(width: 44, height: 44)
                .contentShape(.circle)
        }
        .buttonStyle(.plain)
        .disabled(!model.canSend)
        .accessibilityLabel("Send")
        .accessibilityIdentifier("native-send")
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
            case .handoff, .panel:
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

/// The composer's text view: as many lines as the draft has up to six, then
/// it scrolls. Return is a new line; ⌘↩ sends from a hardware keyboard (the
/// field's own key command, because the text view took ⌘↩ as a new line before
/// a button's shortcut saw it: `ComposerField`); and an image on the pasteboard
/// is handed to `onPasteImages` rather than to the text.
struct NativeComposerField: UIViewRepresentable {
    @Binding var text: String
    /// How tall the typed text is, reported after it changes rather than
    /// negotiated during layout (`AgentView`'s `ComposerTextView` learned
    /// this: a text view given a flexible frame takes the largest one).
    @Binding var height: CGFloat
    /// What VoiceOver calls the field: `Message Claude`, `Message Codex`.
    var label = "Message Claude"
    let onSend: () -> Void
    /// Nil leaves a paste to the text.
    var onPasteImages: (([Data]) -> Void)?
    /// Counts the times something put words in the field from outside (the
    /// suggestion, ov-409): on each, the field takes the keyboard with the
    /// caret after them.
    var claimsKeyboard = 0
    /// Tab on a hardware keyboard: whether it takes a suggestion now, and
    /// taking it.
    var offersTab: (() -> Bool)?
    var onTab: (() -> Void)?

    static var font: UIFont { .preferredFont(forTextStyle: .body) }
    static var lineHeight: CGFloat { font.lineHeight + 2 * inset }
    static let inset: CGFloat = 2
    static let mostLines: CGFloat = 6

    func makeUIView(context: Context) -> ComposerField {
        let view = ComposerField()
        view.font = Self.font
        view.adjustsFontForContentSizeCategory = true
        view.backgroundColor = .clear
        view.isScrollEnabled = true
        view.textContainerInset = UIEdgeInsets(top: Self.inset, left: 0, bottom: Self.inset, right: 0)
        view.textContainer.lineFragmentPadding = 0
        view.autocapitalizationType = .sentences
        view.text = text
        view.delegate = context.coordinator
        view.accessibilityIdentifier = "native-composer"
        view.accessibilityLabel = label
        view.onCommandReturn = onSend
        view.onPasteImages = onPasteImages
        view.offersTab = offersTab
        view.onTab = onTab
        DispatchQueue.main.async { context.coordinator.report(view) }
        return view
    }

    func updateUIView(_ view: ComposerField, context: Context) {
        context.coordinator.parent = self
        view.accessibilityLabel = label
        view.onCommandReturn = onSend
        view.onPasteImages = onPasteImages
        view.offersTab = offersTab
        view.onTab = onTab
        // Measured here as well as on change: at `makeUIView` the view has no
        // width yet.
        context.coordinator.report(view)
        // Only a change from outside the field's own typing (a send clearing
        // it), so typing never resets the undo stack or marked text.
        guard view.text != text else { return }
        view.text = text
        context.coordinator.report(view)
        if claimsKeyboard != context.coordinator.claimed {
            context.coordinator.claimed = claimsKeyboard
            view.selectedRange = NSRange(location: (text as NSString).length, length: 0)
            DispatchQueue.main.async { view.becomeFirstResponder() }
        }
    }

    func makeCoordinator() -> Coordinator { Coordinator(self) }

    final class Coordinator: NSObject, UITextViewDelegate {
        var parent: NativeComposerField
        var claimed: Int
        init(_ parent: NativeComposerField) {
            self.parent = parent
            claimed = parent.claimsKeyboard
        }

        func textViewDidChange(_ textView: UITextView) {
            parent.text = textView.text
            report(textView)
        }

        func report(_ textView: UITextView) {
            let width = textView.bounds.width
            guard width > 0 else { return }
            let fitted = textView.sizeThatFits(CGSize(width: width, height: .greatestFiniteMagnitude)).height
            let most = NativeComposerField.font.lineHeight * NativeComposerField.mostLines + 2 * NativeComposerField.inset
            let clamped = min(max(fitted, NativeComposerField.lineHeight), most)
            guard abs(clamped - parent.height) > 0.5 else { return }
            let binding = parent.$height
            DispatchQueue.main.async { binding.wrappedValue = clamped }
        }
    }
}
