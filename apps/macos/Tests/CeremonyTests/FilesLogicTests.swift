import AgentKit
import Foundation
import Testing

@testable import Far_Cooler

/// The Files tab's rules (ov-189): the tree it draws, what an agent's path
/// means, where a link leads, what a find matches, what Copy copies.
struct FilesLogicTests {
    private static func entry(_ name: String, _ kind: FileEntry.Kind = .file) -> FileEntry {
        FileEntry(name: name, kind: kind, size: 0, linkTarget: "")
    }

    @Test("The tree draws a directory's children under it only when it's open and read")
    func theTreeDrawsWhatIsOpenAndRead() {
        let listings: [String: FileListing] = [
            "": FileListing(path: "", entries: [Self.entry("src", .directory), Self.entry("docs", .directory), Self.entry("README.md")], truncated: false),
            "src": FileListing(path: "src", entries: [Self.entry("deep", .directory), Self.entry("main.rs")], truncated: false),
            "src/deep": FileListing(path: "src/deep", entries: [Self.entry("a.rs")], truncated: false),
        ]
        let closed = FilesLogic.rows(listings: listings, expanded: [])
        #expect(closed.map(\.id) == ["src", "docs", "README.md"])
        let open = FilesLogic.rows(listings: listings, expanded: ["src", "src/deep", "docs"])
        #expect(open.map(\.id) == ["src", "src/deep", "src/deep/a.rs", "src/main.rs", "docs", "README.md"])
        #expect(open.map(\.depth) == [0, 1, 2, 1, 0, 0])
        // `docs` is open but not read yet: nothing under it, and it says it's open.
        #expect(open.first { $0.id == "docs" }?.expanded == true)
        // A child of a closed directory isn't drawn even though it's read.
        let parentClosed = FilesLogic.rows(listings: listings, expanded: ["src/deep"])
        #expect(!parentClosed.contains { $0.id == "src/deep/a.rs" })
        #expect(FilesLogic.ancestors(of: "src/deep/a.rs") == ["src", "src/deep"])
        #expect(FilesLogic.ancestors(of: "README.md") == [])
    }

    @Test("An agent's absolute path is a path in the worktree only when it's under the root")
    func anAgentsPathIsInsideOnlyUnderTheRoot() {
        let root = "/Users/me/wt/ov-189"
        #expect(FilesLogic.relative("/Users/me/wt/ov-189/src/main.rs", in: root) == "src/main.rs")
        #expect(FilesLogic.relative("/Users/me/wt/ov-189/src/main.rs", in: root + "/") == "src/main.rs")
        // macOS reports /tmp paths under /private.
        #expect(FilesLogic.relative("/private/tmp/w/a.txt", in: "/tmp/w") == "a.txt")
        // A sibling whose name starts the same is not inside.
        #expect(FilesLogic.relative("/Users/me/wt/ov-1890/x", in: root) == nil)
        #expect(FilesLogic.relative("/Users/me/.ssh/id_ed25519", in: root) == nil)
        #expect(FilesLogic.relative("/Users/me/wt/ov-189/../other/x", in: root) == nil)
        #expect(FilesLogic.relative("/Users/me/wt/ov-189", in: root) == nil)
        // A relative path is taken as the worktree's already.
        #expect(FilesLogic.relative("src/./lib.rs", in: root) == "src/lib.rs")
    }

    @Test("A link opens where it leads only while that stays inside the worktree")
    func aLinkLeadsInsideOrNowhere() {
        let root = "/w"
        #expect(FilesLogic.linkDestination("docs/latest", target: "v2/index.md", root: root) == "docs/v2/index.md")
        #expect(FilesLogic.linkDestination("docs/latest", target: "../README.md", root: root) == "README.md")
        #expect(FilesLogic.linkDestination("docs/latest", target: "../../etc/hosts", root: root) == nil)
        #expect(FilesLogic.linkDestination("a", target: "/etc/hosts", root: root) == nil)
        #expect(FilesLogic.linkDestination("a", target: "/w/b", root: root) == "b")
    }

