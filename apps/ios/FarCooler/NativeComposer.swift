import PhotosUI
import SwiftUI
import UIKit

/// The conversation's box (ov-373, ov-404), laid out like Messages (ov-677):
/// the photo button is its own glass circle beside a glass field, and the
/// field holds the text and one trailing action. Send goes through
/// `terminal.compose`: typed into claude's own box and submitted, or taken by
/// claude's queue while it works (R-29).
///
/// Where the runner has `compose` (`NativePaneModel.rich`), as on the Mac:
/// Return is a new line and the Send button sends (⌘↩ too, from a hardware
/// keyboard); photos come from the picker or a paste and wait as chips; a
/// slash command goes to claude's picker. The trailing action is Send with
/// something to send, Stop while claude works and the field is empty, and
/// nothing otherwise (`Action`); each waiting Queued row has Send Now
/// (ov-368). Without `compose`, one line and no photos.
struct NativeComposer: View {
    @ObservedObject var model: NativePaneModel
    let showTerminal: () -> Void
    /// The height the conversation has above the keyboard (ov-614): the
    /// composer grows with its text to a share of it, then scrolls inside,
    /// so it never pushes its own buttons under the keyboard.
    var room: CGFloat = .infinity
    @Environment(\.dynamicTypeSize) private var type

    /// The side of a button's target, the photo button's glass and the field
    /// at one line.
    static let targetSide: CGFloat = 44
    /// The +'s label: the glass button style pads it to `targetSide`.
    static let plusSide: CGFloat = 32
    /// The trailing action's capsule, 7 pt in from the field's edge on both
    /// axes, and so concentric with it (`Radius.controlBar`): Messages' Send
    /// measures 39 × 29 pt in a 42 pt field.
    static let actionSize = CGSize(width: 38, height: 30)

    /// A capsule at one line, keeping those corners as it grows.
    static let radius = Radius.controlBar
    static let shape = RoundedRectangle(cornerRadius: radius, style: .continuous)

    /// The tallest the text gets before it scrolls: six lines, or the share
    /// of the room above the keyboard the composer may take, less the photos
    /// over it, and never less than two lines (ov-614).
    private var tallestField: CGFloat {
        let six = NativeComposerField.font.lineHeight * NativeComposerField.mostLines + 2 * NativeComposerField.inset
        let photos: CGFloat = model.images.isEmpty ? 0 : Self.chipSide + 2 * Spacing.group
        let share = room * Self.roomShare - photos - 2 * Spacing.group
        let two = NativeComposerField.font.lineHeight * 2 + 2 * NativeComposerField.inset
        return max(min(six, share), two)
    }

    /// The share of the room above the keyboard the composer may grow to.
    static let roomShare: CGFloat = 0.4
    /// A photo chip's side.
    static let chipSide: CGFloat = 56

    /// The field's one trailing button (ov-677). Content decides it, never
    /// claude's state alone, so a Send about to be tapped can't turn into
    /// Stop under the finger: the Claude app's shared button did exactly that
    /// when a tool started (claude-code#100239).
    enum Action: Equatable {
        /// Something to send: words or a photo. Queued while claude works.
        case send
        /// claude works and the field is empty.
        case stop

        static func showing(hasContent: Bool, offersStop: Bool) -> Action? {
            if hasContent { return .send }
            return offersStop ? .stop : nil
        }
    }

    /// How long a Stop that just appeared ignores taps: the field clears on a
    /// send, and while claude works Stop takes Send's place under a finger
    /// that may tap again.
    static let stopArming: Duration = .milliseconds(500)

    @State private var fieldHeight = NativeComposerField.lineHeight
    @State private var picked: [PhotosPickerItem] = []
    @State private var stopArmed = false
    /// A photo's held preview is up (`chip`).
    @State private var previewing = false
    /// The screen's one margin (ov-611).
    @Environment(\.screenMargin) private var margin

    private var action: Action? { Action.showing(hasContent: model.hasContent, offersStop: model.offersStop) }

    var body: some View {
        VStack(alignment: .leading, spacing: Spacing.group) {
            if let issue = model.issue, issue != .handoff, issue != .panel {
                issueLine(issue)
            }
            // The photo button and the field as one family of glass, spaced
            // closer than the gap between them so they never blend at rest.
            FloatingGroup(spacing: Spacing.group) {
                HStack(alignment: .bottom, spacing: Spacing.inset) {
                    // The glyphs stop growing at the largest standard size: in
                    // a 44 pt target an accessibility-size glyph ran into the
                    // text beside it (review 1). Their labels still scale.
                    if model.rich { attachButton.dynamicTypeSize(...DynamicTypeSize.xxxLarge) }
                    box
                }
            }
        }
        .padding(.horizontal, margin)
        .padding(.vertical, Spacing.group)
        .task(id: action) {
            stopArmed = false
            guard action == .stop else { return }
            try? await Task.sleep(for: Self.stopArming)
            if !Task.isCancelled { stopArmed = true }
        }
        // Its top edge, for `NativeAgentViewTests`: the line the last row
        // has to clear (ov-383's composer report).
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("native-composer-stack")
    }

