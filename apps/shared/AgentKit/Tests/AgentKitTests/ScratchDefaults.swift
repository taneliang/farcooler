import Foundation

/// A defaults suite for one test, kept in a scratch folder instead of
/// `~/Library/Preferences` (ov-541).
///
/// `UserDefaults(suiteName: "name")` writes `~/Library/Preferences/name.plist`,
/// and a test that names its suite with a UUID leaves one more file per run:
/// 99,566 of them by 14 Oct. Even `removePersistentDomain` leaves the file
/// behind, empty, and the preferences daemon writes it again after the call, so
/// a test cannot reliably remove it. A suite named by a full path is kept at that
/// path instead, so this makes one in a folder of its own under the temporary
/// directory, which an `atexit` handler deletes. A test that fails halfway, or
/// never thought about cleanup, leaves nothing either.
///
/// Use this and never `UserDefaults(suiteName:)` in a test: `scripts/test-leak-check.py
/// --lint` fails the build on the latter, and the same script fails a test run
/// that leaves a plist or a temp entry behind.
enum ScratchDefaults {
    private static let lock = NSLock()
    nonisolated(unsafe) private static var root: URL?

    /// The suite called `name`, stored in this process's scratch folder.
    static func suite(_ name: String) -> UserDefaults? {
        lock.lock()
        defer { lock.unlock() }
        if root == nil {
            let folder = FileManager.default.temporaryDirectory
                .appendingPathComponent("fc-defaults-\(UUID().uuidString)", isDirectory: true)
            guard (try? FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)) != nil
            else { return nil }
            root = folder
            atexit { ScratchDefaults.sweep() }
        }
        return UserDefaults(suiteName: root!.appendingPathComponent(name).path)
    }

    fileprivate static func sweep() {
        lock.lock()
        let folder = root
        lock.unlock()
        if let folder { try? FileManager.default.removeItem(at: folder) }
    }
}
