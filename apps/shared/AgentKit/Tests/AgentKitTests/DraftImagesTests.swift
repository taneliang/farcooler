import Foundation
import Testing

@testable import AgentKit

// ov-622: a composer's photos survive a restart, sit under a cap, and go when
// they are sent, taken out, or their pane goes.

@Suite(.serialized)
struct DraftImagesTests {
    private static func scratch() -> URL {
        FileManager.default.temporaryDirectory.appendingPathComponent("draft-images-\(UUID().uuidString)", isDirectory: true)
    }

    private static func store(
        _ root: URL, paneCap: Int = 1_000, totalCap: Int = 10_000, whenFull: PaneImageFiles.WhenFull = .refuse,
        maxAge: TimeInterval = 3600
    ) -> DraftImages {
        DraftImages(files: PaneImageFiles(root: root, paneCap: paneCap, totalCap: totalCap, whenFull: whenFull, maxAge: maxAge))
    }

    private static func image(_ size: Int, mime: String = "image/png") -> OutgoingImage {
        OutgoingImage(mime: mime, data: Data(repeating: 7, count: size))
    }

    @Test func aDraftsPhotosComeBackInOrderAfterARestart() async {
        let root = Self.scratch()
        defer { try? FileManager.default.removeItem(at: root) }
        let first = Self.image(10), second = Self.image(20, mime: "image/jpeg")
        let before = Self.store(root)
        before.keep(first, terminal: "pane-1")
        before.keep(second, terminal: "pane-1")
        before.flush()
        // A new store over the same directory: what a relaunch has.
        let restored = await Self.store(root).restored(terminal: "pane-1")
        #expect(restored.map(\.id) == [first.id, second.id])
        #expect(restored.map(\.mime) == ["image/png", "image/jpeg"])
        #expect(restored.map(\.data.count) == [10, 20])
        #expect(await Self.store(root).restored(terminal: "pane-2").isEmpty)
    }

    @Test func namesSortInTimeOrderWhereTheLow32BitsGoNegative() {
        // 1_763_084_076_008 ms is Nov 2025, and its low 32 bits are negative as an
        // Int32: a 32-bit format printed "-…" names that sorted newest first (ov-844).
        let image = Self.image(1)
        let millis = [1_763_084_076_008, 1_763_084_076_009, 1_763_084_076_010, 9_999_999_999_999]
        let names = millis.map { DraftImages.name(image, millis: $0) }
        #expect(names == names.sorted())
        #expect(names[0].hasPrefix("1763084076008_"))
    }

    @Test func aPhotoTakenOutOrSentDoesNotComeBack() async {
        let root = Self.scratch()
        defer { try? FileManager.default.removeItem(at: root) }
        let store = Self.store(root)
        let kept = Self.image(10), removed = Self.image(10)
        store.keep(kept, terminal: "p")
        store.keep(removed, terminal: "p")
        store.drop(removed.id, terminal: "p")
        #expect(await store.restored(terminal: "p").map(\.id) == [kept.id])
        store.dropAll(terminal: "p")
        #expect(await store.restored(terminal: "p").isEmpty)
        #expect(!FileManager.default.fileExists(atPath: root.appendingPathComponent("p").path), "the pane's folder stayed")
    }

    @Test func aPaneIsCappedAndTheOnesAlreadyKeptStay() async {
        let root = Self.scratch()
        defer { try? FileManager.default.removeItem(at: root) }
        let store = Self.store(root, paneCap: 100, totalCap: 10_000)
        let a = Self.image(60), b = Self.image(60), c = Self.image(40)
        store.keep(a, terminal: "p")
        store.keep(b, terminal: "p")  // 120 > 100: not kept
        store.keep(c, terminal: "p")  // 100: fits
        #expect(await store.restored(terminal: "p").map(\.id) == [a.id, c.id])
    }

