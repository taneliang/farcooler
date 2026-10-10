import AppKit
import Testing

@testable import Far_Cooler

/// How a paste and a drop reach the native composer through AppKit's own
/// route (ov-454): Edit > Paste is enabled for a copied screenshot, and a
/// drop of image data alone is accepted, before `paste` or
/// `performDragOperation` runs. A plain-text view refuses both on its own,
/// which is why ⌘V with a screenshot on the clipboard only beeped.
@MainActor
@Suite(.serialized)
struct NativeComposerPasteTests {
    typealias C = NativeComposerTests
    /// This suite's own pane, so no other suite's remembered view is touched.
    static let terminal = "0199aaaa-0000-7000-8000-0000000004a5"

    /// Whether the Paste menu item, targeted at `text` as the responder chain
    /// would, is enabled when the menu updates.
    static func pasteEnabled(_ text: ComposerTextView) -> Bool {
        let item = NSMenuItem(title: "Paste", action: #selector(NSText.paste(_:)), keyEquivalent: "v")
        item.target = text
        let menu = NSMenu()
        menu.autoenablesItems = true
        menu.addItem(item)
        menu.update()
        return item.isEnabled
    }

    @Test("Paste is enabled for a copied screenshot, and only where the runner takes images")
    func pasteIsEnabledForAScreenshot() async throws {
        let rich = try await C.composer(rich: true, terminal: Self.terminal)
        defer { rich.window.close() }
        rich.text.pasteboard = C.pasteboard(with: C.png())
        #expect(Self.pasteEnabled(rich.text))

        // Validation reads the box's own pasteboard, never the clipboard, so
        // whatever the clipboard holds can't enable it: without compose, a
        // screenshot alone leaves Paste off, and text turns it on.
        let plain = try await C.composer(rich: false, terminal: Self.terminal)
        defer { plain.window.close() }
        plain.text.pasteboard = C.pasteboard(with: C.png())
        #expect(!Self.pasteEnabled(plain.text), "no images to take, and no text")
        let words = NSPasteboard(name: NSPasteboard.Name("fc-composer-test-\(UUID().uuidString)"))
        words.clearContents()
        words.setString("fix the build", forType: .string)
        plain.text.pasteboard = words
        #expect(Self.pasteEnabled(plain.text))
    }

    @Test("A drop of image data alone is prepared for and attaches")
    func aDropOfImageDataAttaches() async throws {
        let c = try await C.composer(rich: true, terminal: Self.terminal)
        defer { c.window.close() }
        let drop = Drop(C.pasteboard(with: C.png()))
        #expect(c.text.draggingEntered(drop) == .copy)
        #expect(c.text.prepareForDragOperation(drop))
        #expect(c.text.performDragOperation(drop))
        #expect(c.model.images.count == 1)
    }

    /// A drag's information, as AppKit hands it to a destination.
    final class Drop: NSObject, NSDraggingInfo {
        let board: NSPasteboard
        init(_ board: NSPasteboard) { self.board = board }
        var draggingDestinationWindow: NSWindow? { nil }
        var draggingSourceOperationMask: NSDragOperation { .copy }
        var draggingLocation: NSPoint { .zero }
        var draggedImageLocation: NSPoint { .zero }
        var draggedImage: NSImage? { nil }
        var draggingPasteboard: NSPasteboard { board }
        var draggingSource: Any? { nil }
        var draggingSequenceNumber: Int { 1 }
        func slideDraggedImage(to screenPoint: NSPoint) {}
        var draggingFormation: NSDraggingFormation = .default
        var animatesToDestination = false
        var numberOfValidItemsForDrop = 1
        func enumerateDraggingItems(
            options: NSDraggingItemEnumerationOptions = [], for view: NSView?, classes: [AnyClass],
            searchOptions: [NSPasteboard.ReadingOptionKey: Any] = [:],
            using block: @escaping (NSDraggingItem, Int, UnsafeMutablePointer<ObjCBool>) -> Void
        ) {}
        var springLoadingHighlight: NSSpringLoadingHighlight { .none }
        func resetSpringLoading() {}
    }
}
