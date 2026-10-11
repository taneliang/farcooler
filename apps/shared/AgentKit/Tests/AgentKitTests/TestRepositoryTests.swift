import Foundation
import Testing

/// `TestRepository` finds this checkout and never the one around it.
@Suite struct TestRepositoryTests {
    @Test("The root is this checkout's: it holds this file, and its .git is the first one up")
    func theRootIsThisCheckouts() throws {
        let root = try #require(TestRepository.root(), "no .git above \(#filePath)")
        let here = URL(fileURLWithPath: #filePath).standardizedFileURL.path
        #expect(here.hasPrefix(root.standardizedFileURL.path + "/"))
        #expect(FileManager.default.fileExists(atPath: root.appendingPathComponent(".git").path))
        #expect(TestRepository.source("apps/shared/AgentKit/Package.swift") != nil)
    }

    /// A worktree nested in the main checkout stops at its own root, so a
    /// file deleted in the lane is missing here even while the main checkout
    /// still has it.
    @Test("A nested checkout stops at its own .git")
    func aNestedCheckoutStopsAtItsOwnGit() throws {
        let outer = FileManager.default.temporaryDirectory
            .appendingPathComponent("test-repository-\(UUID().uuidString)")
        defer { try? FileManager.default.removeItem(at: outer) }
        let inner = outer.appendingPathComponent(".claude/worktrees/lane")
        let tests = inner.appendingPathComponent("apps/shared/Tests")
        try FileManager.default.createDirectory(at: tests, withIntermediateDirectories: true)
        try FileManager.default.createDirectory(
            at: outer.appendingPathComponent(".git"), withIntermediateDirectories: true)
        try "gitdir: elsewhere\n".write(to: inner.appendingPathComponent(".git"), atomically: true, encoding: .utf8)
        try FileManager.default.createDirectory(
            at: outer.appendingPathComponent("apps/ios"), withIntermediateDirectories: true)
        try "stale".write(
            to: outer.appendingPathComponent("apps/ios/Gone.swift"), atomically: true, encoding: .utf8)

        let file = tests.appendingPathComponent("SomeTests.swift").path
        #expect(TestRepository.root(from: file)?.standardizedFileURL == inner.standardizedFileURL)
        #expect(TestRepository.source("apps/ios/Gone.swift", from: file) == nil, "borrowed the outer checkout's file")
        #expect(TestRepository.source("apps/ios/Gone.swift", from: outer.appendingPathComponent("x.swift").path) == "stale")
    }
}
