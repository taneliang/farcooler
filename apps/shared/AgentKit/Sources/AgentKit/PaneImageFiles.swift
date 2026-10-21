import Foundation

/// Image files kept on disk per pane, under a size cap and a maximum age
/// (ov-622): a draft's photos waiting to be sent (`DraftImages`) and the
/// thumbnails and originals of photos already sent (the phone's prompt-image
/// cache).
///
/// One directory per pane under `root`, one file per image. Everything here
/// is private to the app's own container, user-only, and left out of backups:
/// a photo is nothing the user asked to have copied off the device. A file
/// that can't be written or read is a miss, never an error to the person.
///
/// The caps are bytes. `paneCap` bounds one pane and `totalCap` the whole
/// root; a write that would pass either is `refuse`d (a draft's photo stays
/// in memory only, and the pane keeps what it already saved) or makes room by
/// removing the oldest files (a cache, which can always fetch again).
public struct PaneImageFiles: Sendable {
    public enum WhenFull: Sendable { case refuse, evictOldest }

    public let root: URL
    public let paneCap: Int
    public let totalCap: Int
    public let whenFull: WhenFull
    /// Files older than this are removed by `prune`.
    public let maxAge: TimeInterval

    public init(root: URL, paneCap: Int, totalCap: Int, whenFull: WhenFull, maxAge: TimeInterval) {
        self.root = root
        self.paneCap = paneCap
        self.totalCap = totalCap
        self.whenFull = whenFull
        self.maxAge = maxAge
    }

    /// A pane's directory name: its id as it is where that is plain, else the
    /// plain part and a hash of the whole, so two ids never share one.
    static func safe(_ name: String) -> String {
        let plain = name.unicodeScalars.map { CharacterSet.alphanumerics.contains($0) || $0 == "-" ? String($0) : "_" }.joined()
        guard plain != name || name.isEmpty else { return name }
        var hash: UInt64 = 0xcbf2_9ce4_8422_2325
        for byte in name.utf8 { hash = (hash ^ UInt64(byte)) &* 0x100_0000_01b3 }
        return "\(plain)-\(String(hash, radix: 16))"
    }

    /// `name` under the user's `directory`, in a folder of this app's bundle
    /// (a stable, a preview and a canary build on one Mac each have their own,
    /// and the system's own sandbox does the same on a phone). Without a
    /// bundle id, the folder is `Far Cooler`.
    public static func appFolder(_ directory: FileManager.SearchPathDirectory, _ name: String) -> URL {
        let base = FileManager.default.urls(for: directory, in: .userDomainMask).first ?? FileManager.default.temporaryDirectory
        return base.appendingPathComponent(Bundle.main.bundleIdentifier ?? "Far Cooler", isDirectory: true)
            .appendingPathComponent(name, isDirectory: true)
    }

    private func directory(_ pane: String) -> URL {
        root.appendingPathComponent(Self.safe(pane), isDirectory: true)
    }

    private struct Entry {
        let url: URL
        let pane: String
        let size: Int
        let modified: Date
    }

    private func entries(in pane: URL) -> [Entry] {
        let keys: [URLResourceKey] = [.fileSizeKey, .contentModificationDateKey, .isRegularFileKey]
        let files = (try? FileManager.default.contentsOfDirectory(at: pane, includingPropertiesForKeys: keys)) ?? []
        return files.compactMap { url in
            guard let values = try? url.resourceValues(forKeys: Set(keys)), values.isRegularFile == true else { return nil }
            return Entry(
                url: url, pane: pane.lastPathComponent, size: values.fileSize ?? 0,
                modified: values.contentModificationDate ?? .distantPast)
        }
    }

    private func allEntries() -> [Entry] {
        let panes = (try? FileManager.default.contentsOfDirectory(
            at: root, includingPropertiesForKeys: nil)) ?? []
        return panes.flatMap { entries(in: $0) }
    }

    /// Bytes held for `pane`.
    public func bytes(pane: String) -> Int { entries(in: directory(pane)).reduce(0) { $0 + $1.size } }

    /// Bytes held in all.
    public var totalBytes: Int { allEntries().reduce(0) { $0 + $1.size } }

    /// The names of `pane`'s files, oldest first.
    public func names(pane: String) -> [String] {
        entries(in: directory(pane)).sorted { ($0.modified, $0.url.lastPathComponent) < ($1.modified, $1.url.lastPathComponent) }
            .map { $0.url.lastPathComponent }
    }