    @Test func theWholeStoreIsCapped() async {
        let root = Self.scratch()
        defer { try? FileManager.default.removeItem(at: root) }
        let store = Self.store(root, paneCap: 100, totalCap: 150)
        store.keep(Self.image(100), terminal: "one")
        store.keep(Self.image(100), terminal: "two")  // 200 > 150
        #expect(await store.restored(terminal: "one").count == 1)
        #expect(await store.restored(terminal: "two").isEmpty)
    }

    @Test func aPhotoBiggerThanAPaneIsNeverWritten() async {
        let root = Self.scratch()
        defer { try? FileManager.default.removeItem(at: root) }
        let store = Self.store(root, paneCap: 50)
        store.keep(Self.image(51), terminal: "p")
        #expect(await store.restored(terminal: "p").isEmpty)
    }

    @Test func goneAndOldPanesArePrunedAtLaunch() async throws {
        let root = Self.scratch()
        defer { try? FileManager.default.removeItem(at: root) }
        let store = Self.store(root, maxAge: 3600)
        store.keep(Self.image(10), terminal: "live")
        store.keep(Self.image(10), terminal: "closed")
        store.keep(Self.image(10), terminal: "stale")
        store.flush()
        // `stale`'s file was last touched two hours ago.
        let folder = root.appendingPathComponent("stale")
        for file in try FileManager.default.contentsOfDirectory(at: folder, includingPropertiesForKeys: nil) {
            try FileManager.default.setAttributes([.modificationDate: Date().addingTimeInterval(-7200)], ofItemAtPath: file.path)
        }
        store.prune(keeping: ["live", "stale"])
        #expect(await store.restored(terminal: "live").count == 1)
        #expect(await store.restored(terminal: "closed").isEmpty, "a closed pane's photos stayed")
        #expect(await store.restored(terminal: "stale").isEmpty, "a photo past its age stayed")
    }

    @Test func aCacheMakesRoomByRemovingTheOldest() throws {
        let root = Self.scratch()
        defer { try? FileManager.default.removeItem(at: root) }
        let files = PaneImageFiles(root: root, paneCap: 130, totalCap: 150, whenFull: .evictOldest, maxAge: 3600)
        #expect(files.write(Data(count: 60), named: "a", pane: "p"))
        try FileManager.default.setAttributes(
            [.modificationDate: Date().addingTimeInterval(-60)], ofItemAtPath: root.appendingPathComponent("p/a").path)
        #expect(files.write(Data(count: 60), named: "b", pane: "p"))
        #expect(files.write(Data(count: 60), named: "c", pane: "p"))  // 180 > 130: a goes
        #expect(files.names(pane: "p").sorted() == ["b", "c"])
        #expect(files.write(Data(count: 60), named: "d", pane: "q"))  // 180 > 150 in all: the oldest goes
        #expect(files.totalBytes <= 150)
        #expect(!files.names(pane: "p").contains("a"))
        #expect(files.names(pane: "q") == ["d"], "the file just written was the one removed")
    }

    @Test func theFilesAreLeftOutOfBackups() throws {
        let root = Self.scratch()
        defer { try? FileManager.default.removeItem(at: root) }
        let files = PaneImageFiles(root: root, paneCap: 100, totalCap: 100, whenFull: .refuse, maxAge: 60)
        files.write(Data(count: 1), named: "a", pane: "p")
        #expect(try root.resourceValues(forKeys: [.isExcludedFromBackupKey]).isExcludedFromBackup == true)
    }

    @Test func twoPaneIdsThatSanitizeAlikeKeepApart() {
        let root = Self.scratch()
        defer { try? FileManager.default.removeItem(at: root) }
        let files = PaneImageFiles(root: root, paneCap: 100, totalCap: 100, whenFull: .refuse, maxAge: 60)
        files.write(Data(count: 1), named: "a", pane: "a/b")
        files.write(Data(count: 2), named: "a", pane: "a:b")
        #expect(files.bytes(pane: "a/b") == 1)
        #expect(files.bytes(pane: "a:b") == 2)
    }
}