    @Test("A file's lines: no phantom line after the last newline, CRLF read as one")
    func linesAreNumberedAsAnEditorNumbersThem() {
        #expect(FilesLogic.lines(of: "") == [])
        #expect(FilesLogic.lines(of: "one") == ["one"])
        #expect(FilesLogic.lines(of: "one\n") == ["one"])
        #expect(FilesLogic.lines(of: "one\r\ntwo\r\n") == ["one", "two"])
        #expect(FilesLogic.lines(of: "one\r\n\r\nthree") == ["one", "", "three"])
        #expect(FilesLogic.lines(of: "one\rtwo\r") == ["one", "two"], "a bare return breaks a line too")
        #expect(FilesLogic.lines(of: "mixed\r\nends\nhere\r") == ["mixed", "ends", "here"])
        #expect(FilesLogic.lines(of: "one\n\nthree\n") == ["one", "", "three"])
        #expect(FilesLogic.lines(of: "\n") == [""])
    }

    @Test("Find matches every place, ignoring case, in order, and stops at the cap")
    func findMatchesEveryPlace() {
        let lines = ["let token = Token()", "no match", "TOKEN token"]
        let found = FilesLogic.matches("token", in: lines)
        #expect(found.map(\.line) == [0, 0, 2, 2])
        #expect(String(lines[0][found[1].range]) == "Token")
        #expect(FilesLogic.matches("", in: lines).isEmpty)
        #expect(FilesLogic.matches("aa", in: ["aaaa"]).count == 2, "matches don't overlap")
        #expect(FilesLogic.matches("t", in: lines, limit: 3).count == 3)
    }

    @Test("Go to line reads what people type and stays inside the file")
    func goToLineReadsWhatPeopleType() {
        #expect(FilesLogic.line(from: "12", count: 100) == 12)
        #expect(FilesLogic.line(from: " L12 ", count: 100) == 12)
        #expect(FilesLogic.line(from: ":12", count: 100) == 12)
        #expect(FilesLogic.line(from: "500", count: 100) == 100)
        #expect(FilesLogic.line(from: "0", count: 100) == 1)
        #expect(FilesLogic.line(from: "twelve", count: 100) == nil)
        #expect(FilesLogic.line(from: "3", count: 0) == nil)
    }

    @Test("Copy takes the chosen lines; Copy Reference says where they are")
    func copyTakesTheChosenLines() {
        let lines = ["a", "b", "c", "d"]
        #expect(FilesLogic.copied(lines, range: 1...2) == "b\nc")
        #expect(FilesLogic.copied(lines, range: 3...9) == "d")
        #expect(FilesLogic.copied([], range: 0...0) == "")
        #expect(FilesLogic.reference("src/a.rs", range: 11...11) == "src/a.rs:12")
        #expect(FilesLogic.reference("src/a.rs", range: 11...29) == "src/a.rs:12-30")
        #expect(FilesLogic.reference("src/a.rs", range: nil) == "src/a.rs")
        #expect(FilesLogic.extend(from: 7, to: 3) == 3...7)
    }

    @Test("The title bar's find lists the files found for what's typed, and only for that")
    func findListsTheFilesFoundForWhatsTyped() {
        let files = PaletteFiles(
            worktree: Worktree(
                id: "w-1", short: "w1", task: "t", branch: "b", repository: nil, host: "", path: "/tmp/w",
                state: "active", terminals: []),
            search: { _ in [] })
        let entry = files.entry("src/main.rs")
        #expect(entry.action == .openFile(worktree: "w-1", path: "src/main.rs"))
        #expect(entry.title == "main.rs" && entry.detail == "src/main.rs")

        let actions = TitleConsoleActions(find: { _ in [] }, files: files)
        var console = TitleConsole()
        console.open(recents: false)
        console.edit("ma")
        console.found(files: ["src/main.rs"], for: "m")
        #expect(actions.entries(console).isEmpty, "an answer for an older query isn't listed")
        console.found(files: ["src/main.rs"], for: "ma")
        #expect(actions.entries(console).map(\.action) == [.openFile(worktree: "w-1", path: "src/main.rs")])
        console.edit("mai")
        #expect(actions.entries(console).isEmpty, "typing on hides what the last query found")
        console.close()
        console.found(files: ["src/main.rs"], for: "ma")
        #expect(actions.entries(console).isEmpty, "a closed field finds nothing")
    }
}
