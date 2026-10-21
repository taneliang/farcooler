import Foundation
import Testing

@testable import AgentKit

// ov-622, the review's findings: a pane's photos go only once it has really
// gone, a resumed session's turns are never cached, and a sent message's
// images are kept as a pair under the caps.

struct PromptImageDiskTests {
    private static func scratch() -> PromptImageDisk {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("prompt-disk-\(UUID().uuidString)", isDirectory: true)
        return PromptImageDisk(files: PaneImageFiles(root: root, paneCap: 10_000, totalCap: 100_000, whenFull: .evictOldest, maxAge: 3600))
    }

    @Test("An image is kept with its thumbnail, and comes back whole")
    func aPairComesBack() throws {
        let disk = Self.scratch()
        defer { try? FileManager.default.removeItem(at: disk.files.root) }
        let url = disk.keep(pane: "p", row: "turn:1", index: 0, mime: "image/png", original: Data(repeating: 1, count: 50), thumbnail: Data(repeating: 2, count: 5))
        let held = try #require(disk.held(pane: "p", row: "turn:1", index: 0, mime: "image/png"))
        #expect(held.original == url && held.thumbnail == Data(repeating: 2, count: 5))
        #expect(try Data(contentsOf: held.original).count == 50)
        #expect(disk.held(pane: "p", row: "turn:1", index: 1, mime: "image/png") == nil)
        #expect(disk.held(pane: "other", row: "turn:1", index: 0, mime: "image/png") == nil)
        #expect(disk.kept(pane: "p").count == 2)
    }

    @Test("A pair that lost a file is a miss")
    func halfAPairIsAMiss() {
        let disk = Self.scratch()
        defer { try? FileManager.default.removeItem(at: disk.files.root) }
        _ = disk.keep(pane: "p", row: "turn:1", index: 0, mime: "image/png", original: Data(count: 10), thumbnail: Data(count: 2))
        disk.files.remove(named: disk.names(row: "turn:1", index: 0, mime: "image/png").thumbnail, pane: "p")
        #expect(disk.held(pane: "p", row: "turn:1", index: 0, mime: "image/png") == nil)
    }

    @Test("A resumed session's turns are never cached, since their ids name another turn in another session")
    func resumedTurnsAreNeverCached() {
        let disk = Self.scratch()
        defer { try? FileManager.default.removeItem(at: disk.files.root) }
        #expect(!PromptImageDisk.cacheable("turn:resumed:3") && PromptImageDisk.cacheable("turn:abc") && PromptImageDisk.cacheable("turn:p1"))
        let url = disk.keep(pane: "p", row: "turn:resumed:3", index: 0, mime: "image/png", original: Data(count: 10), thumbnail: Data(count: 2))
        #expect(url == nil)
        #expect(disk.held(pane: "p", row: "turn:resumed:3", index: 0, mime: "image/png") == nil)
        #expect(disk.files.totalBytes == 0)
    }

    @Test("Two images of one pane that sanitize alike keep apart")
    func similarRowsKeepApart() {
        let disk = Self.scratch()
        #expect(
            disk.names(row: "turn:a/b", index: 0, mime: "image/png").original != disk.names(row: "turn:a:b", index: 0, mime: "image/png").original)
    }

    @Test("A pair too big for the pane's cap is not kept, and leaves nothing behind")
    func aTooBigPairIsNotKept() {
        let disk = Self.scratch()
        defer { try? FileManager.default.removeItem(at: disk.files.root) }
        #expect(disk.keep(pane: "p", row: "turn:1", index: 0, mime: "image/png", original: Data(count: 20_000), thumbnail: Data(count: 2)) == nil)
        #expect(disk.files.totalBytes == 0)
    }

    @Test("A folder is the app's own: its bundle's, under the user's directory")
    func theFolderIsTheBundles() {
        let url = PaneImageFiles.appFolder(.cachesDirectory, "prompt-images")
        #expect(url.lastPathComponent == "prompt-images")
        #expect(url.deletingLastPathComponent().lastPathComponent == (Bundle.main.bundleIdentifier ?? "Far Cooler"))
    }
}

struct MissingPanesTests {
    private let start = Date(timeIntervalSince1970: 1_000_000)

    @Test("One read without a pane doesn't make it gone")
    func oneMissingReadKeepsIt() {
        var panes = MissingPanes()
        #expect(panes.read(["a", "b"], at: start).isEmpty)
        #expect(panes.read(["a"], at: start + 5).isEmpty, "a single read that lacked b")
        #expect(panes.read(["a"], at: start + 20).isEmpty, "a second read, but not 30 s on")
    }

    @Test("Missing from two reads at least 30 s apart, it is gone, once")
    func twoReadsMakeItGone() {
        var panes = MissingPanes()
        _ = panes.read(["a", "b"], at: start)
        #expect(panes.read(["a"], at: start + 5).isEmpty)
        #expect(panes.read(["a"], at: start + 36) == ["b"])
        #expect(panes.read(["a"], at: start + 90).isEmpty, "told twice")
    }

    @Test("A pane back in between starts the count again")
    func aPaneBackStartsAgain() {
        var panes = MissingPanes()
        _ = panes.read(["a", "b"], at: start)
        #expect(panes.read(["a"], at: start + 5).isEmpty)
        #expect(panes.read(["a", "b"], at: start + 20).isEmpty, "b is back")
        #expect(panes.read(["a"], at: start + 40).isEmpty, "the first read of a new absence")
        #expect(panes.read(["a"], at: start + 80) == ["b"])
    }

    @Test("The first read only says what there is")
    func theFirstReadIsABaseline() {
        var panes = MissingPanes()
        #expect(panes.read([], at: start).isEmpty)
        #expect(panes.read(["a"], at: start + 60).isEmpty)
    }
}
