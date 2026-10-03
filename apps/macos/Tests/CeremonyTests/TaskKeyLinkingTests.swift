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

        // The window's own builder, as `ContentView.taskKeyLinker` calls it,
        // with `openTask` recorded: which id goes where is the wiring.
        final class Opened: @unchecked Sendable { var calls: [[String]] = [] }
        let opened = Opened()
        let linker = TaskKeyLinker.mac(host: "studio", workspaces: workspaces, stores: [mine, theirs]) {
            opened.calls.append([$0, $1, $2])
        }
        #expect(linker.index == index)
        Markdown.openGuard(linker)(try #require(TaskKeyLinks.url(runner: "studio", key: "ov-190")))
        let call = try #require(opened.calls.first)
        #expect(call == ["t190", "studio", "ws-1"], "openTask(task, host:, workspace:) got \(call)")

        // What `openTask` does with those, which lands as the palette's
        // open does.
        let landed = WorkspaceNavigation.openingTask(
            call[0], host: call[1], workspace: call[2], from: WorkspaceNavigation.BoardState(opened: false))
        #expect(landed.selection == ContentView.Selection.workspace(host: "studio", workspace: "ws-1", focus: .task("t190")))
    }

    /// **This Mac's own runner, host `""`** (integ-3): its keys once made
    /// no link at all, as `TaskKeyLinks.url` refused an empty runner.
    @Test("This Mac's own runner links its keys, and a link opens its task")
    func theLocalRunner() async throws {
        let local = await store(host: "", workspace: "ws-1", prefix: "ov", tasks: [("t190", "ov-190")])
        final class Opened: @unchecked Sendable { var calls: [[String]] = [] }
        let opened = Opened()
        let linker = TaskKeyLinker.mac(host: "", workspaces: [local.workspace], stores: [local]) {
            opened.calls.append([$0, $1, $2])
        }
        let linked = linker.linked(AttributedString("see ov-190"))
        let link = try #require(linked.runs.compactMap(\.link).first, "no link on this Mac's runner")
        Markdown.openGuard(linker)(link)
        #expect(opened.calls == [["t190", "", "ws-1"]])
    }
}
