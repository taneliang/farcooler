import Foundation

/// A scratch directory for one test, removed when the test process ends (ov-541).
///
/// A test used to make `farcooler-route-<UUID>` in the temporary directory and
/// never remove it, and a run of the Mac tests left one more of each behind:
/// 685,000 entries in `$TMPDIR` by 14 Oct. Every directory made here is
/// remembered and deleted by an `atexit` handler, so a test that fails halfway
/// leaves nothing either. `scripts/test-leak-check.py` fails a run that leaves a
/// `farcooler-*` entry behind.
enum ScratchDirectory {
    private static let lock = NSLock()
    nonisolated(unsafe) private static var made: [URL] = []
    nonisolated(unsafe) private static var registered = false

    /// A new, empty directory named `<name>-<UUID>`.
    static func make(_ name: String) throws -> URL {
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("\(name)-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        lock.lock()
        defer { lock.unlock() }
        if !registered {
            registered = true
            atexit { ScratchDirectory.sweep() }
        }
        made.append(directory)
        return directory
    }

    fileprivate static func sweep() {
        lock.lock()
        let all = made
        lock.unlock()
        for directory in all { try? FileManager.default.removeItem(at: directory) }
    }
}
