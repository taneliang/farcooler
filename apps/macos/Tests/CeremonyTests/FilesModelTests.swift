import Foundation
import Testing

@testable import Far_Cooler

/// A worktree's Files (ov-189), against a runner that answers from a table:
/// what it reads and when, what a file opens as, and the find in it.
@MainActor
struct FilesModelTests {
    /// A runner with a few files, counting what it was asked.
    @MainActor
    final class Runner {
        var listed: [String] = []
        var read: [String] = []
        var searched: [String] = []
        var files: [String: FileRead] = [:]
        var dirs: [String: [FileEntry]] = [:]

        var source: FilesModel.Source {
            FilesModel.Source(
                list: { [self] path in
                    listed.append(path)
                    guard let entries = dirs[path] else { return .failure(.missing) }
                    return .success(FileListing(path: path, entries: entries, truncated: false))
                },
                read: { [self] path in
                    read.append(path)
                    guard let file = files[path] else { return .failure(.missing) }
                    return .success(file)
                },
                search: { [self] query in
                    searched.append(query)
                    return ["src/main.rs"]
                })
        }
    }

    private static let worktree = Worktree(
        id: "w-1", short: "w1", task: "fix-it", branch: "fix-it", repository: nil, host: "", path: "/tmp/fix-it",
        state: "active", terminals: [])

    private static func text(_ path: String, _ text: String) -> FileRead {
        FileRead(path: path, state: .text, size: UInt64(text.utf8.count), text: text, linkTarget: "")
    }

    private func runner() -> Runner {
        let runner = Runner()
        runner.dirs = [
            "": [FileEntry(name: "src", kind: .directory, size: 0, linkTarget: ""),
                 FileEntry(name: ".env", kind: .file, size: 12, linkTarget: "")],
            "src": [FileEntry(name: "main.rs", kind: .file, size: 40, linkTarget: "")],
        ]
        runner.files = [
            "src/main.rs": Self.text("src/main.rs", "fn main() {\n    let token = 1;\n    print(token);\n}\n"),
            ".env": Self.text(".env", "TOKEN=plain\n"),
            "windows.txt": Self.text("windows.txt", "first\r\nsecond line\r\n\r\nfourth\r\n"),
            "big.log": FileRead(path: "big.log", state: .tooLarge, size: 9_000_000, text: "", linkTarget: ""),
            "latest": FileRead(path: "latest", state: .link, size: 0, text: "", linkTarget: "src/main.rs"),
        ]
        return runner
    }

    @Test("Nothing is read until it's asked for, and the root only once")
    func nothingIsReadUntilAsked() async {
        let runner = runner()
        let model = FilesModel(worktree: Self.worktree, source: runner.source)
        #expect(runner.listed.isEmpty && runner.read.isEmpty)
        await model.loadIfNeeded()
        await model.loadIfNeeded()
        #expect(runner.listed == [""])
        await model.toggle("src")
        #expect(runner.listed == ["", "src"])
        #expect(model.rows.map(\.id) == ["src", "src/main.rs", ".env"])
        await model.toggle("src")
        await model.toggle("src")
        #expect(runner.listed == ["", "src"], "a directory read once isn't read again on reopening")
    }

    @Test("Opening a file reads it whole, shows it in the tree, and goes to the line")
    func openingAFileShowsItAtTheLine() async {
        let runner = runner()
        let model = FilesModel(worktree: Self.worktree, source: runner.source)
        await model.loadIfNeeded()
        await model.open("src/main.rs", line: 3)
        #expect(model.lines == ["fn main() {", "    let token = 1;", "    print(token);", "}"])
        #expect(model.expanded.contains("src"), "the tree opens to the file")
        #expect(model.selection == 2...2)
        #expect(model.scroll?.line == 2)
        #expect(model.widest == "    let token = 1;".count)
        // A .env is shown as it is.
        await model.open(".env", line: nil)
        #expect(model.lines == ["TOKEN=plain"])
        #expect(model.selection == nil, "a new file starts with nothing chosen")
    }

