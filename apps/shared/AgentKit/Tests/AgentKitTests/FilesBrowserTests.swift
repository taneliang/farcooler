import Foundation
import Testing

@testable import AgentKit

// The phones' read-only Files browser, minus its views (ov-259), and the
// pieces the Mac's Files tab shares with it. Android's `FilesTest` reads the
// same `test/fixtures/files-lines.json`, because Kotlin cannot import this.

private struct FilesFixture: Decodable {
    struct Lines: Decodable {
        var text: String
        var lines: [String]
    }
    struct Display: Decodable {
        var prefix: String
        var `repeat`: String
        var count: Int
        var limit: Int
        var keeps: Int
        var cut: Bool
    }
    var lines: [Lines]
    var display: [Display]

    static func load() throws -> FilesFixture {
        var root = URL(fileURLWithPath: #filePath)
        // …/apps/shared/AgentKit/Tests/AgentKitTests/<this file>
        for _ in 0..<6 { root.deleteLastPathComponent() }
        let data = try Data(contentsOf: root.appendingPathComponent("test/fixtures/files-lines.json"))
        return try JSONDecoder().decode(FilesFixture.self, from: data)
    }
}

struct FilesTextTests {
    @Test func everyLineCaseInTheSharedFixtureBreaksAsItSays() throws {
        let fixture = try FilesFixture.load()
        #expect(fixture.lines.count >= 10)
        for c in fixture.lines {
            #expect(FilesText.lines(of: c.text) == c.lines, "\(c.text.debugDescription)")
        }
    }

    @Test func everyDisplayCaseInTheSharedFixtureCutsWhereItSays() throws {
        let fixture = try FilesFixture.load()
        #expect(fixture.display.count >= 6)
        for c in fixture.display {
            let text = c.prefix + String(repeating: c.repeat, count: c.count)
            let shown = FilesText.display(text, limit: c.limit)
            #expect(shown.cut == c.cut, "\(c)")
            if c.cut {
                let kept = String(decoding: Array(text.utf16.prefix(c.keeps)), as: UTF16.self)
                #expect(shown.text == kept + "…", "\(c)")
            } else {
                #expect(shown.text == text, "\(c)")
            }
        }
    }

    @Test func aFileOfOneHugeLineIsOneShortLine() {
        let huge = String(repeating: "x", count: 512 * 1024)
        guard case .code(let code) = FilesItemModel.file(
            FileRead(path: "min.js", state: .text, size: UInt64(huge.utf8.count), text: huge, linkTarget: ""),
            at: FilesLocation(place: .worktree("w"), path: "min.js", expecting: .file), root: "")
        else { Issue.record("not code"); return }
        #expect(code.lines.count == 1)
        #expect(code.lines[0].utf16.count == FilesText.lineLimit + 1)
        #expect(code.anyCut)
        #expect(code.widest == FilesText.lineLimit + 1)
    }
}

struct FilesPathsTests {
    @Test func aLinkLeadsWhereItPointsOrNowhere() {
        #expect(FilesPaths.linkDestination("docs/latest", target: "v2/readme.md", root: "") == "docs/v2/readme.md")
        #expect(FilesPaths.linkDestination("docs/latest", target: "../src/main.rs", root: "") == "src/main.rs")
        #expect(FilesPaths.linkDestination("docs/latest", target: "../../etc/passwd", root: "") == nil)
        // A phone never learns the runner's path, so an absolute target leads nowhere.
        #expect(FilesPaths.linkDestination("latest", target: "/var/log/syslog", root: "") == nil)
        #expect(FilesPaths.linkDestination("latest", target: "/repo/src/a.rs", root: "/repo") == "src/a.rs")
    }

    @Test func failuresAreSentencesNotWords() {
        #expect(FileReadFailure.from(refusal: "not-found", inFolder: false, atRoot: false) == .missing)
        #expect(FileReadFailure.from(refusal: "not-found", inFolder: true, atRoot: true) == .folderGone)
        #expect(FileReadFailure.from(refusal: "not-found", inFolder: true, atRoot: false) == .missingInFolder)
        #expect(FileReadFailure.from(refusal: "invalid-argument", inFolder: false, atRoot: false) == .notAFile)
        #expect(FileReadFailure.from(refusal: "capability-unsupported", inFolder: true, atRoot: true) == .runnerTooOld)
        #expect(FileReadFailure.from(refusal: nil, inFolder: false, atRoot: false) == .failed)
        for why: FileReadFailure in [.runnerTooOld, .missing, .notAFile, .missingInFolder, .folderGone, .failed] {
            #expect(!why.sentence.contains("-"), "\(why)")
            #expect(!why.directorySentence.isEmpty)
        }
        #expect(FileReadFailure.missing.directorySentence != FileReadFailure.missing.sentence)
    }