    /// The field's glass: photos over the text, and the trailing action on
    /// the text's bottom row whatever its height (ov-614).
    private var box: some View {
        VStack(alignment: .leading, spacing: 0) {
            if !model.images.isEmpty { chips }
            HStack(alignment: .bottom, spacing: Spacing.tight) {
                field
                if let action { actionButton(action).dynamicTypeSize(...DynamicTypeSize.xxxLarge) }
            }
            .padding(.leading, Spacing.section)
            .padding(.trailing, action == nil ? Spacing.section : Spacing.tight)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        // Never laid out shorter than its contents: squeezed by the tray
        // above it, the box kept one height and its text and photos drew
        // over its edges (ov-614).
        .fixedSize(horizontal: false, vertical: true)
        // Clipped BEFORE the glass: the text and photos stay inside the
        // field's shape, and the glass's own response to a press, which
        // swells past that shape, isn't cut off by it.
        .clipShape(Self.shape)
        // Interactive, as Messages' field is (ov-789): pressed or dragged, it
        // swells and lights under the finger. The field itself is the thing
        // touched; Send inside it keeps its own plain response.
        //
        // Not while a held photo's menu is up: the menu's Remove sits over
        // the field, and interactive glass under it took the tap, so Remove
        // did nothing (measured, GlassUITests).
        .surface(.floating, in: Self.shape, interactive: !previewing)
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
                // The whole of it, or "Message" where the type is too large
                // for the name to fit one line: cut to "Messag…" it read as
                // a typo (ov-685).
                Text(model.hint ?? (type.isAccessibilitySize ? "Message" : "Message \(model.agent)"))
                    .lineLimit(1)
                    .foregroundStyle(.quietInk)
                    // `NativeComposerField`'s own `textContainerInset`, so the
                    // placeholder sits where the first typed letter will land.
                    .padding(.top, NativeComposerField.inset)
                    .allowsHitTesting(false)
                    .accessibilityHidden(model.hint == nil)
                    .accessibilityIdentifier(model.hint == nil ? "native-placeholder" : "native-hint")
            }
        }
        // One line centered in the field's 44 pt, so it and the trailing
        // action share a middle; grown, 10 pt from the edge to the text.
        .padding(.vertical, Spacing.group)
        .frame(minHeight: Self.targetSide, alignment: .center)
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
                .accessibilityValue("overwrote=\(probe.overwrites) \(probe.fade)")
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
                .foregroundStyle(.quietInk)
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

    /// The photo picker, outside the field as Messages' + is: photos added as
    /// chips, up to what the message takes.
    private var attachButton: some View {
        PhotosPicker(
            selection: $picked, maxSelectionCount: max(1, model.imageRoom), matching: .images
        ) {
            Image(systemName: "plus")
                .font(.title3)
                .frame(width: Self.plusSide, height: Self.plusSide)
        }
        // The system's glass button, as Messages' + is (ov-789): its glass,
        // its press and drag response, and its place in the composer's one
        // glass container, rather than glass drawn under a plain button that
        // never reacted.
        .buttonStyle(.glass)
        .buttonBorderShape(.circle)
        // A `PhotosPicker` tints its label with the app's accent, and a
        // foreground style on the label doesn't survive it (ov-404 review 1).
        .tint(.primary)
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

    /// The images waiting to go, inside the field over the text, each with a
    /// button to take it out, and a hairline under them as in Messages.
    private var chips: some View {
        ScrollView(.horizontal) {
            HStack(spacing: Spacing.group) {
                ForEach(model.images) { image in chip(image) }
            }
            .padding(Spacing.group)
        }
        .scrollIndicators(.never)
        // The remove buttons' 44 pt targets reach past the strip; the field's
        // own shape still clips what's drawn.
        .scrollClipDisabled()
        .separator(.listEdge)
        .accessibilityIdentifier("native-image-chips")
    }

    /// Concentric with the field, `Spacing.group` in from its corner.
    private static let chipShape = RoundedRectangle(
        cornerRadius: Radius.concentric(outer: radius, padding: Spacing.group), style: .continuous)

    private func chip(_ image: OutgoingImage) -> some View {
        Group {
            if let thumbnail = model.thumbnails[image.id] {
                Image(uiImage: thumbnail).resizable().aspectRatio(contentMode: .fill)
            } else {
                Image(systemName: "photo").foregroundStyle(.quietInk)
            }
        }
        .frame(width: Self.chipSide, height: Self.chipSide)
        .surface(.inset, in: Self.chipShape)
        .clipShape(Self.chipShape)
        // Held, the photo as it will go, large, with Remove under it, as
        // Messages does with a photo waiting in its field (ov-792). Before
        // the remove badge, so the badge isn't lifted with it.
        .contentShape(.contextMenuPreview, Self.chipShape)
        .contextMenu {
            Button("Remove", systemImage: "trash", role: .destructive) { model.detach(image.id) }
                .accessibilityIdentifier("native-image-preview-remove")
        } preview: {
            OutgoingImagePreview(image: image)
                .onAppear { previewing = true }
                .onDisappear { previewing = false }
        }
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
            // Inside the photo's corner, as in Messages: the badge's middle
            // 14 pt in from each edge, its 44 pt target centered on it and
            // inside the field's padding.
            .offset(x: Spacing.group, y: -Spacing.group)
        }
        .accessibilityElement(children: .contain)
        .accessibilityLabel("Photo")
        .accessibilityIdentifier("native-image-chip")
    }

