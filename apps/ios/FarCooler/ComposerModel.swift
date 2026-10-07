import SwiftUI
import UIKit

/// What a pane's composer holds, kept above the layout that draws it (ov-357).
///
/// On an iPad the chat's composer is two views: the column's, pinned inline,
/// at regular width, and the keyboard's, an input accessory, at compact
/// width. Crossing 700 points (Split View, Stage Manager, a turn) builds the
/// other and drops the first, and everything the first kept in its own
/// `@State` went with it: the picked photos, the caret and the keyboard. Only
/// the words crossed, through `PaneDraftStore`.
///
/// `AgentView` owns one of these for as long as the pane lives, and either
/// composer binds to it. A composer that appears reads the caret and the
/// focus back out of it (`ComposerTextView.makeUIView`).
@MainActor
final class ComposerModel: ObservableObject {
    @Published var text = ""
    /// The selection in UTF-16 units, as `UITextView.selectedRange` has it, so
    /// a new field is handed back exactly the range the old one held, a
    /// selected span included.
    @Published var selection = NSRange(location: 0, length: 0)
    @Published var attachments: [ComposerAttachment] = []
    /// Why the last attachment did not attach. Shown in the composer.
    @Published var attachmentError: String?
    /// Whether the field is being typed in: the keyboard is up for it, Hide
    /// Keyboard has something to do, and a composer built next takes focus.
    /// Written from the text view's own begin and end callbacks, so a resign
    /// from anywhere reaches it; not from a field being taken down.
    @Published var isFocused = false
    /// Whether the pane is in the iPad's column, kept by `AgentView`, so a
    /// field taken down can be told as a hand-off (the width changed) from a
    /// hidden pane (it didn't).
    var inColumn = false
    private weak var field: UITextView?
    private var handoffPending = false
    /// Bumped by each hand-off, so an earlier one's timer can't clear a later one's focus.
    private var handoffGeneration = 0
    private weak var handoffFrom: UITextView?

    /// A composer field was made. If a field was waiting for its replacement,
    /// this is it.
    func fieldMade(_ new: UITextView) {
        field = new
        handoffPending = false
    }

    /// The newest field, made for the column or for the keyboard, left its
    /// window while it had the keyboard.
    ///
    /// Judged on the next turn of the main queue, by when the pane's mode
    /// has been updated: a pane in the mode the field was made for is hidden,
    /// by a screen over it or another tab, nothing is coming and the
    /// reader's keyboard went with the field. A pane in the other mode is
    /// crossing 700 points, and the other composer is on its way, a
    /// moment later when it's the keyboard's: the focus is kept for it, and
    /// dropped if it never comes.
    func fieldTakenDown(_ old: UITextView, madeForColumn: Bool) {
        guard isFocused, field === old else { return }
        DispatchQueue.main.async { [weak self] in
            guard let self, isFocused, field === old else { return }
            guard madeForColumn != inColumn else {
                isFocused = false
                return
            }
            // `willMove(toWindow: nil)` and the dismantle both report one
            // take-down: one hand-off, one timer.
            if handoffPending, handoffFrom === old { return }
            handoffPending = true
            handoffFrom = old
            handoffGeneration += 1
            let generation = handoffGeneration
            DispatchQueue.main.asyncAfter(deadline: .now() + 1) { [weak self] in
                guard let self, handoffPending, generation == handoffGeneration else { return }
                handoffPending = false
                isFocused = false
            }
        }
    }

    /// A field began editing: the focus is somebody's again.
    func focusBegan() {
        handoffPending = false
        isFocused = true
    }

    /// The caret as a `Character` offset, which `activeToken(in:cursor:)`
    /// counts in. Set, it puts the caret there with nothing selected.
    var cursor: Int {
        get { Self.characterOffset(forUTF16Offset: selection.location, in: text) }
        set { selection = NSRange(location: Self.utf16Offset(forCharacterOffset: newValue, in: text), length: 0) }
    }

    /// Takes a picked photo's bytes into the strip, or says why not. The one
    /// path a photo takes, from the picker or from a harness standing in for
    /// it (`ComposerHarness`).
    func attach(_ data: Data?) {
        guard let data, let image = UIImage(data: data) else {
            attachmentError = "That photo could not be read. If it lives in "
                + "iCloud, open it in Photos first so it downloads."
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
        guard let (payload, payloadMime) = PromptImageBudget.fit(image, original: data, mime: mime) else {
            attachmentError = "That photo couldn’t be prepared to send."
            return
        }
        attachments.append(ComposerAttachment(image: image, data: payload, mime: payloadMime))
        attachmentError = nil
    }

    /// `UITextView.selectedRange` is UTF-16 code units; `activeToken` counts
    /// `Character`s. The two agree for plain ASCII commands and paths — the
    /// only content these pickers ever match against — and diverge only
    /// inside a multi-scalar grapheme cluster (an emoji, say), where landing
    /// mid-cluster falls back to the nearest end rather than crashing.
    static func characterOffset(forUTF16Offset utf16Offset: Int, in text: String) -> Int {
        guard
            let utf16Index = text.utf16.index(
                text.utf16.startIndex, offsetBy: utf16Offset, limitedBy: text.utf16.endIndex),
            let index = String.Index(utf16Index, within: text)
        else { return text.count }
        return text.distance(from: text.startIndex, to: index)
    }

    static func utf16Offset(forCharacterOffset characterOffset: Int, in text: String) -> Int {
        guard let index = text.index(text.startIndex, offsetBy: characterOffset, limitedBy: text.endIndex)
        else { return (text as NSString).length }
        return text.utf16.distance(from: text.utf16.startIndex, to: index.samePosition(in: text.utf16) ?? text.utf16.endIndex)
    }
}

/// `-composer-no-send-shortcut` (DEBUG): the Send button keeps no ⌘↩, so a test
/// can tell the field's key command from the button's shortcut. Only the
/// shortcut is dropped; both sends are the shipping code.
enum ComposerHarness {
    static var dropsSendShortcut: Bool {
        #if DEBUG
        CommandLine.arguments.contains("-composer-no-send-shortcut")
        #else
        false
        #endif
    }
}

struct ComposerAttachment: Identifiable {
    let id = UUID()
    let image: UIImage
    /// The bytes to send — the original when it already fits inside one control
    /// envelope, a resized JPEG when it did not. See `PromptImageBudget`.
    ///
    /// The `image` above stays the FULL-size one, because it is what the
    /// thumbnail is drawn from and shrinking that would show a worse picture
    /// than was actually sent.
    let data: Data
    let mime: String

    var payload: (mime: String, data: Data)? { (mime, data) }
}

#if DEBUG
/// A `com.farcooler.harness.composer-photo` notice picks a photo into the
/// composer, as the system's picker can't be driven from a test. It hands
/// `ComposerModel.attach` the bytes of a small PNG, which is the path a picked
/// photo takes once the picker has loaded it: the size budget, the strip and
/// the send all see it as they'd see any other.
struct ComposerPhotoHarness: ViewModifier {
    let model: ComposerModel

    func body(content: Content) -> some View {
        content.onReceive(NotificationCenter.default.publisher(for: HarnessTaps.composerPhoto)) { _ in
            let size = CGSize(width: 64, height: 64)
            let png = UIGraphicsImageRenderer(size: size).pngData { context in
                UIColor.systemTeal.setFill()  // style-exempt: a test photo's pixels, not UI
                context.fill(CGRect(origin: .zero, size: size))
            }
            model.attach(png)
        }
    }
}
#endif