    @Test func aFileStateOrKindANewerRunnerSendsIsStillDecoded() throws {
        let read = try JSONDecoder().decode(
            FileRead.self,
            from: Data(#"{"path":"a","state":"hologram","size":1,"text":"","linkTarget":""}"#.utf8))
        #expect(read.state == .unknown)
        let tooLarge = try JSONDecoder().decode(
            FileRead.self,
            from: Data(#"{"path":"a","state":"too_large","size":9,"text":"","linkTarget":""}"#.utf8))
        #expect(tooLarge.state == .tooLarge)
        let dir = try JSONDecoder().decode(
            FileListing.self,
            from: Data(
                #"{"path":"","truncated":false,"entries":[{"name":"x","kind":"socket","size":0,"linkTarget":""}]}"#.utf8))
        #expect(dir.entries[0].kind == .other)
    }
}

/// A fake runner: directories by path, files by path, and a count of reads.
private final class FilesFakeRunner: @unchecked Sendable {
    var dirs: [String: FileListing] = [:]
    var files: [String: FileRead] = [:]
    private(set) var listed: [String] = []
    private(set) var read: [String] = []

    var source: FilesSource {
        FilesSource(
            list: { [self] place, path in
                listed.append(path)
                if let listing = dirs[path] { return .success(listing) }
                return .failure(files[path] != nil ? .notAFile : .missing)
            },
            read: { [self] place, path in
                read.append(path)
                if let file = files[path] { return .success(file) }
                return .failure(dirs[path] != nil ? .notAFile : .missing)
            })
    }
}

private func entry(_ name: String, _ kind: FileEntry.Kind, size: UInt64 = 0, to target: String = "") -> FileEntry {
    FileEntry(name: name, kind: kind, size: size, linkTarget: target)
}

@MainActor
struct FilesItemModelTests {
    let wt = FilesPlace.worktree("w1")

    private func runner() -> FilesFakeRunner {
        let r = FilesFakeRunner()
        r.dirs[""] = FileListing(
            path: "",
            entries: [
                entry("src", .directory), entry("README.md", .file, size: 2_048),
                entry("latest", .link, to: "src/main.rs"), entry("out", .link, to: "/var/out"),
                entry("fifo", .other),
            ],
            truncated: false)
        r.dirs["src"] = FileListing(path: "src", entries: [entry("main.rs", .file, size: 12)], truncated: false)
        r.files["src/main.rs"] = FileRead(
            path: "src/main.rs", state: .text, size: 12, text: "fn main() {}\r\n}\n", linkTarget: "")
        r.files["README.md"] = FileRead(path: "README.md", state: .binary, size: 12_000_000, text: "", linkTarget: "")
        r.files["latest"] = FileRead(path: "latest", state: .link, size: 0, text: "", linkTarget: "src/main.rs")
        return r
    }

    @Test func aDirectoryListsRowsThatLeadToTheNextScreen() async {
        let r = runner()
        let model = FilesItemModel(location: FilesLocation(place: wt), source: r.source)
        await model.load()
        guard case .directory(let dir) = model.content else { Issue.record("\(model.content)"); return }
        #expect(dir.rows.map(\.name) == ["src", "README.md", "latest", "out", "fifo"])
        #expect(dir.rows[0].destination == FilesLocation(place: wt, path: "src", expecting: .directory))
        #expect(dir.rows[1].destination == FilesLocation(place: wt, path: "README.md", expecting: .file))
        #expect(dir.rows[1].detail == FilesText.size(2_048))
        #expect(dir.rows[2].detail == "→ src/main.rs")
        #expect(dir.rows[2].destination == FilesLocation(place: wt, path: "src/main.rs", expecting: .either))
        // A link out of the place, and what isn't a file, lead nowhere.
        #expect(dir.rows[3].destination == nil)
        #expect(dir.rows[4].destination == nil)
        #expect(dir.footer == nil && dir.empty == nil)
    }

    @Test func aChildScreenReadsOnlyItsOwnPathAndTheTextIsNumberedByLine() async {
        let r = runner()
        let model = FilesItemModel(
            location: FilesLocation(place: wt, path: "src/main.rs", expecting: .file), source: r.source)
        await model.load()
        guard case .code(let code) = model.content else { Issue.record("\(model.content)"); return }
        #expect(code.lines == ["fn main() {}", "}"])
        #expect(code.gutterDigits == 1 && !code.anyCut)
        #expect(r.read == ["src/main.rs"] && r.listed.isEmpty)
    }

    @Test func aFolderRootIsTitledByItsNameAndAChildByItsLastName() {
        #expect(FilesLocation(place: .folder("logs")).title == "logs")
        #expect(FilesLocation(place: .worktree("w")).title == "Files")
        #expect(FilesLocation(place: .worktree("w"), path: "src/lib").title == "lib")
    }

    /// A Linux runner's two names that differ only in bytes UTF-8 can't read
    /// both arrive as "x\u{FFFD}"; a list keyed by name would repeat an id.
    @Test func twoNamesThatDecodeAlikeAreTwoRowsWithTwoIds() {
        let dir = FilesItemModel.directory(
            FileListing(
                path: "", entries: [entry("x\u{FFFD}", .file), entry("x\u{FFFD}", .file), entry("y", .directory)],
                truncated: false),
            at: FilesLocation(place: wt), root: "")
        #expect(dir.rows.count == 3)
        #expect(Set(dir.rows.map(\.id)).count == 3)
    }

    @Test func aTruncatedListingAndAnEmptyOneSaySo() {
        let cut = FilesItemModel.directory(
            FileListing(path: "", entries: (0..<5_000).map { entry("f\($0)", .file) }, truncated: true),
            at: FilesLocation(place: wt), root: "")
        #expect(cut.footer == "Showing the first 5,000 items.")
        let empty = FilesItemModel.directory(
            FileListing(path: "", entries: [], truncated: false), at: FilesLocation(place: wt), root: "")
        #expect(empty.empty == "This folder is empty.")
    }

    @Test func binaryAndTooLargeFilesSayHowBigTheyAreAndNothingElse() {
        let here = FilesLocation(place: wt, path: "a", expecting: .file)
        let size = FilesText.size(12_000_000)
        #expect(
            FilesItemModel.file(
                FileRead(path: "a", state: .binary, size: 12_000_000, text: "", linkTarget: ""), at: here, root: "")
                == .message("This is a binary file. It’s \(size)."))
        #expect(
            FilesItemModel.file(
                FileRead(path: "a", state: .tooLarge, size: 12_000_000, text: "", linkTarget: ""), at: here, root: "")
                == .message("This file is too large to show here. It’s \(size)."))
    }

    @Test func aLinkScreenOffersItsDestinationOnlyInsideThePlace() {
        let here = FilesLocation(place: .folder("logs"), path: "current", expecting: .file)
        let inside = FilesItemModel.file(
            FileRead(path: "current", state: .link, size: 0, text: "", linkTarget: "2026/a.log"), at: here, root: "")
        #expect(
            inside
                == .link(
                    target: "2026/a.log",
                    destination: FilesLocation(place: .folder("logs"), path: "2026/a.log", expecting: .either)))
        let outside = FilesItemModel.file(
            FileRead(path: "current", state: .link, size: 0, text: "", linkTarget: "/etc/hosts"), at: here, root: "")
        #expect(outside == .link(target: "/etc/hosts", destination: nil))
    }

    @Test func aLinkToAFolderIsReadAsAFileFirstAndThenListed() async {
        let r = runner()
        let model = FilesItemModel(
            location: FilesLocation(place: wt, path: "src", expecting: .either), source: r.source)
        await model.load()
        guard case .directory(let dir) = model.content else { Issue.record("\(model.content)"); return }
        #expect(dir.rows.map(\.name) == ["main.rs"])
        #expect(r.read == ["src"] && r.listed == ["src"])
    }

    @Test func aFailureIsOneSentenceForTheKindOfScreen() async {
        let r = FilesFakeRunner()
        let gone = FilesItemModel(
            location: FilesLocation(place: wt, path: "gone", expecting: .file), source: r.source)
        await gone.load()
        #expect(gone.content == .failed(FileReadFailure.missing.sentence))
        let dir = FilesItemModel(
            location: FilesLocation(place: wt, path: "gone", expecting: .directory), source: r.source)
        await dir.load()
        #expect(dir.content == .failed(FileReadFailure.missing.directorySentence))
    }

    @Test func aPlaceAndItsPathSurviveASavedStack() throws {
        let location = FilesLocation(place: .folder("logs"), path: "2026/a.log", expecting: .either)
        let data = try JSONEncoder().encode(location)
        #expect(try JSONDecoder().decode(FilesLocation.self, from: data) == location)
    }
}

struct FilesWireTests {
    @Test func aCallNamesAWorktreeOrAFolderNeverBoth() {
        let wt = FilesWire.arguments(.worktree("w1"), path: "src") as? [String: String]
        #expect(wt == ["worktree": "w1", "path": "src"])
        let folder = FilesWire.arguments(.folder("logs"), path: "") as? [String: String]
        #expect(folder == ["folder": "logs", "path": ""])
    }

    @Test func theCoreSAnswerIsDecodedAndItsRefusalIsASentence() async {
        struct Refused: Error {}
        let source = FilesWire.source(
            call: { method, args in
                if method == FilesWire.listMethod, args["folder"] == nil {
                    return Data(
                        #"{"path":"","truncated":false,"entries":[{"name":"a","kind":"file","size":3,"linkTarget":""}]}"#
                            .utf8)
                }
                throw Refused()
            },
            refusalWord: { _ in "not-found" })
        let listing = try? await source.list(.worktree("w"), "").get()
        #expect(listing?.entries.map(\.name) == ["a"])
        let missing = await source.read(.folder("logs"), "")
        #expect(missing == .failure(.folderGone))
        let inside = await source.read(.folder("logs"), "a")
        #expect(inside == .failure(.missingInFolder))
        let broken = FilesWire.source(call: { _, _ in Data("[]".utf8) }, refusalWord: { _ in nil })
        #expect(await broken.list(.worktree("w"), "") == .failure(.failed))
    }
}

struct FilesDoorTests {
    private func build(
        _ caps: Set<String>, scope: String = "control", folders: [String]? = nil
    ) -> DaemonBuild {
        DaemonBuild(
            version: "1", matches: true, platform: "linux", capabilities: caps, grantedScope: scope,
            readOnlyFolders: folders)
    }

    @Test func filesAreOfferedOnlyWhereTheRunnerServesThemAndTheGrantMayRead() {
        #expect(build(["worktree_files"]).offersFiles)
        #expect(!build(["tasks"]).offersFiles, "an older runner")
        #expect(!build(["worktree_files"], scope: "read").offersFiles, "a read grant is refused by the runner")
        #expect(build(["worktree_files"], scope: "unspecified").offersFiles, "no answer is not no permission")
        #expect(!build([]).offersFiles, "a runner older than capabilities")
    }

    @Test func foldersAreOfferedWithTheirCapabilityAndAtLeastOneName() {
        let both: Set<String> = ["worktree_files", "read_only_folders"]
        #expect(build(both, folders: ["logs", "notes"]).sharedFolders == ["logs", "notes"])
        #expect(build(both, folders: []).sharedFolders.isEmpty)
        #expect(build(both, folders: nil).sharedFolders.isEmpty)
        #expect(build(["worktree_files"], folders: ["logs"]).sharedFolders.isEmpty, "names without the capability")
        #expect(build(both, scope: "read", folders: ["logs"]).sharedFolders.isEmpty)
    }
}

struct HostBuildTests {
    /// What the client core's `host` answers for a runner that shares a folder
    /// (`crates/client/src/ffi/files_phone_tests.rs` pins the Rust side).
    private let wire = """
        {"runnerId":"r1","pushPaired":false,"agentsFound":["claude"],"daemonVersion":"1","buildsMatch":true,
         "capabilities":["worktree_files","read_only_folders"],"grantedScope":"control","platform":"linux",
         "readOnlyFolders":["logs","notes"]}
        """

    @Test func theRealHostAnswerShowsTheRunnersFolders() throws {
        let body = try #require(
            JSONSerialization.jsonObject(with: Data(wire.utf8)) as? [String: Any])
        let build = DaemonBuild(host: body)
        #expect(build.readOnlyFolders == ["logs", "notes"])
        #expect(build.sharedFolders == ["logs", "notes"])
        #expect(build.offersFiles)
        #expect(build.agentsFound == ["claude"] && build.runnerId == "r1")
    }

    @Test func aRunnerThatSaysNothingOffersNothing() {
        let build = DaemonBuild(host: [:])
        #expect(build.readOnlyFolders == nil && build.sharedFolders.isEmpty && !build.offersFiles)
        #expect(build.grantedScope == "unspecified" && build.version == "unknown")
    }
}
