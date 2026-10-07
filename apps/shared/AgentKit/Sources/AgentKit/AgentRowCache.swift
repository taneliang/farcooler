import Foundation

/// The last rows each pane showed, in memory and on disk, so a pane coming
/// back draws its first frame at once instead of after a round trip (ov-371).
///
/// Memory answers synchronously, for a pane re-opened in this launch; disk
/// answers off the main thread, for the first open after a relaunch. Both
/// hold `AgentRowSnapshot`s: the newest rows and the cursor to follow from,
/// so a returning pane asks the runner only what changed since.
///
/// What is on disk is the session's own words, as the runner already serves
/// them to this user; it lives in this app's caches directory, user-only, and
/// a cache that can't be read is a cache miss.
public final class AgentRowCache: @unchecked Sendable {
    /// Where the Mac and the phone keep theirs.
    public static let shared = AgentRowCache(
        directory: FileManager.default.urls(for: .cachesDirectory, in: .userDomainMask).first?
            .appendingPathComponent("agent-rows", isDirectory: true))

    /// How many rows a snapshot keeps: a screenful and then some.
    public static let rowsKept = 200
    /// How many panes memory holds before the least recent goes.
    public static let panesKept = 24

    private let directory: URL?
    private let lock = NSLock()
    private var memory: [String: AgentRowSnapshot] = [:]
    private var recency: [String] = []
    private let writes = DispatchQueue(label: "agent-rows.cache", qos: .utility)

    /// `directory` nil keeps memory only.
    public init(directory: URL?) {
        self.directory = directory
    }

    /// What this launch last held for `key`.
    public func inMemory(_ key: String) -> AgentRowSnapshot? {
        lock.withLock { memory[key] }
    }

    /// What disk held for `key`. Reads and decodes a file: call it off the
    /// main thread.
    public func onDisk(_ key: String) -> AgentRowSnapshot? {
        guard let url = file(for: key), let data = try? Data(contentsOf: url) else { return nil }
        return try? JSONDecoder().decode(AgentRowSnapshot.self, from: data)
    }

    /// Keep `snapshot` for `key`: in memory now, on disk soon after.
    public func keep(_ snapshot: AgentRowSnapshot, for key: String) {
        lock.withLock {
            memory[key] = snapshot
            recency.removeAll { $0 == key }
            recency.append(key)
            while recency.count > Self.panesKept { memory[recency.removeFirst()] = nil }
        }
        guard let url = file(for: key) else { return }
        writes.async {
            guard let data = try? JSONEncoder().encode(snapshot) else { return }
            try? FileManager.default.createDirectory(
                at: url.deletingLastPathComponent(), withIntermediateDirectories: true,
                attributes: [.posixPermissions: 0o700])
            try? data.write(to: url, options: [.atomic])
        }
    }

    /// Wait for the writes so far. For tests.
    public func flush() {
        writes.sync {}
    }

    /// Forget memory, so the next read goes to disk. For tests.
    public func dropMemory() {
        lock.withLock {
            memory = [:]
            recency = []
        }
    }

    private func file(for key: String) -> URL? {
        let safe = key.unicodeScalars.map { CharacterSet.alphanumerics.contains($0) || $0 == "-" ? String($0) : "_" }.joined()
        return directory?.appendingPathComponent("\(safe).json")
    }
}
