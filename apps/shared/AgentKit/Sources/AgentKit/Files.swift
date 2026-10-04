import Foundation

// A worktree's files, and a runner's extra read-only folders, as the Mac's
// Files tab and both phones read them (ov-189, ov-232, ov-259).
//
// One copy of the wire shapes, the path rules and the sentences, because three
// screens that disagree about where a link leads, or what "too large" says, are
// the same file shown three ways. Android keeps a Kotlin port in
// `model/Files.kt`, pinned to this one by `test/fixtures/files-lines.json`.

/// One name in a directory, as `farcooler files ls --json` and the client
/// core's `worktree.list_dir` say it (`farcooler_client::files_json::dir_json`).
public struct FileEntry: Decodable, Hashable, Sendable {
    public enum Kind: String, Decodable, Sendable {
        case file, directory, link, other

        /// A word a newer runner sends that this build hasn't heard of is
        /// listed, never opened.
        public init(from decoder: Decoder) throws {
            let word = try decoder.singleValueContainer().decode(String.self)
            self = Kind(rawValue: word) ?? .other
        }
    }

    public var name: String
    public var kind: Kind
    public var size: UInt64
    public var linkTarget: String

    public init(name: String, kind: Kind, size: UInt64, linkTarget: String) {
        self.name = name
        self.kind = kind
        self.size = size
        self.linkTarget = linkTarget
    }
}

/// One directory's answer.
public struct FileListing: Decodable, Equatable, Sendable {
    public var path: String
    public var entries: [FileEntry]
    /// The runner sends at most 5,000 entries for one directory.
    public var truncated: Bool

    public init(path: String, entries: [FileEntry], truncated: Bool) {
        self.path = path
        self.entries = entries
        self.truncated = truncated
    }
}

/// One file's answer (`farcooler_client::files_json::file_json`).
public struct FileRead: Decodable, Equatable, Sendable {
    public enum State: String, Decodable, Sendable {
        case text, binary, tooLarge = "too_large", link, unknown

        public init(from decoder: Decoder) throws {
            let word = try decoder.singleValueContainer().decode(String.self)
            self = State(rawValue: word) ?? .unknown
        }
    }

    public var path: String
    public var state: State
    public var size: UInt64
    public var text: String
    public var linkTarget: String

    public init(path: String, state: State, size: UInt64, text: String, linkTarget: String) {
        self.path = path
        self.state = state
        self.size = size
        self.text = text
        self.linkTarget = linkTarget
    }
}

/// Why a read came back with nothing, in a sentence for the screen.
public enum FileReadFailure: Error, Equatable, Sendable {
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

    /// For a file.
    public var sentence: String {
        switch self {
        case .runnerTooOld: return "Update Far Cooler on this runner to see its files."
        case .missing: return "This file isn’t in the worktree anymore."
        case .missingInFolder: return "This isn’t in the folder anymore."
        case .folderGone: return "This runner doesn’t share this folder anymore."
        case .notAFile: return "This isn’t a file Far Cooler can show."
        case .failed: return "Couldn’t read this file. Check that the runner is reachable, then try again."
        }
    }

    /// For a directory.
    public var directorySentence: String {
        switch self {
        case .missing: return "This folder isn’t in the worktree anymore."
        case .notAFile: return "This isn’t a folder Far Cooler can show."
        case .failed: return "Couldn’t read this folder. Check that the runner is reachable, then try again."
        case .runnerTooOld, .missingInFolder, .folderGone: return sentence
        }
    }

    /// From the client core's refusal word (`RunnerRefusal`'s kebab word, nil
    /// when the link itself failed). In an extra folder the runner answers a
    /// name it no longer shares, and a path it refuses, as "not found", so
    /// nothing at the folder's root means the folder is gone.
    public static func from(refusal word: String?, inFolder: Bool, atRoot: Bool) -> FileReadFailure {
        switch word {
        case "not-found":
            guard inFolder else { return .missing }
            return atRoot ? .folderGone : .missingInFolder
        case "invalid-argument": return .notAFile
        case "capability-unsupported": return .runnerTooOld
        default: return .failed
        }
    }
}

