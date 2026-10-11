import Foundation

/// The checkout this test file is in, for the tests that read the tree's own
/// sources.
///
/// **Stops at the first `.git`, file or directory.** A lane's worktree sits
/// inside the main checkout (`.claude/worktrees/<lane>`), and a walk that went
/// on past its own root found the MAIN checkout's copy of a file the lane had
/// deleted, so a guard passed in every lane and failed only in CI (Train 77,
/// `AgentView.swift`). `.git` is a directory at a main checkout's root and a
/// file at a worktree's, so the first one up is this tree's root, whichever
/// this is.
enum TestRepository {
    static func root(from file: String = #filePath) -> URL? {
        var directory = URL(fileURLWithPath: file).deletingLastPathComponent()
        while directory.path != "/" {
            if FileManager.default.fileExists(atPath: directory.appendingPathComponent(".git").path) {
                return directory
            }
            directory = directory.deletingLastPathComponent()
        }
        return nil
    }

    /// The text of `path`, relative to this checkout's root, or nil when this
    /// checkout has no such file. Never another checkout's.
    static func source(_ path: String, from file: String = #filePath) -> String? {
        guard let root = root(from: file) else { return nil }
        return try? String(contentsOf: root.appendingPathComponent(path), encoding: .utf8)
    }
}
