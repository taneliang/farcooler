import Foundation
import SwiftUI
import Testing

@testable import AgentKit

/// `test/fixtures/task-key-links.json`, which Android's `TaskKeyLinksTest`
/// reads too: text, and the task keys in it that become links.
private struct TaskKeyLinksFixture: Decodable {
    struct Link: Decodable, Equatable {
        var start: Int
        var key: String
    }
    struct Case: Decodable {
        var text: String
        var links: [Link]
    }
    var prefixes: [String]
    var known: [String]
    var cases: [Case]

    static func load() throws -> TaskKeyLinksFixture {
        var root = URL(fileURLWithPath: #filePath)
        // …/apps/shared/AgentKit/Tests/AgentKitTests/<this file>
        for _ in 0..<6 { root.deleteLastPathComponent() }
        let data = try Data(contentsOf: root.appendingPathComponent("test/fixtures/task-key-links.json"))
        return try JSONDecoder().decode(TaskKeyLinksFixture.self, from: data)
    }
}

/// Task keys in text as links (ov-196).
@MainActor
struct TaskKeyLinksTests {
    /// The board a runner "r1" has read: ov-190 and ov-7 on Main, lo-3 on
    /// another workspace.
    static func index() -> TaskKeyIndex {
        func board(_ rows: [(String, String)]) -> TaskBoardModel {
            TaskBoardModel(columns: [
                TaskBoardColumn(
                    status: .todo,
                    rows: rows.map { TaskRow(id: $0.0, key: $0.1, title: $0.1, status: .todo, statusSince: .now) })
            ])
        }
        return TaskKeyIndex(
            runner: "r1",
            workspaces: [
                WorkspaceSummary(id: "w-main", name: "Main", taskPrefix: "ov", isMain: true, ordinal: 0),
                WorkspaceSummary(id: "w-lo", name: "Lo", taskPrefix: "lo", isMain: false, ordinal: 1),
            ],
            boards: ["w-main": board([("t190", "ov-190"), ("t7", "ov-7")]), "w-lo": board([("t3", "lo-3")])])
    }

    @Test("Every case in the shared fixture finds exactly its keys")
    func theFixture() throws {
        let fixture = try TaskKeyLinksFixture.load()
        #expect(fixture.cases.count >= 20)
        #expect(fixture.cases.contains { $0.links.isEmpty } && fixture.cases.contains { !$0.links.isEmpty })
        for item in fixture.cases {
            let found = TaskKeyLinks.matches(
                in: item.text, prefixes: Set(fixture.prefixes), known: Set(fixture.known))
            #expect(
                found.map { TaskKeyLinksFixture.Link(start: $0.start, key: $0.key) } == item.links,
                "\(item.text.debugDescription)")
        }
    }

    @Test("A runner's index is its workspaces' prefixes and its boards' keys")
    func theIndex() {
        let index = Self.index()
        #expect(index.prefixes == ["ov", "lo"])
        #expect(index.targets["ov-190"] == TaskKeyTarget(runner: "r1", workspace: "w-main", task: "t190", key: "ov-190"))
        #expect(index.targets["lo-3"]?.workspace == "w-lo")
        #expect(TaskKeyLinks.matches(in: "ov-190, ov-191 and lo-3", index: index).map(\.key) == ["ov-190", "lo-3"])
        #expect(TaskKeyLinks.matches(in: "ov-190", index: .empty).isEmpty)
    }