    /// `data` as `name` in `pane`'s directory. False where it wasn't kept:
    /// bigger than a pane may hold, over a cap with `refuse`, or unwritable.
    @discardableResult
    public func write(_ data: Data, named name: String, pane: String) -> Bool {
        guard data.count <= paneCap else { return false }
        let manager = FileManager.default
        let folder = directory(pane)
        let file = folder.appendingPathComponent(name)
        // A file of this name is replaced, not added to.
        let replaced = (try? file.resourceValues(forKeys: [.fileSizeKey]).fileSize) ?? 0
        if whenFull == .refuse {
            guard bytes(pane: pane) - replaced + data.count <= paneCap, totalBytes - replaced + data.count <= totalCap else {
                return false
            }
        }
        do {
            try manager.createDirectory(at: folder, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
            try data.write(to: file, options: [.atomic, .completeFileProtection])
        } catch {
            return false
        }
        keepOutOfBackups()
        if whenFull == .evictOldest { makeRoom(keeping: file, pane: pane) }
        return true
    }

    /// The file named `name` in `pane`, or nil. Reading marks it recent, so a
    /// cache evicts what has not been looked at.
    public func url(named name: String, pane: String) -> URL? {
        let file = directory(pane).appendingPathComponent(name)
        guard FileManager.default.fileExists(atPath: file.path) else { return nil }
        try? FileManager.default.setAttributes([.modificationDate: Date()], ofItemAtPath: file.path)
        return file
    }

    /// Whether `name` is kept, without marking it recent.
    public func contains(named name: String, pane: String) -> Bool {
        FileManager.default.fileExists(atPath: directory(pane).appendingPathComponent(name).path)
    }

    public func read(named name: String, pane: String) -> Data? {
        url(named: name, pane: pane).flatMap { try? Data(contentsOf: $0) }
    }

    public func remove(named name: String, pane: String) {
        try? FileManager.default.removeItem(at: directory(pane).appendingPathComponent(name))
    }

    /// Everything kept for `pane`.
    public func removePane(_ pane: String) {
        try? FileManager.default.removeItem(at: directory(pane))
    }

    /// Files past `maxAge` go, panes not in `keeping` go (nil keeps every
    /// pane), and the oldest of what is left go until the root is under its
    /// cap.
    public func prune(keeping panes: Set<String>? = nil, now: Date = Date()) {
        let manager = FileManager.default
        if let panes {
            let keep = Set(panes.map(Self.safe))
            for folder in (try? manager.contentsOfDirectory(at: root, includingPropertiesForKeys: nil)) ?? []
            where !keep.contains(folder.lastPathComponent) {
                try? manager.removeItem(at: folder)
            }
        }
        var left: [Entry] = []
        for entry in allEntries() {
            if now.timeIntervalSince(entry.modified) > maxAge {
                try? manager.removeItem(at: entry.url)
            } else {
                left.append(entry)
            }
        }
        var total = left.reduce(0) { $0 + $1.size }
        for entry in left.sorted(by: { $0.modified < $1.modified }) where total > totalCap {
            try? manager.removeItem(at: entry.url)
            total -= entry.size
        }
        removeEmptyPanes()
    }

    private func makeRoom(keeping file: URL, pane: String) {
        let manager = FileManager.default
        let inPane = entries(in: directory(pane)).sorted { $0.modified < $1.modified }
        var held = inPane.reduce(0) { $0 + $1.size }
        for entry in inPane where held > paneCap && entry.url != file {
            try? manager.removeItem(at: entry.url)
            held -= entry.size
        }
        var total = totalBytes
        for entry in allEntries().sorted(by: { $0.modified < $1.modified }) where total > totalCap && entry.url != file {
            try? manager.removeItem(at: entry.url)
            total -= entry.size
        }
    }

    private func removeEmptyPanes() {
        let manager = FileManager.default
        for folder in (try? manager.contentsOfDirectory(at: root, includingPropertiesForKeys: nil)) ?? []
        where (try? manager.contentsOfDirectory(atPath: folder.path))?.isEmpty == true {
            try? manager.removeItem(at: folder)
        }
    }

    private func keepOutOfBackups() {
        var url = root
        var values = URLResourceValues()
        values.isExcludedFromBackup = true
        try? url.setResourceValues(values)
    }
}
