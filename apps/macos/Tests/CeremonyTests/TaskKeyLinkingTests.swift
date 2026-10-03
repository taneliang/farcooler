import AgentKit
import Foundation
import Testing

@testable import Far_Cooler

/// Task keys in the window's text as links (ov-196): the Mac's index is the
/// selection's runner's boards, and a link lands where the palette opens a
/// task.
@MainActor
struct TaskKeyLinkingTests {
    /// A store on `host` whose board holds `tasks`, read.
    private func store(host: String, workspace: String, prefix: String, tasks: [(String, String)]) async
        -> TaskBoardStore
    {
        let client = DaemonClient(target: host, notifications: NotificationCenter())
        client.commandRunnerForTesting = { args in
            guard args.starts(with: ["task", "list"]) else { return (Data(), nil) }
            let rows = tasks.map { id, key in #"{"id":"\#(id)","key":"\#(key)","title":"T","status":"todo"}"# }
            return (Data(#"{"tasks":[\#(rows.joined(separator: ","))]}"#.utf8), nil)
        }
        let made = TaskBoardStore(
            client: client,
            workspace: WorkspaceSummary(id: workspace, name: workspace, taskPrefix: prefix, isMain: true, ordinal: 0),
            readStore: DefaultsBoardReads(UserDefaults(suiteName: "ov196-\(UUID().uuidString)")!))
        await made.reload()
        return made
    }

    @Test("A runner's text links its own boards' keys, and a link opens as the palette opens a task")
    func linksAndOpens() async throws {
        let mine = await store(host: "studio", workspace: "ws-1", prefix: "ov", tasks: [("t190", "ov-190")])
        let theirs = await store(host: "laptop", workspace: "ws-9", prefix: "ov", tasks: [("t7", "ov-7")])
        let workspaces = [mine.workspace]
        let index = TaskKeyIndex.mac(host: "studio", workspaces: workspaces, stores: [mine, theirs])
        #expect(TaskKeyLinks.matches(in: "ov-190 and ov-7", index: index).map(\.key) == ["ov-190"])

        final class Opened: @unchecked Sendable { var targets: [TaskKeyTarget] = [] }
        let opened = Opened()
        let linker = TaskKeyLinker(index: index) { opened.targets.append($0) }
        Markdown.openGuard(linker)(try #require(TaskKeyLinks.url(runner: "studio", key: "ov-190")))
        let target = try #require(opened.targets.first)
        #expect(target == TaskKeyTarget(runner: "studio", workspace: "ws-1", task: "t190", key: "ov-190"))

        // What `ContentView.taskKeyLinker` hands `openTask`, which lands as
        // the palette's open does.
        let landed = WorkspaceNavigation.openingTask(
            target.task, host: target.runner, workspace: target.workspace, from: WorkspaceNavigation.BoardState(opened: false))
        #expect(landed.selection == ContentView.Selection.workspace(host: "studio", workspace: "ws-1", focus: .task("t190")))
    }
}