    @Test("A key in text links to its task; in a link or code it doesn't")
    func linkedText() throws {
        let index = Self.index()
        func links(_ markdown: String) -> [String: URL] {
            let text = TaskKeyLinks.linked(Markdown.inline(markdown), index: index)
            var out: [String: URL] = [:]
            for run in text.runs { if let url = run.link { out[String(text[run.range].characters)] = url } }
            return out
        }
        let task = try #require(TaskKeyLinks.url(runner: "r1", key: "ov-190"))
        #expect(task.absoluteString == "farcooler://task/r1/ov-190")
        #expect(links("Done in **ov-190**, not utf-8 or ov-191.") == ["ov-190": task])
        #expect(links("`ov-190` is quoted").isEmpty, "a key in code is quoted, not linked")
        let spec = try #require(URL(string: "https://x.y/ov-190"))
        #expect(links("[see ov-190](https://x.y/ov-190)") == ["see ov-190": spec], "the link's own words keep it")
        #expect(String(TaskKeyLinks.linked(Markdown.inline("a ov-7 b"), index: index).characters) == "a ov-7 b")
        let emoji = TaskKeyLinks.linked(AttributedString("👩‍👩‍👧 ov-7 é"), index: index)
        #expect(emoji.runs.compactMap { $0.link == nil ? nil : String(emoji[$0.range].characters) } == ["ov-7"])
        #expect(TaskKeyLinks.linked(AttributedString("ov-190"), index: .empty).runs.allSatisfy { $0.link == nil })
    }

    @Test("A task link names its runner and key, and nothing else parses as one")
    func theURL() throws {
        let url = try #require(TaskKeyLinks.url(runner: "e@host:22/x", key: "ov-190"))
        #expect(TaskKeyLinks.parse(url)?.runner == "e@host:22/x")
        #expect(TaskKeyLinks.parse(url)?.key == "ov-190")
        for other in [
            "farcooler://task/r1", "farcooler://task/r1/ov-1/x", "farcooler://terminal/r1/ov-1",
            "farcooler://x", "https://task/r1/ov-1", "farcooler-canary://task/r1/ov-1",
        ] {
            #expect(TaskKeyLinks.parse(URL(string: other)!) == nil, "\(other)")
        }
    }

    /// The guard lets the web, mail and a task link through, and nothing
    /// else new: the app's other `farcooler://` URLs are still refused.
    @Test("The open guard adds the task link and only it")
    func theGuard() {
        for allowed in ["https://a.b", "HTTP://a.b", "mailto:o@a.b", "farcooler://task/r1/ov-190"] {
            #expect(Markdown.opens(URL(string: allowed)!), "\(allowed) refused")
        }
        for refused in [
            "farcooler://x", "farcooler://terminal/abc", "farcooler://task/r1", "farcooler://auth?code=1",
            "file:///tmp/x.command", "javascript:alert(1)", "tel:123",
        ] {
            #expect(!Markdown.opens(URL(string: refused)!), "\(refused) let through")
        }
    }

    /// Following a task link opens its task through the linker, on its own
    /// runner only, and a task link is never handed on, opened or not.
    @Test("A task link opens its task on its runner, and nothing else does")
    func following() {
        final class Opened: @unchecked Sendable { var targets: [TaskKeyTarget] = [] }
        let box = Opened()
        let linker = TaskKeyLinker(index: Self.index(), open: { box.targets.append($0) })
        var opened: [TaskKeyTarget] { box.targets }
        #expect(linker.follow(URL(string: "farcooler://task/r1/ov-190")!))
        #expect(opened == [TaskKeyTarget(runner: "r1", workspace: "w-main", task: "t190", key: "ov-190")])
        #expect(linker.follow(URL(string: "farcooler://task/r2/ov-190")!), "another runner's: taken, not opened")
        #expect(linker.follow(URL(string: "farcooler://task/r1/ov-999")!), "unknown: taken, not opened")
        #expect(!linker.follow(URL(string: "https://a.b")!))
        #expect(opened.count == 1)

        // Through the guard a view draws under: the task opens, in the app.
        Markdown.openGuard(linker)(URL(string: "farcooler://task/r1/lo-3")!)
        #expect(opened.last?.key == "lo-3")

        // A phone pushes it as the task it is, on its runner's workspace.
        #expect(opened.last?.phoneRoute == .task(PhoneWorkspace(runner: "r1", workspace: "w-lo"), task: "t3"))
    }
}