/// The path rules, as values. Pure, so the Mac's, iOS's and the tests' answers
/// are one.
public enum FilesPaths {
    /// `parent`'s child named `name`, as a path from the root.
    public static func join(_ parent: String, _ name: String) -> String {
        parent.isEmpty ? name : "\(parent)/\(name)"
    }

    /// `location`, a path an agent's tool call named, as a path inside the
    /// worktree at `root`, or nil when it's somewhere else.
    ///
    /// Agents name absolute paths (ACP's `locations`). Lexical only, and the
    /// runner walks it again without following links, so a path that only
    /// looks inside is still refused there.
    public static func relative(_ location: String, in root: String) -> String? {
        let root = trimmedRoot(root)
        guard !root.isEmpty else { return nil }
        let path: String
        if location.hasPrefix("/") {
            let candidates = [root, "/private" + root].filter { location.hasPrefix($0 + "/") }
            guard let match = candidates.first else { return nil }
            path = String(location.dropFirst(match.count + 1))
        } else {
            path = location
        }
        return normalized(path)
    }

    /// Where a link at `path` leads, as a path inside the worktree at `root`,
    /// or nil when it leaves the worktree. A phone has no `root`: the runner
    /// never sends it one, so an absolute target leads nowhere there.
    public static func linkDestination(_ path: String, target: String, root: String) -> String? {
        if target.hasPrefix("/") { return relative(target, in: root) }
        let parent = (path as NSString).deletingLastPathComponent
        return normalized(join(parent, target))
    }

    /// `path` with `.` and `..` worked out, or nil when `..` climbs above the
    /// root or nothing is left.
    public static func normalized(_ path: String) -> String? {
        var parts: [Substring] = []
        for part in path.split(separator: "/", omittingEmptySubsequences: true) {
            switch part {
            case ".": continue
            case "..":
                guard !parts.isEmpty else { return nil }
                parts.removeLast()
            default: parts.append(part)
            }
        }
        return parts.isEmpty ? nil : parts.joined(separator: "/")
    }

    private static func trimmedRoot(_ root: String) -> String {
        var root = root
        while root.count > 1 && root.hasSuffix("/") { root.removeLast() }
        return root
    }
}

/// A file's text, as a viewer numbers and draws it.
public enum FilesText {
    /// A file's lines, as the viewer numbers them: broken at `\n`, `\r\n` and
    /// a bare `\r` (an old Mac file), none of them shown, and no phantom empty
    /// line after a final break.
    ///
    /// By scalar, not by `Character`: Swift reads `\r\n` as ONE character, so
    /// splitting on `"\n"` never split a Windows file at all, and it drew as
    /// one line.
    public static func lines(of text: String) -> [String] {
        var lines: [String] = []
        var current = String.UnicodeScalarView()
        var afterReturn = false
        for scalar in text.unicodeScalars {
            if afterReturn {
                afterReturn = false
                if scalar == "\n" { continue }
            }
            if scalar == "\n" || scalar == "\r" {
                lines.append(String(current))
                current = String.UnicodeScalarView()
                afterReturn = scalar == "\r"
            } else {
                current.append(scalar)
            }
        }
        if !current.isEmpty { lines.append(String(current)) }
        return lines
    }

    /// The most a phone draws of one line, in UTF-16 code units, the unit both
    /// platforms' strings count in. The runner caps a file's bytes and not a
    /// line's length, so a minified 512 KiB file is one line, and one
    /// half-megabyte `Text` stalls a phone (ov-259).
    public static let lineLimit = 2_000

    /// `line` as drawn: whole, or cut at `limit` code units, never in the
    /// middle of a surrogate pair, with a closing "…".
    public static func display(_ line: String, limit: Int = lineLimit) -> (text: String, cut: Bool) {
        let units = line.utf16
        guard let end = units.index(units.startIndex, offsetBy: limit, limitedBy: units.endIndex),
            end != units.endIndex
        else { return (line, false) }
        var keep = end
        if UTF16.isTrailSurrogate(units[keep]) { keep = units.index(before: keep) }
        return (String(decoding: Array(units[..<keep]), as: UTF16.self) + "…", true)
    }

    /// A file's size the way Finder says it.
    public static func size(_ bytes: UInt64) -> String {
        ByteCountFormatter.string(fromByteCount: Int64(clamping: bytes), countStyle: .file)
    }
}
