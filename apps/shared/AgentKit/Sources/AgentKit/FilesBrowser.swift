import Foundation

// The phones' read-only Files browser, minus its views (ov-259): where you are,
// how a runner is asked, and what each screen shows, as values a test reads.
//
// One directory or one file is one screen, and a tap pushes the next, the way
// the platforms' own Files apps work. So there is no tree to keep open, and a
// saved stack restores by its places alone.

/// Where a path is read from: a worktree, or one of the runner's extra
/// read-only folders by the name `host` gives it (ov-232).
public enum FilesPlace: Hashable, Codable, Sendable {
    case worktree(String)
    case folder(String)
}

/// One screen's place and path, and what to expect there.
public struct FilesLocation: Hashable, Codable, Sendable {
    /// What the path names, when the caller knows. A link's destination could
    /// be either, and is read as a file first.
    public enum Expecting: String, Hashable, Codable, Sendable {
        case directory, file, either
    }

    public var place: FilesPlace
    /// From the place's root; empty is the root.
    public var path: String
    public var expecting: Expecting

    public init(place: FilesPlace, path: String = "", expecting: Expecting = .directory) {
        self.place = place
        self.path = path
        self.expecting = expecting
    }

    /// The navigation bar's title: the last name, or the place at its root.
    public var title: String {
        if let last = path.split(separator: "/").last { return String(last) }
        switch place {
        case .worktree: return "Files"
        case .folder(let name): return name
        }
    }

    var inFolder: Bool {
        if case .folder = place { return true }
        return false
    }
}

/// How the runner is asked, as a screen sees it: a closure each, so a test
/// answers without a runner. The apps fill these from their client core.
public struct FilesSource: Sendable {
    public var list: @Sendable (FilesPlace, String) async -> Result<FileListing, FileReadFailure>
    public var read: @Sendable (FilesPlace, String) async -> Result<FileRead, FileReadFailure>

    public init(
        list: @escaping @Sendable (FilesPlace, String) async -> Result<FileListing, FileReadFailure>,
        read: @escaping @Sendable (FilesPlace, String) async -> Result<FileRead, FileReadFailure>
    ) {
        self.list = list
        self.read = read
    }
}

/// The client core's `worktree.list_dir` and `worktree.read_file`, as an app
/// passes and reads them.
public enum FilesWire {
    public static let listMethod = "worktree.list_dir"
    public static let readMethod = "worktree.read_file"

    /// The arguments: a worktree or a folder, never both.
    public static func arguments(_ place: FilesPlace, path: String) -> [String: Any] {
        switch place {
        case .worktree(let id): return ["worktree": id, "path": path]
        case .folder(let name): return ["folder": name, "path": path]
        }
    }

    /// What `source` reads for a core that answers a call with JSON bytes, or
    /// throws a refusal `word` (nil when the link failed).
    public static func source(
        call: @escaping @Sendable (String, [String: Any]) async throws -> Data,
        refusalWord: @escaping @Sendable (Error) -> String?
    ) -> FilesSource {
        @Sendable func fetch<T: Decodable & Sendable>(
            _ method: String, _ type: T.Type, _ place: FilesPlace, _ path: String
        ) async -> Result<T, FileReadFailure> {
            let folder = place.isFolder
            do {
                let data = try await call(method, arguments(place, path: path))
                guard let value = try? JSONDecoder().decode(T.self, from: data) else { return .failure(.failed) }
                return .success(value)
            } catch {
                return .failure(
                    .from(refusal: refusalWord(error), inFolder: folder, atRoot: path.isEmpty))
            }
        }
        return FilesSource(
            list: { place, path in await fetch(listMethod, FileListing.self, place, path) },
            read: { place, path in await fetch(readMethod, FileRead.self, place, path) })
    }
}

extension FilesPlace {
    var isFolder: Bool {
        if case .folder = self { return true }
        return false
    }
}

/// One row of a directory.
public struct FilesRow: Identifiable, Equatable, Sendable {
    /// Unique within its directory: the runner sends names through a lossy
    /// UTF-8 decode, so two names on disk can arrive as one string, and a list
    /// keyed by name drops or misdraws one of them.
    public var id: String
    public var name: String
    public var kind: FileEntry.Kind
    /// A file's size, a link's "→ target", nothing for a directory.
    public var detail: String
    /// Where a tap goes: nil for a link out of the place, and for what Far
    /// Cooler can't show.
    public var destination: FilesLocation?
}

/// A directory as a screen draws it.
public struct FilesDirectory: Equatable, Sendable {
    public var rows: [FilesRow]
    /// "Showing the first 5,000 items.", when the runner cut the list.
    public var footer: String?
    /// "This folder is empty.", when there is nothing to list.
    public var empty: String?
}

/// A file's text as a viewer draws it.
public struct FilesCode: Equatable, Sendable {
    /// Each line, cut at `FilesText.lineLimit`.
    public var lines: [String]
    public var anyCut: Bool
    /// The widest line, in characters, for a horizontal scroller whose width
    /// is stated rather than measured: measuring makes a lazy stack build
    /// every row.
    public var widest: Int
    /// The gutter's width in digits.
    public var gutterDigits: Int

    public static let cutNote = "Long lines are cut at 2,000 characters."
}

/// What a screen shows.
public enum FilesContent: Equatable, Sendable {
    case loading
    case directory(FilesDirectory)
    case code(FilesCode)
    /// A file Far Cooler doesn't draw: binary, or past the runner's limit.
    case message(String)
    /// A link: where it points, and the place it leads when that's inside.
    case link(target: String, destination: FilesLocation?)
    case failed(String)
}