    /// integ-8 found a CRLF file drawn as one line: Swift reads `\r\n` as one
    /// character, so a split on `"\n"` never split it.
    @Test("A Windows file opens as its lines, with no return shown")
    func aCRLFFileOpensAsItsLines() async {
        let model = FilesModel(worktree: Self.worktree, source: runner().source)
        await model.open("windows.txt", line: 4)
        #expect(model.lines == ["first", "second line", "", "fourth"])
        #expect(!model.lines.contains { $0.unicodeScalars.contains("\r") })
        #expect(model.widest == "second line".count)
        #expect(model.selection == 3...3)
    }

    @Test("A file too large, a link, and a missing one each say what they are")
    func eachKindOfFileSaysWhatItIs() async {
        let model = FilesModel(worktree: Self.worktree, source: runner().source)
        await model.open("big.log", line: nil)
        #expect(model.opened?.content == .tooLarge(9_000_000))
        await model.open("latest", line: nil)
        #expect(model.opened?.content == .link(target: "src/main.rs", inside: "src/main.rs"))
        await model.open("gone.txt", line: nil)
        #expect(model.opened?.content == .failed(.missing))
        #expect(model.lines.isEmpty)
    }

    @Test("Find steps through every match, wrapping, and follows a new file")
    func findStepsThroughMatches() async {
        let model = FilesModel(worktree: Self.worktree, source: runner().source)
        await model.open("src/main.rs", line: nil)
        model.findQuery = "TOKEN"
        model.refind()
        #expect(model.matches.map(\.line) == [1, 2])
        #expect(model.currentMatch == 0)
        model.step(forward: true)
        #expect(model.currentMatch == 1 && model.scroll?.line == 2)
        model.step(forward: true)
        #expect(model.currentMatch == 0, "wraps forward")
        model.step(forward: false)
        #expect(model.currentMatch == 1, "wraps back")
        await model.open(".env", line: nil)
        #expect(model.matches.map(\.line) == [0], "a file opened with a find up is searched too")
    }

    @Test("Shift-click grows the selection from where it started; Copy copies it")
    func shiftClickGrowsTheSelection() async {
        let model = FilesModel(worktree: Self.worktree, source: runner().source)
        await model.open("src/main.rs", line: nil)
        model.click(line: 1, extending: false)
        model.click(line: 3, extending: true)
        #expect(model.selection == 1...3)
        model.click(line: 0, extending: true)
        #expect(model.selection == 0...1)
        #expect(model.copiedText == "fn main() {\n    let token = 1;")
    }

    @Test("The filter searches the whole worktree on the runner, and only the newest query lands")
    func theFilterSearchesOnTheRunner() async {
        let runner = runner()
        let model = FilesModel(worktree: Self.worktree, source: runner.source)
        model.filter = " main "
        await model.runFilter()
        #expect(runner.searched == ["main"])
        #expect(model.found == ["src/main.rs"])
        model.filter = ""
        await model.runFilter()
        #expect(model.found.isEmpty)
        #expect(runner.searched == ["main"], "an empty filter asks nothing")
    }

    @Test("The CLI's refusals read as sentences, never its words")
    func refusalsReadAsSentences() {
        #expect(FileReadFailure.from(cli: "error: nothing by that name\ncode: not-found") == .missing)
        #expect(FileReadFailure.from(cli: "error: that's a directory\ncode: invalid-argument") == .notAFile)
        #expect(
            FileReadFailure.from(cli: "error: this runner needs an update to show a worktree's files")
                == .runnerTooOld)
        #expect(FileReadFailure.from(cli: "ssh: connect to host runner: Connection refused") == .failed)
        for failure in [FileReadFailure.runnerTooOld, .missing, .notAFile, .failed] {
            #expect(!failure.sentence.contains("error:") && failure.sentence.hasSuffix("."))
        }
    }
}
