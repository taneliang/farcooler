import AgentKit
import CryptoKit
import Foundation
import Testing

@testable import Far_Cooler

/// A worktree says when large files weren’t downloaded, and Try Again asks the
/// runner (ov-199).
///
/// The Mac reads the count off `farcooler worktree list --json` and asks again
/// with `farcooler worktree hydrate-lfs`. The command line is held to the CLI by
/// `WorktreeCallsTests`; this holds the other half, the bytes: the real CLI,
/// against a scratch daemon of its own, makes a worktree of an LFS repository
/// whose large file isn’t in the runner’s store, and the Mac’s own decoder
/// reads what it printed.
@MainActor
struct LfsNoticeTests {
    @Test func tryAgainSendsTheRunnersCommandAndSaysWhenItCouldNotAsk() async {
        var sent: [[String]] = []
        var fails = false
        let client = DaemonClient(target: "", notifications: NotificationCenter())
        client.commandRunnerForTesting = { args in
            sent.append(args)
            return fails ? (nil, "ssh: connect to host runner: Connection refused") : (Data(#"{"lfs_pointers":0}"#.utf8), nil)
        }
        #expect(await client.hydrateLfs("w-1") == nil)
        #expect(sent.first == ["worktree", "hydrate-lfs", "w-1", "--json"])
        fails = true
        #expect(await client.hydrateLfs("w-1") != nil, "a runner that can’t be reached is said not to have been")
    }

    /// The stderr is what `farcooler worktree hydrate-lfs --json` prints for a
    /// runner without `lfs_pointers` (held by the CLI's
    /// `an_old_runner_is_refused_in_a_sentence_before_the_wire`), so the Mac
    /// says the shared sentence, never the CLI's own.
    @Test func anOldRunnersRefusalReadsAsTheSharedSentence() async {
        let stderr = "error: This runner needs an update to download large files again.\ncode: capability-unsupported"
        let client = DaemonClient(target: "", notifications: NotificationCenter())
        client.commandRunnerForTesting = { _ in (nil, stderr) }
        let message = await client.hydrateLfs("w-1")
        let said = LfsNotice.failure(word: message.flatMap { TaskFailure.code(in: $0) })
        #expect(said == "Couldn’t ask the runner to try again. \(RunnerRefusal.capabilityUnsupported.sentence)")
        #expect(!said.contains("needs an update"), "the CLI’s own sentence stayed off the screen")
    }

    @Test func theCountDecodesUnderLfsPointersAndIsAbsentFromAnOlderRunner() {
        func fleet(_ pointers: Int?) throws -> Worktree {
            let extra = pointers.map { #","lfs_pointers":\#($0)"# } ?? ""
            let row = #"{"id":"w1","short":"w1","task":"t","branch":"b","worktree":"/x","state":"ready","terminals":[]\#(extra)}"#
            return try JSONDecoder().decode(Worktree.self, from: Data(row.utf8))
        }
        #expect((try? fleet(2))?.lfsPointers == 2)
        #expect((try? fleet(nil))?.lfsPointers == nil)
    }

    // MARK: - The real CLI

    /// The CLI this tree builds, or nil.
    private static var cli: String? {
        if let bin = ProcessInfo.processInfo.environment["FARCOOLER_BIN"] { return bin }
        let root = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent()  // CeremonyTests
            .deletingLastPathComponent()  // Tests
            .deletingLastPathComponent()  // macos
            .deletingLastPathComponent()  // apps
            .deletingLastPathComponent()  // the repository
        return ["release", "debug"].map { root.appendingPathComponent("target/\($0)/farcooler").path }
            .filter { FileManager.default.isExecutableFile(atPath: $0) }
            .max {
                let date = { (p: String) in
                    (try? FileManager.default.attributesOfItem(atPath: p)[.modificationDate] as? Date) ?? .distantPast
                }
                return date($0) < date($1)
            }
    }

    /// Run `executable` with `arguments`, and answer its stdout.
    @discardableResult
    private static func run(
        _ executable: String, _ arguments: [String], home: String? = nil, in directory: String? = nil
    ) throws -> String {
        let process = Process()
        process.executableURL = URL(fileURLWithPath: executable)
        process.arguments = arguments
        var environment = ProcessInfo.processInfo.environment
        for key in environment.keys where key.hasPrefix("GIT_") { environment[key] = nil }
        environment["GIT_CONFIG_GLOBAL"] = "/dev/null"
        if let home { ScratchDaemon.isolate(&environment, home: home) }
        process.environment = environment
        if let directory { process.currentDirectoryURL = URL(fileURLWithPath: directory) }
        let out = Pipe()
        let err = Pipe()
        process.standardOutput = out
        process.standardError = err
        try process.run()
        let stdout = out.fileHandleForReading.readDataToEndOfFile()
        let stderr = err.fileHandleForReading.readDataToEndOfFile()
        process.waitUntilExit()
        guard process.terminationStatus == 0 else {
            throw LfsFixtureFailure("\(executable) \(arguments): \(String(decoding: stderr, as: UTF8.self))")
        }
        return String(decoding: stdout, as: UTF8.self)
    }

    struct LfsFixtureFailure: Error, CustomStringConvertible {
        let description: String
        init(_ description: String) { self.description = description }
    }

    @Test func theRealCLIsCountReachesTheMacAndTryAgainClearsIt() async throws {
        let cli = try #require(
            Self.cli, "No farcooler CLI to read. Build it first: cargo build --bin farcooler, or set FARCOOLER_BIN.")
        // A short home: a unix socket path has a length limit.
        let base = "/tmp/fc-t/lfs-\(UUID().uuidString.prefix(8))"
        // Stopped, with the tmux server `daemon stop` leaves running, and
        // removed on every way out, a failed expectation included: `defer`
        // can't await.
        do {
            try scenario(cli: cli, base: base)
        } catch {
            await ScratchDaemon.stop(cli: cli, farcoolerHome: "\(base)/h")
            try? FileManager.default.removeItem(atPath: base)
            throw error
        }
        await ScratchDaemon.stop(cli: cli, farcoolerHome: "\(base)/h")
        try? FileManager.default.removeItem(atPath: base)
    }

    private func scenario(cli: String, base: String) throws {
        let home = "\(base)/h"
        let work = "\(base)/work"
        let repo = "\(work)/proj"
        try FileManager.default.createDirectory(atPath: repo, withIntermediateDirectories: true)

        // A repository whose `big.bin` is committed as a pointer.
        let content = Data((0..<4096).map { UInt8(($0 * 7 + 3) % 256) })
        let oid = Self.sha256Hex(content)
        let pointer = "version https://git-lfs.github.com/spec/v1\noid sha256:\(oid)\nsize \(content.count)\n"
        try "*.bin filter=lfs -text\n".write(toFile: "\(repo)/.gitattributes", atomically: true, encoding: .utf8)
        try pointer.write(toFile: "\(repo)/big.bin", atomically: true, encoding: .utf8)
        let git = "/usr/bin/git"
        let quiet = ["-c", "filter.lfs.process=", "-c", "user.name=t", "-c", "user.email=t@example.com", "-c", "commit.gpgsign=false"]
        try Self.run(git, ["init", "-q", "-b", "main"], in: repo)
        try Self.run(git, quiet + ["add", "-A"], in: repo)
        try Self.run(git, quiet + ["commit", "-q", "-m", "base"], in: repo)

        try Self.run(cli, ["root", "add", work], home: home)
        try Self.run(cli, ["repo", "register", repo], home: home)
        try Self.run(cli, ["worktree", "create", "proj", "feature", "--branch", "feature"], home: home)

        func count() throws -> Int? {
            let listed = try Self.run(cli, ["worktree", "list", "--json"], home: home)
            let fleet = try JSONDecoder().decode(Fleet.self, from: Data(listed.utf8))
            return try #require(fleet.worktrees.first { $0.task == "feature" }).lfsPointers
        }
        // The object isn't in the runner's store, so the file stays a pointer.
        #expect(try count() == 1, "the CLI's row did not carry the count the Mac decodes")
        #expect(try Self.run(cli, ["worktree", "hydrate-lfs", "feature", "--json"], home: home).contains(#""lfs_pointers":1"#))

        // The object arrives, and Try Again downloads it.
        let store = "\(repo)/.git/lfs/objects/\(oid.prefix(2))/\(oid.dropFirst(2).prefix(2))"
        try FileManager.default.createDirectory(atPath: store, withIntermediateDirectories: true)
        try content.write(to: URL(fileURLWithPath: "\(store)/\(oid)"))
        #expect(try Self.run(cli, ["worktree", "hydrate-lfs", "feature", "--json"], home: home).contains(#""lfs_pointers":0"#))
        #expect(try count() == 0)
    }

    private static func sha256Hex(_ data: Data) -> String {
        var digest = CryptoKit.SHA256()
        digest.update(data: data)
        return digest.finalize().map { String(format: "%02x", $0) }.joined()
    }
}
