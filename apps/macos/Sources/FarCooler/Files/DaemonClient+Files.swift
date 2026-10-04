import AgentKit
import Foundation

extension FileReadFailure {
    /// From the CLI's stderr, by the `code:` word `--json` puts under it.
    static func from(cli message: String?) -> FileReadFailure {
        switch TaskFailure.code(in: message) {
        case "not-found": return .missing
        case "invalid-argument": return .notAFile
        default:
            // The CLI's own refusal for a runner without the capability
            // carries no runner code.
            return (message ?? "").contains("needs an update") ? .runnerTooOld : .failed
        }
    }
}

extension FileReadFailure {
    /// As `from(cli:)`, for a read in an extra read-only folder (ov-232): the
    /// runner answers a name it no longer shares, and a path it refuses, as
    /// "not found", so nothing at the folder's root means the folder is gone.
    static func from(cli message: String?, inFolder atRoot: Bool) -> FileReadFailure {
        let why = from(cli: message)
        guard why == .missing else { return why }
        return atRoot ? .folderGone : .missingInFolder
    }
}

extension DaemonClient {
    /// Whether this runner can show a worktree's files: nil until its status
    /// has been read. `"worktree_files"` is
    /// `farcooler_protocol::capability::WORKTREE_FILES`.
    var showsFiles: Bool? {
        daemonBuild.map { $0.can(.worktreeFiles) }
    }

    /// Every read here puts `--` before the names it passes: a file in a
    /// directory like /var/log is named by whoever writes there, and one
    /// called `--runner=x` must be read as a name, never as a flag.

    /// One directory of `worktree`, `path` relative to its root ("" is the
    /// root). `files ls`.
    func listFiles(in worktree: Worktree, path: String) async -> Result<FileListing, FileReadFailure> {
        let (data, message) = await runRaw(["files", "ls", "--json", "--", worktree.short, path], background: true)
        guard let data else { return .failure(.from(cli: message)) }
        guard let listing = try? JSONDecoder().decode(FileListing.self, from: data) else { return .failure(.failed) }
        return .success(listing)
    }

    /// One file of `worktree`, whole, up to the runner's 512 KiB. `files cat`.
    func readFile(in worktree: Worktree, path: String) async -> Result<FileRead, FileReadFailure> {
        let (data, message) = await runRaw(["files", "cat", "--json", "--", worktree.short, path], background: true)
        guard let data else { return .failure(.from(cli: message)) }
        guard let read = try? JSONDecoder().decode(FileRead.self, from: data) else { return .failure(.failed) }
        return .success(read)
    }
}

extension DaemonClient {
    /// One directory of the extra read-only folder `name`, `path` relative to
    /// it ("" is its root). `files folder-ls`.
    func listFiles(inFolder name: String, path: String) async -> Result<FileListing, FileReadFailure> {
        let (data, message) = await runRaw(["files", "folder-ls", "--json", "--", name, path], background: true)
        guard let data else { return .failure(.from(cli: message, inFolder: path.isEmpty)) }
        guard let listing = try? JSONDecoder().decode(FileListing.self, from: data) else { return .failure(.failed) }
        return .success(listing)
    }

    /// One file of the extra read-only folder `name`. `files folder-cat`.
    func readFile(inFolder name: String, path: String) async -> Result<FileRead, FileReadFailure> {
        let (data, message) = await runRaw(["files", "folder-cat", "--json", "--", name, path], background: true)
        guard let data else { return .failure(.from(cli: message, inFolder: false)) }
        guard let read = try? JSONDecoder().decode(FileRead.self, from: data) else { return .failure(.failed) }
        return .success(read)
    }
}
