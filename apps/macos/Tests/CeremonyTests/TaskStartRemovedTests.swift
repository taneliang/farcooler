import Foundation
import Testing

@testable import Far_Cooler

/// Nothing on the Mac starts a task from typed words (ov-951). A person tells
/// the orchestrator, and the orchestrator starts the work. New Worktree and
/// Resume Branch stay: they make a worktree you name yourself, not a task.
struct TaskStartRemovedTests {
    /// What a typed-description start is made of: the quick-create panel and
    /// its request, the window's start, and the client call that makes a
    /// worktree and an agent from a description.
    static let startMarkers = [
        "struct QuickCreate", "QuickCreate(", "showQuickCreate", "struct TaskRequest", "class TaskSubmission",
        "func startTask(", ".startTask(",
    ]

    static func startsFound(in texts: [(String, String)]) -> [String] {
        texts.flatMap { name, text in
            startMarkers.filter { text.contains($0) }.map { "\($0) in \(name)" }
        }
    }

    @Test("No Mac source offers a way to start a task from a description")
    func noMacSourceStartsATask() throws {
        let texts = try TaskManagementRemovedTests.macSources()
        #expect(texts.count > 100, "found \(texts.count) sources, so this proves nothing")
        // The manual paths are still there, so the scan is looking at the right tree.
        #expect(texts.contains { $0.1.contains("struct NewWorktreeSheet") }, "New Worktree moved")
        #expect(texts.contains { $0.1.contains("struct ResumeBranch") }, "Resume Branch moved")
        let found = Self.startsFound(in: texts)
        #expect(found.isEmpty, "a task start the orchestrator owns: \(found)")
    }

    @Test("A planted start is found")
    func aPlantedStartIsFound() {
        let texts = [
            ("A.swift", "if showQuickCreate { QuickCreate(projects: p) }"),
            ("B.swift", "let r = await client.startTask(project: p)"),
            ("C.swift", "Button(\"New Worktree…\") { sheet = true }"),
        ]
        #expect(
            Self.startsFound(in: texts)
                == ["QuickCreate( in A.swift", "showQuickCreate in A.swift", ".startTask( in B.swift"])
    }
}