/// One screen's model: reads once, on demand, and never again until asked.
/// Nothing polls (ov-229).
@MainActor
public final class FilesItemModel: ObservableObject {
    public let location: FilesLocation
    @Published public private(set) var content: FilesContent = .loading
    private let source: FilesSource
    private let root: String

    /// `root` is the worktree's path on the runner when the app knows it. A
    /// phone doesn't, so a link with an absolute target leads nowhere there.
    public init(location: FilesLocation, source: FilesSource, root: String = "") {
        self.location = location
        self.source = source
        self.root = root
    }

    public func load() async {
        content = .loading
        switch location.expecting {
        case .directory:
            content = await directory(fallback: nil)
        case .file:
            content = await file(thenDirectory: false)
        case .either:
            content = await file(thenDirectory: true)
        }
    }

    private func directory(fallback: FileReadFailure?) async -> FilesContent {
        switch await source.list(location.place, location.path) {
        case .success(let listing): return .directory(Self.directory(listing, at: location, root: root))
        case .failure(let why): return .failed((fallback ?? why).directorySentence)
        }
    }

    private func file(thenDirectory: Bool) async -> FilesContent {
        switch await source.read(location.place, location.path) {
        case .success(let read): return Self.file(read, at: location, root: root)
        case .failure(.notAFile) where thenDirectory: return await directory(fallback: .notAFile)
        case .failure(let why): return .failed(why.sentence)
        }
    }

    nonisolated static func directory(_ listing: FileListing, at here: FilesLocation, root: String) -> FilesDirectory {
        let rows = listing.entries.enumerated().map { index, entry -> FilesRow in
            let id = "\(index)/\(entry.name)"
            let path = FilesPaths.join(here.path, entry.name)
            switch entry.kind {
            case .directory:
                return FilesRow(
                    id: id, name: entry.name, kind: .directory, detail: "",
                    destination: FilesLocation(place: here.place, path: path, expecting: .directory))
            case .file:
                return FilesRow(
                    id: id, name: entry.name, kind: .file, detail: FilesText.size(entry.size),
                    destination: FilesLocation(place: here.place, path: path, expecting: .file))
            case .link:
                let inside = FilesPaths.linkDestination(path, target: entry.linkTarget, root: root)
                return FilesRow(
                    id: id, name: entry.name, kind: .link, detail: "→ \(entry.linkTarget)",
                    destination: inside.map { FilesLocation(place: here.place, path: $0, expecting: .either) })
            case .other:
                return FilesRow(id: id, name: entry.name, kind: .other, detail: "", destination: nil)
            }
        }
        return FilesDirectory(
            rows: rows,
            footer: listing.truncated ? "Showing the first \(listing.entries.count.formatted()) items." : nil,
            empty: rows.isEmpty ? "This folder is empty." : nil)
    }

    nonisolated static func file(_ read: FileRead, at here: FilesLocation, root: String) -> FilesContent {
        switch read.state {
        case .text:
            let shown = FilesText.lines(of: read.text).map { FilesText.display($0) }
            let lines = shown.map(\.text)
            return .code(
                FilesCode(
                    lines: lines, anyCut: shown.contains { $0.cut },
                    widest: lines.reduce(0) { max($0, $1.count) },
                    gutterDigits: String(max(lines.count, 1)).count))
        case .binary:
            return .message("This is a binary file. It’s \(FilesText.size(read.size)).")
        case .tooLarge:
            return .message("This file is too large to show here. It’s \(FilesText.size(read.size)).")
        case .link:
            let inside = FilesPaths.linkDestination(here.path, target: read.linkTarget, root: root)
            return .link(
                target: read.linkTarget,
                destination: inside.map { FilesLocation(place: here.place, path: $0, expecting: .either) })
        case .unknown:
            return .message("Far Cooler can’t show this file yet. Update the app to see it.")
        }
    }
}

extension DaemonBuild {
    /// Whether a phone offers Files for this runner's worktrees: the runner
    /// serves them (`worktree_files`) and this connection may read them. The
    /// runner refuses a read grant, so the door isn't offered rather than
    /// shown to fail. Hidden, not dimmed: there is nothing to do about it
    /// from here.
    public var offersFiles: Bool { can(.worktreeFiles) && mayAct }

    /// The runner's extra read-only folders, by name, when there are any to
    /// offer: it names `read_only_folders` and `offersFiles` holds.
    public var sharedFolders: [String] {
        guard offersFiles, can(.readOnlyFolders) else { return [] }
        return readOnlyFolders ?? []
    }
}

extension DaemonBuild {
    /// A build from the client core's `host` answer, as the phone parses it.
    ///
    /// Here and not in `Connection` so a test can feed it the wire's own JSON:
    /// a key left out of this list is a fact a phone never learns, and the
    /// harness's stand-in builds skip it (`readOnlyFolders` was left out once,
    /// so a real device never showed a shared folder). Absent keys read as
    /// they do from an older runner: no capabilities (`can(_:)` reads that as
    /// the features that existed then), `unspecified` scope (no answer, never
    /// no permission), no agents list (every harness offered), no folders.
    public init(host body: [String: Any]) {
        self.init(
            version: body["daemonVersion"] as? String ?? "unknown",
            matches: body["buildsMatch"] as? Bool ?? true,
            platform: body["platform"] as? String ?? "",
            capabilities: Set(body["capabilities"] as? [String] ?? []),
            grantedScope: body["grantedScope"] as? String ?? "unspecified",
            runnerId: body["runnerId"] as? String,
            pushPaired: body["pushPaired"] as? Bool ?? false,
            agentsFound: body["agentsFound"] as? [String],
            readOnlyFolders: body["readOnlyFolders"] as? [String])
    }
}
