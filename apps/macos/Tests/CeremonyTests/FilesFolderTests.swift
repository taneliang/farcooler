import AgentKit
import Foundation
import Testing

@testable import Far_Cooler

/// A runner's extra read-only folders in Files (ov-232): they show only for a
/// runner that offers them, are read by name, and a refusal reads plainly.
@MainActor
struct FilesFolderTests {
    // MARK: - Whether they show

    @Test("A runner with the capability shows the folders it lists")
    func theFoldersShowWithTheCapability() {
        let status: [String: Any] = ["readOnlyFolders": [["name": "logs", "path": ""], ["name": "etc", "path": ""]]]
        #expect(ReadOnlyFolders.names(in: status, offered: true) == ["logs", "etc"])
    }

    @Test("A runner without the capability shows none, whatever it lists")
    func noFoldersWithoutTheCapability() {
        let status: [String: Any] = ["readOnlyFolders": [["name": "logs", "path": ""]]]
        #expect(ReadOnlyFolders.names(in: status, offered: false).isEmpty)
        #expect(ReadOnlyFolders.names(in: [:], offered: true).isEmpty)
        #expect(ReadOnlyFolders.names(in: ["readOnlyFolders": NSNull()], offered: true).isEmpty)
    }

    @Test("The client keeps the names only while its runner says it can show them")
    func theClientGatesOnTheCapability() async {
        func names(_ capabilities: [String]) async -> [String] {
            let recorder = WorktreeCallsTests.Recorder()
            recorder.capabilities = capabilities
            recorder.folders = ["logs"]
            let client = DaemonClient(target: "", notifications: NotificationCenter())
            client.commandRunnerForTesting = { recorder.answer($0) }
            await client.refreshNeedsYou()
            return client.readOnlyFolders
        }
        #expect(await names(["worktrees", "terminals", "read_only_folders"]) == ["logs"])
        #expect(await names(["worktrees", "terminals"]).isEmpty)
    }

    @Test("The switcher's menu has a Folders submenu only when a runner shares some")
    func theSwitcherOffersFolders() {
        func entries(_ folders: [ReadOnlyFolders.Group]) -> [SwitcherEntry] {
            WorkspaceSwitcherMenu.entries(
                groups: [], current: nil, waiting: { _ in 0 }, showsHosts: false, needsYou: 0,
                offersNewWorkspace: false, status: "", statusTrouble: false, troubled: [], folders: folders)
        }
        let commands = entries([.init(host: "", names: ["logs"])]).flatMap(\.commands)
        #expect(commands.contains(.openFolder(host: "", name: "logs")))
        #expect(!entries([]).flatMap(\.commands).contains { if case .openFolder = $0 { true } else { false } })
        // With several runners each one's folders sit under its name.
        let two = entries([.init(host: "", names: ["logs"]), .init(host: "box", names: ["etc"])])
        guard case .submenu(let title, _, let lines)? = two.first(where: { if case .submenu = $0 { true } else { false } })
        else { Issue.record("no submenu"); return }
        #expect(title == "Folders")
        #expect(lines.contains(.header("This Mac")) && lines.contains(.header("box")))
    }

    // MARK: - Reading

    private func model(_ files: [String: Result<FileRead, FileReadFailure>] = [:], root: Result<FileListing, FileReadFailure>) -> FilesModel {
        FilesModel(
            place: FilesModel.Place(folder: "logs", host: ""),
            source: FilesModel.Source(
                list: { _ in root },
                read: { files[$0] ?? .failure(.missingInFolder) },
                search: { _ in ["never"] }))
    }

    @Test("A folder is browsed and read like a worktree's files, without a filter or an editor")
    func aFolderBrowsesLikeAWorktree() async {
        let entry = FileEntry(name: "syslog", kind: .file, size: 5, linkTarget: "")
        let read = FileRead(path: "syslog", state: .text, size: 5, text: "a\nb\n", linkTarget: "")
        let model = model(["syslog": .success(read)], root: .success(FileListing(path: "", entries: [entry], truncated: false)))
        await model.loadIfNeeded()
        #expect(model.rows.map(\.name) == ["syslog"])
        await model.open("syslog", line: nil)
        #expect(model.lines == ["a", "b"])
        #expect(model.worktree == nil)
        #expect(!model.canFilter)
        #expect(model.place.title == "logs")
    }

    @Test("Reading a folder's file goes through the folder, never a worktree")
    func readsGoThroughTheFolderField() async {
        let recorder = WorktreeCallsTests.Recorder()
        let client = DaemonClient(target: "", notifications: NotificationCenter())
        client.commandRunnerForTesting = { recorder.answer($0) }
        let model = FilesModel(folder: "logs", client: client)
        await model.loadIfNeeded()
        await model.open("nginx/a.log", line: nil)
        #expect(recorder.calls.contains(["files", "folder-ls", "logs", "", "--json"]), "\(recorder.calls)")
        #expect(recorder.calls.contains(["files", "folder-cat", "logs", "nginx/a.log", "--json"]), "\(recorder.calls)")
        #expect(recorder.calls.allSatisfy { $0.count < 2 || $0[1] != "ls" }, "a worktree's reads name an id")
    }

    // MARK: - Refusals

    @Test("A folder the runner no longer shares reads as a sentence")
    func aGoneFolderReadsPlainly() async {
        let refused = "error: no such folder\ncode: not-found"
        #expect(FileReadFailure.from(cli: refused, inFolder: true) == .folderGone)
        #expect(FileReadFailure.from(cli: refused, inFolder: false) == .missingInFolder)
        #expect(FileReadFailure.folderGone.sentence == "This runner doesn’t share this folder anymore.")
        #expect(FileReadFailure.missingInFolder.sentence == "This isn’t in the folder anymore.")
        // Anything the CLI says that isn't a known code stays the general sentence.
        #expect(FileReadFailure.from(cli: "Error: Os { code: 2 }", inFolder: true) == .failed)

        let model = model(root: .failure(.folderGone))
        await model.loadIfNeeded()
        #expect(model.folders[""] == .failed(.folderGone))
        let file = self.model(["x": .failure(.missingInFolder)], root: .failure(.folderGone))
        await file.open("x", line: nil)
        #expect(file.opened?.content == .failed(.missingInFolder))
        for why in [FileReadFailure.folderGone, .missingInFolder] {
            #expect(!why.sentence.contains("Os {") && !why.sentence.contains("code"))
        }
    }

    // MARK: - Where it opens

    @Test("Opening a folder closes a worktree's Files, and opening a worktree closes the folder")
    func oneInspectorAtATime() {
        let routing = FilesRouting()
        let worktree = Worktree(
            id: "w-1", short: "w1", task: "t", branch: "t", repository: nil, host: "", path: "/tmp/t", state: "active",
            terminals: [])
        routing.toggleInspector(for: worktree)
        routing.openFolder(host: "", name: "logs")
        #expect(routing.inspector == nil)
        #expect(routing.inspectorFolder == FilesRouting.FolderRef(host: "", name: "logs"))
        #expect(routing.inspectorOpen)
        routing.toggleInspector(for: worktree)
        #expect(routing.inspectorFolder == nil && routing.inspector?.id == "w-1")
    }
}