extension TaskKeyLinksTests {
    /// **The phone's link opens its task's route** through the navigator's
    /// push, with the runner and the workspace each where they go: what
    /// `Connection.taskKeyLinker` hands the screens.
    @Test("The phone's linker pushes the linked task's route")
    func thePhoneWiring() throws {
        final class Box: @unchecked Sendable { var routes: [PhoneRoute] = [] }
        let box = Box()
        let main = WorkspaceSummary(id: "w-main", name: "Main", taskPrefix: "ov", isMain: true, ordinal: 0)
        let boards = [
            "w-main": TaskBoardModel(columns: [
                TaskBoardColumn(
                    status: .todo, rows: [TaskRow(id: "t190", key: "ov-190", title: "T", status: .todo, statusSince: .now)])
            ])
        ]
        let linker = TaskKeyLinker.phone(runner: "R1", workspaces: [main], boards: boards) { box.routes.append($0) }
        #expect(linker.index.runner == "R1")
        #expect(linker.follow(try #require(TaskKeyLinks.url(runner: "R1", key: "ov-190"))))
        #expect(box.routes == [.task(PhoneWorkspace(runner: "R1", workspace: "w-main"), task: "t190")])

        // No runner yet, or nothing to push with: nothing is linked.
        #expect(TaskKeyLinker.phone(runner: nil, workspaces: [main], boards: boards) { _ in } == .none)
        #expect(TaskKeyLinker.phone(runner: "R1", workspaces: [main], boards: boards, open: nil) == .none)
    }

    /// **A row that speaks as one element still offers its links**, one
    /// action per task, in order, the same one twice only once, and none
    /// for a key the text quotes or a link to the web.
    @Test("A linked line's tasks, once each, for its accessibility actions")
    func theActions() throws {
        final class Box: @unchecked Sendable { var targets: [TaskKeyTarget] = [] }
        let box = Box()
        let linker = TaskKeyLinker(index: Self.index()) { box.targets.append($0) }
        let text = try AttributedString(
            markdown: "lo-3 before ov-190, `ov-7`, [ov-7](https://x.dev) and lo-3 again")
        let targets = linker.targets(in: linker.linked(text))
        #expect(targets.map(\.key) == ["lo-3", "ov-190"])
        #expect(linker.targets(in: text).isEmpty, "unlinked text has no actions")
    }

    /// **A link's target is a `Destination`** (ov-182) in this device's ids,
    /// so the apps can open links through it with one line.
    @Test("A task link's target as a destination")
    func theDestination() {
        let target = TaskKeyTarget(runner: "R1", workspace: "w-main", task: "t190", key: "ov-190")
        #expect(
            target.destination
                == Destination(
                    runner: .init(host: "R1"), place: .task(workspace: "w-main", task: .init(id: "t190", key: "ov-190"))))
    }
}

extension TaskKeyLinksTests {
    /// **The Mac's own runner links too** (integ-3): its host target is
    /// `""`, and a link to it once came out nil, so no key on this Mac's
    /// boards became a link.
    @Test("The Mac's own runner, host \"\", links its keys and opens them")
    func theLocalRunner() throws {
        let url = try #require(TaskKeyLinks.url(runner: "", key: "ov-190"))
        let parsed = try #require(TaskKeyLinks.parse(url))
        #expect(parsed.runner == "" && parsed.key == "ov-190")

        var index = Self.index()
        index.runner = ""
        index.targets = index.targets.mapValues {
            TaskKeyTarget(runner: "", workspace: $0.workspace, task: $0.task, key: $0.key)
        }
        final class Box: @unchecked Sendable { var targets: [TaskKeyTarget] = [] }
        let box = Box()
        let linker = TaskKeyLinker(index: index) { box.targets.append($0) }
        let linked = linker.linked(AttributedString("see ov-190"))
        let link = try #require(linked.runs.compactMap(\.link).first, "no link on the local runner")
        #expect(linker.follow(link))
        #expect(box.targets.map(\.task) == ["t190"])
    }
}
