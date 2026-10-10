import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// A message's attachments both ways (ov-454): a file of any kind dropped or
/// pasted into the composer goes with the message where the runner takes
/// files, and a prompt's images show above it as thumbnails, fetched from the
/// runner, written where Quick Look opens them.
@MainActor
@Suite(.serialized)
struct NativeAttachmentTests {
    typealias C = NativeComposerTests

    /// A file of the test's own, `name`, holding `bytes`.
    static func file(_ name: String, _ bytes: Data) throws -> URL {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent("fc-attach-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        let url = dir.appendingPathComponent(name)
        try bytes.write(to: url)
        return url
    }

    /// A pasteboard of the test's own naming `urls`, as a Finder copy or a
    /// drag from Finder puts them.
    static func pasteboard(with urls: [URL]) -> NSPasteboard {
        let board = NSPasteboard(name: NSPasteboard.Name("fc-attach-test-\(UUID().uuidString)"))
        board.clearContents()
        board.writeObjects(urls as [NSURL])
        return board
    }

    @Test("A dropped PDF waits as a chip and goes with the message, by name")
    func aDroppedFileIsSent() async throws {
        let c = try await C.composer(rich: true)
        defer { c.window.close() }
        c.model.takesFiles = true
        let pdf = Data("%PDF-1.7 a report".utf8)
        let drop = NativeComposerPasteTests.Drop(Self.pasteboard(with: [try Self.file("Q3 report.pdf", pdf)]))
        #expect(c.text.draggingEntered(drop) == .copy)
        #expect(c.text.performDragOperation(drop))
        #expect(c.model.files.map(\.name) == ["Q3 report.pdf"])
        #expect(c.text.string.isEmpty, "not its path as text")
        await NativeAgentTests.settle(c.window)
        #expect(c.seen.ids.contains("native-file-chip"))

        C.type(c.text, "summarize this")
        C.press(c.text, shift: false)
        await C.until("the send") { await !c.sink.sent.isEmpty && c.model.files.isEmpty }
        let files = await c.sink.files
        #expect(files.count == 1 && files[0].map(\.name) == ["Q3 report.pdf"] && files[0].first?.data == pdf)
        #expect(await c.sink.sent == ["summarize this"])
    }

    @Test("Without compose_files a file isn't taken, and the text view drops its path as before")
    func withoutFilesAFileIsNotTaken() async throws {
        let c = try await C.composer(rich: true)
        defer { c.window.close() }
        let board = Self.pasteboard(with: [try Self.file("notes.txt", Data("n".utf8))])
        #expect(c.text.onImages?(board) == false)
        #expect(c.model.files.isEmpty)
    }

    @Test("A file past 16 MB is left out, with its sentence")
    func aFileTooLargeIsSaid() async throws {
        let c = try await C.composer(rich: true)
        defer { c.window.close() }
        c.model.takesFiles = true
        let big = try Self.file("dump.bin", Data(count: ComposeFile.largest + 1))
        #expect(c.model.attach(fileURLs: [big]))
        #expect(c.model.files.isEmpty && c.model.issue == .said(NativePaneModel.fileTooLarge))
    }

    @Test("A queued echo settles against the transcript's text, its file paths left out")
    func anEchoSettlesPastFilePaths() {
        let typed = #""/Users/a/Library/Application Support/FC/pastes/compose-0199aa-Q3report.pdf" summarize this"#
        #expect(NativePaneModel.words(typed) == NativePaneModel.words("summarize this"))
        #expect(NativePaneModel.words("/tmp/fc/pastes/compose-01-notes.txt hi") == NativePaneModel.words("hi"))
    }

    // MARK: - A prompt's images

    /// A runner that answers each image with `png`, counting the asks.
    actor StandInImages: PromptImageSource {
        let png: Data
        var asked: [String] = []
        init(_ png: Data) { self.png = png }
        func promptImage(terminal: String, row: String, index: Int) async throws -> Data {
            asked.append("\(row)#\(index)")
            return png
        }
    }

    @Test("A prompt's images show above it as thumbnails, written where Quick Look opens them")
    func aPromptsImagesShow() async throws {
        let png = C.png()
        let images = StandInImages(png)
        let model = NativeAgentTests.model(try NativeAgentTests.terminal())
        model.promptImages.source = images
        model.store.apply(try await model.store.ledger.page(NativeAgentTests.page([
            NativeAgentTests.row(0, "turn:p1", ["Turn": [
                "prompt": "What's wrong here? [Image #3] [Image #4]", "origin": "Typed", "background_running": 0,
                "images": [["mime": "image/png"], ["mime": "image/png"]],
            ]]),
        ])))
        let seen = NativeAgentTests.Seen()
        let window = NativeAgentTests.window(
            NativeAgentTests.Probe(seen: seen, content: NativeAgentView(model: model, isFocused: true, showTerminal: {})))
        defer { window.close() }
        await C.until("the thumbnails") {
            await NativeAgentTests.settle(window, 40)
            return seen.ids.contains("native-prompt-image")
                && (0..<2).allSatisfy { if case .shown = model.promptImages.state(row: "turn:p1", index: $0) { true } else { false } }
        }
        #expect(seen.ids.contains("native-prompt-images"))
        #expect(Set(await images.asked) == ["turn:p1#0", "turn:p1#1"])
        guard case .shown(_, let url) = model.promptImages.state(row: "turn:p1", index: 1) else { return }
        #expect(url.pathExtension == "png")
        #expect(try Data(contentsOf: url) == png, "the whole image, for Quick Look")
        try? FileManager.default.removeItem(at: model.promptImages.folder)
    }

    @Test("A runner without prompt images shows the message alone")
    func noSourceNoThumbnails() async throws {
        let model = NativeAgentTests.model(try NativeAgentTests.terminal())
        model.store.apply(try await model.store.ledger.page(NativeAgentTests.page([
            NativeAgentTests.row(0, "turn:p1", ["Turn": [
                "prompt": "Look [Image #1]", "origin": "Typed", "background_running": 0, "images": [["mime": "image/png"]],
            ]]),
        ])))
        let seen = NativeAgentTests.Seen()
        let window = NativeAgentTests.window(
            NativeAgentTests.Probe(seen: seen, content: NativeAgentView(model: model, isFocused: true, showTerminal: {})))
        defer { window.close() }
        await NativeAgentTests.settle(window)
        #expect(seen.ids.contains("native-row-turn:p1"))
        #expect(!seen.ids.contains("native-prompt-images"))
    }
}
