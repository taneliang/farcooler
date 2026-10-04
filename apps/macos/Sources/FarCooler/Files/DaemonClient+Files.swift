import AgentKit
import Foundation

/// One name in a worktree directory, as `farcooler files ls --json` says it
/// (`farcooler_client::files_json::dir_json`).
struct FileEntry: Decodable, Hashable {
    enum Kind: String, Decodable {
        case file, directory, link, other

        /// A word a newer runner sends that this build hasn't heard of is
        /// listed, never opened.
        init(from decoder: Decoder) throws {
            let word = try decoder.singleValueContainer().decode(String.self)
            self = Kind(rawValue: word) ?? .other
        }
    }

    var name: String
    var kind: Kind
    var size: UInt64
    var linkTarget: String
}

/// One directory's answer.
struct FileListing: Decodable, Equatable {
    var path: String
    var entries: [FileEntry]
    /// The runner sends at most 5,000 entries for one directory.
    var truncated: Bool
}

/// One file's answer (`farcooler_client::files_json::file_json`).
struct FileRead: Decodable, Equatable {
    enum State: String, Decodable {
        case text, binary, tooLarge = "too_large", link, unknown

        init(from decoder: Decoder) throws {
            let word = try decoder.singleValueContainer().decode(String.self)
            self = State(rawValue: word) ?? .unknown
        }
    }

    var path: String
    var state: State
    var size: UInt64
    var text: String
    var linkTarget: String
}

/// Why a read came back with nothing, in a sentence for the pane.
enum FileReadFailure: Error, Equatable {
    /// The runner predates `worktree_files`.
    case runnerTooOld
    /// Nothing at that path, or a link on the way to it.
    case missing
    /// A directory, a FIFO, a socket.
    case notAFile
    /// Nothing at that path in an extra folder.
    case missingInFolder
    /// The runner won't show this extra folder at all anymore: it was taken
    /// out of its config, or the runner now refuses it.
    case folderGone
    /// Anything else: the runner couldn't be reached, or said something new.
    case failed

    var sentence: String {
        switch self {
        case .runnerTooOld: return "Update Far Cooler on this runner to see its files."
        case .missing: return "This file isn’t in the worktree anymore."
        case .missingInFolder: return "This isn’t in the folder anymore."
        case .folderGone: return "This runner doesn’t share this folder anymore."
        case .notAFile: return "This isn’t a file Far Cooler can show."
        case .failed: return "Couldn’t read this file. Check that the runner is reachable, then try again."
        }
    }

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

    /// One directory of `worktree`, `path` relative to its root ("" is the
    /// root). `files ls`.
    func listFiles(in worktree: Worktree, path: String) async -> Result<FileListing, FileReadFailure> {
        let (data, message) = await runRaw(["files", "ls", worktree.short, path, "--json"], background: true)
        guard let data else { return .failure(.from(cli: message)) }
        guard let listing = try? JSONDecoder().decode(FileListing.self, from: data) else { return .failure(.failed) }
        return .success(listing)
    }

    /// One file of `worktree`, whole, up to the runner's 512 KiB. `files cat`.
    func readFile(in worktree: Worktree, path: String) async -> Result<FileRead, FileReadFailure> {
        let (data, message) = await runRaw(["files", "cat", worktree.short, path, "--json"], background: true)
        guard let data else { return .failure(.from(cli: message)) }
        guard let read = try? JSONDecoder().decode(FileRead.self, from: data) else { return .failure(.failed) }
        return .success(read)
    }
}

extension DaemonClient {
    /// One directory of the extra read-only folder `name`, `path` relative to
    /// it ("" is its root). `files folder-ls`.
    func listFiles(inFolder name: String, path: String) async -> Result<FileListing, FileReadFailure> {
        let (data, message) = await runRaw(["files", "folder-ls", name, path, "--json"], background: true)
        guard let data else { return .failure(.from(cli: message, inFolder: path.isEmpty)) }
        guard let listing = try? JSONDecoder().decode(FileListing.self, from: data) else { return .failure(.failed) }
        return .success(listing)
    }

    /// One file of the extra read-only folder `name`. `files folder-cat`.
    func readFile(inFolder name: String, path: String) async -> Result<FileRead, FileReadFailure> {
        let (data, message) = await runRaw(["files", "folder-cat", name, path, "--json"], background: true)
        guard let data else { return .failure(.from(cli: message, inFolder: false)) }
        guard let read = try? JSONDecoder().decode(FileRead.self, from: data) else { return .failure(.failed) }
        return .success(read)
    }
}