    /// Stop's capsule and square: the label color and the page's, as UIKit
    /// colors, because `.primary` inside glass is drawn vibrant, a pale gray
    /// that read as a disabled button.
    private static let stopFill = Color(uiColor: .label)
    private static let stopInk = Color(uiColor: .systemBackground)

    /// The field's trailing button (`Action`): Send, a filled accent capsule,
    /// gray while it can't send; or Stop, one Esc pressed by the runner only
    /// while claude works and nothing is asking (ov-368), in the label color
    /// so it never reads as Send.
    private func actionButton(_ action: Action) -> some View {
        let stop = action == .stop
        let enabled = stop ? stopArmed && model.pressing == nil : model.canSend
        return Button {
            Task {
                if stop { await model.stop() } else { await model.send() }
            }
        } label: {
            // Only the glyph and the fill animate, scoped to them: an
            // animation on the stack also animated the field's growth when a
            // paste brought Send and new lines in one update, and the text
            // view clipped its first line for a frame.
            Image(systemName: stop ? "stop.fill" : "arrow.up")
                .font(stop ? .footnote.weight(.bold) : .body.weight(.semibold))
                .animation(Self.actionSpring) { glyph in
                    glyph
                        .contentTransition(.symbolEffect(.replace))
                        .foregroundStyle(stop ? AnyShapeStyle(Self.stopInk) : enabled ? AnyShapeStyle(.white) : AnyShapeStyle(.tertiary))
                }
                .frame(width: Self.actionSize.width, height: Self.actionSize.height)
                .animation(Self.actionSpring) { capsule in
                    capsule.background(
                        stop ? AnyShapeStyle(Self.stopFill) : enabled ? AnyShapeStyle(.tint) : AnyShapeStyle(.quaternary),
                        in: .capsule)
                }
                .frame(width: Self.targetSide, height: Self.targetSide)
                .contentShape(.rect)
        }
        // Stop's 0.5 s guard: disabled, but not dimmed (`.plain` dimmed it to
        // gray for the half second, which read as a flicker); VoiceOver hears
        // why it waits. A Send that can't send draws its own gray.
        .buttonStyle(ActionStyle())
        .disabled(!enabled)
        .modifier(Arrives())
        // One element whatever it says: Send turning into Stop keeps the
        // view, so VoiceOver's focus stays on it and reads the new label.
        .accessibilityLabel(stop ? "Stop" : "Send")
        .accessibilityHint(stop && !stopArmed ? "Available in a moment." : "")
        .accessibilityIdentifier(stop ? "native-stop" : "native-send")
    }

    static let actionSpring = Animation.spring(duration: 0.25)

    /// The label as drawn, lighter while pressed, and never dimmed by
    /// `isEnabled`.
    private struct ActionStyle: ButtonStyle {
        func makeBody(configuration: Configuration) -> some View {
            configuration.label.opacity(configuration.isPressed ? 0.7 : 1)
        }
    }

    /// The action's arrival: it scales and fades in by itself, so the field
    /// beside it takes its new width at once rather than animating with it.
    private struct Arrives: ViewModifier {
        @State private var shown = false

        func body(content: Content) -> some View {
            content
                .scaleEffect(shown ? 1 : 0.5)
                .opacity(shown ? 1 : 0)
                .onAppear { withAnimation(NativeComposer.actionSpring) { shown = true } }
        }
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
        #if DEBUG
        view.onFade = { NativeComposerProbe.shared.faded($0) }
        #endif
        DispatchQueue.main.async { context.coordinator.report(view) }
        // A change of text size from outside the app (Control Center, Settings)
        // grows the field's lines before SwiftUI knows: measured again then, or
        // the box kept its old height and clipped the new lines (ov-685).
        view.registerForTraitChanges([UITraitPreferredContentSizeCategory.self]) { [weak coordinator = context.coordinator] (field: ComposerField, _) in
            DispatchQueue.main.async { coordinator?.report(field) }
        }
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

        /// The caret's line may have moved into an edge's fade, or out of
        /// it (`ComposerField.fade`).
        func textViewDidChangeSelection(_ textView: UITextView) {
            textView.setNeedsLayout()
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
