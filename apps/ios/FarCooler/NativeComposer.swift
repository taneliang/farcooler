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
    /// The height the conversation has above the keyboard (ov-614): the
    /// composer grows with its text to a share of it, then scrolls inside,
    /// so it never pushes its own buttons under the keyboard.
    var room: CGFloat = .infinity

    /// The side of a button's target, and of its glyph's room.
    private static let targetSide: CGFloat = 44
    /// What the attach glyph (`photo.badge.plus` at `.title3`, about 26 pt wide)
    /// leaves each side of its target: a chip strip is inset by it, so the
    /// first chip's edge and the glyph's ink share a line.
    private static let glyphSlack: CGFloat = 9

    /// Concentric with the 44 pt buttons inside it, which sit `Spacing.tight`
    /// in from its edge (ov-611): a capsule at one line, and the same corners
    /// as it grows.
    static let radius = Radius.controlBar
    static let shape = RoundedRectangle(cornerRadius: radius, style: .continuous)

    /// The tallest the text gets before it scrolls: six lines, or the share
    /// of the room above the keyboard the composer may take, less the photos
    /// over it, and never less than two lines (ov-614).
    private var tallestField: CGFloat {
        let six = NativeComposerField.font.lineHeight * NativeComposerField.mostLines + 2 * NativeComposerField.inset
        let photos: CGFloat = model.images.isEmpty ? 0 : Self.chipSide + Spacing.inset + Spacing.group
        let share = room * Self.roomShare - photos - 2 * Spacing.tight
        let two = NativeComposerField.font.lineHeight * 2 + 2 * NativeComposerField.inset
        return max(min(six, share), two)
    }

    /// The share of the room above the keyboard the composer may grow to.
    static let roomShare: CGFloat = 0.4
    /// A photo chip's side.
    static let chipSide: CGFloat = 56

    @State private var fieldHeight = NativeComposerField.lineHeight
    @State private var picked: [PhotosPickerItem] = []
    /// The screen's one margin (ov-611): the composer used `Spacing.inset`,
    /// 4 pt wider each side than everything above it.
    @Environment(\.screenMargin) private var margin

    var body: some View {
        VStack(alignment: .leading, spacing: Spacing.group) {
            if let issue = model.issue, issue != .handoff, issue != .panel {
                issueLine(issue)
            }
            // Photos above the text, inside the one surface, and attach and
            // Send on the bottom row whatever the text's height (ov-614).
            VStack(alignment: .leading, spacing: Spacing.group) {
                if !model.images.isEmpty { chips }
                HStack(alignment: .bottom, spacing: Spacing.group) {
                    // The glyphs stop growing at the largest standard size: in
                    // a 44 pt target an accessibility-size glyph ran into the
                    // text beside it (review 1). Their labels still scale.
                    if model.rich { attachButton.dynamicTypeSize(...DynamicTypeSize.xxxLarge) }
                    field
                    if model.offersStop { stopButton.dynamicTypeSize(...DynamicTypeSize.xxxLarge) }
                    sendButton.dynamicTypeSize(...DynamicTypeSize.xxxLarge)
                }
            }
            .padding(.leading, Spacing.section)
            .padding(.trailing, Spacing.tight)
            .padding(.vertical, Spacing.tight)
            .frame(minHeight: 44)
            // Never laid out shorter than its contents: squeezed by the tray
            // above it, the box kept one height and its text and photos drew
            // over its edges (ov-614).
            .fixedSize(horizontal: false, vertical: true)
            .surface(.floating, in: Self.shape)
            .clipShape(Self.shape)
            #if DEBUG
            // The surface's own edges, for the UI tests (ov-611, ov-614): a
            // container's frame is its children's, not its surface's.
            .background {
                Color.clear
                    .accessibilityElement()
                    .accessibilityIdentifier("native-composer-surface")
            }
            #endif
        }
        .padding(.horizontal, margin)
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
        .frame(height: min(fieldHeight, tallestField))
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
        #if DEBUG
        .overlay(alignment: .topTrailing) { FieldProbe() }
        #endif
    }

    #if DEBUG
    /// `overwrote=N`, for the UI tests: `NativeComposerProbe`.
    private struct FieldProbe: View {
        @ObservedObject var probe = NativeComposerProbe.shared

        var body: some View {
            Color.clear
                .frame(width: 1, height: 1)  // style-exempt: DEBUG probe: a 1 pt element the UI tests read, nothing drawn
                .allowsHitTesting(false)
                .accessibilityElement()
                .accessibilityIdentifier("native-composer-probe")
                .accessibilityValue("overwrote=\(probe.overwrites)")
        }
    }
    #endif

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
            // The badge stands 9 pt above its chip's corner.
            .padding(.top, Spacing.inset)
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
        .frame(width: Self.chipSide, height: Self.chipSide)
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
        // Three buttons beside the words squeeze them to a column on a phone:
        // with Bring Here, the words go above and the buttons below.
        if issue == .draftInTerminal, model.offersBringHere {
            VStack(alignment: .leading, spacing: Spacing.tight) {
                Text(AgentConversation.draftInTerminalBring)
                HStack(spacing: Spacing.group) {
                    Spacer(minLength: 0)
                    Button("Bring Here") { Task { await model.bringHere() } }
                        .disabled(model.bringing)
                        .accessibilityIdentifier("native-bring-here")
                    Button("Show Terminal", action: showTerminal)
                    Button("Dismiss") { model.issue = nil }
                }
            }
            .font(.callout)
            .padding(.horizontal, Spacing.inset)
            .padding(.vertical, Spacing.group)
            .surface(.floating, in: .card)
            .accessibilityElement(children: .contain)
            .accessibilityIdentifier("native-send-issue")
        } else {
            plainIssueLine(issue)
        }
    }

    private func plainIssueLine(_ issue: AgentConversation.SendIssue) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: Spacing.group) {
            switch issue {
            case .draftInTerminal:
                // R-28: refused, with the terminal (Bring Here is drawn above).
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
        // it), so typing never resets the undo stack, and never while an
        // input method holds marked text, which setting the text would throw
        // away (the Mac's field refuses the same).
        guard AgentConversation.fieldTakesDraft(text, showing: view.text, composing: view.markedTextRange != nil, echo: context.coordinator.echo)
        else { return }
        #if DEBUG
        if !(view.text ?? "").isEmpty, !text.isEmpty { NativeComposerProbe.shared.overwrote() }
        #endif
        view.text = text
        context.coordinator.echo.took(text)
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
        /// What the person typed since the field last took a draft from
        /// outside, so a draft that lags the typing is never written back.
        var echo = AgentConversation.DraftEcho()
        init(_ parent: NativeComposerField) {
            self.parent = parent
            claimed = parent.claimsKeyboard
        }

        func textViewDidChange(_ textView: UITextView) {
            echo.typed(textView.text)
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
