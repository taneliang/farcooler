import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// The Changes pane (ov-224): the file column and its filter, the strip of
/// controls, the diff with its hunk rules, the two warning strips. A rendering,
/// written by `VisualSpecimen`.
@MainActor
struct ChangesSpecimenTests {
    @Test("Write the Changes pane sheet")
    func writeSheet() throws {
        try VisualSpecimen.shoot("changes", size: CGSize(width: 980, height: 520), Self.pane())
    }

    private static func store() -> ChangesStore {
        let worktree = Worktree(
            id: "co", short: "co", task: "overnight", branch: "main", repository: "overnight", host: "",
            path: "/tmp/overnight", state: "active", terminals: [])
        let store = ChangesStore(client: DaemonClient(target: ""), worktree: worktree)
        func file(_ path: String, _ status: ChangedFileStatus, _ plus: Int, _ minus: Int) -> ChangedFile {
            ChangedFile(path: path, status: status, oldPath: nil, insertions: plus, deletions: minus, binary: false)
        }
        store.changeSet = ChangeSet(
            branch: "mac-vis", baseRef: "main", baseSource: "guessed", baseCommit: "", headCommit: "",
            insertions: 19, deletions: 6, commits: [],
            files: [
                file("apps/macos/Sources/FarCooler/ChangesPane.swift", .modified, 12, 4),
                file("apps/macos/Sources/FarCooler/DiffView.swift", .modified, 5, 2),
                file("scripts/visual-tokens-baseline.json", .added, 2, 0),
            ], workingTree: nil)
        store.selectedFile = "apps/macos/Sources/FarCooler/ChangesPane.swift"
        let lines: [DiffComputation.Line] = [
            .init(id: 0, kind: .context, oldNumber: 88, newNumber: 88, text: "                    HStack(spacing: 0) {"),
            .init(id: 1, kind: .removed, oldNumber: 89, newNumber: nil, text: "                        Divider()"),
            .init(id: 2, kind: .added, oldNumber: nil, newNumber: 89, text: "                        fileColumn.separator(.split, edge: .trailing)"),
            .init(id: 3, kind: .context, oldNumber: 90, newNumber: 90, text: "                        VStack(spacing: 0) {"),
            .init(id: 4, kind: .context, oldNumber: 91, newNumber: 91, text: "                            diffNavigator(compact: false)"),
            .init(id: 5, kind: .context, oldNumber: 92, newNumber: 92, text: "                            diffBody"),
        ]
        for f in store.changeSet.files { store.fileDiffs[f.path] = FileDiff(lines: lines) }
        store.error = "The command that reads it didn’t finish."
        return store
    }

    private static func pane() -> some View {
        ChangesPane(changes: store(), isFocused: true, agents: [])
    }
}
