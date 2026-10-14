import Foundation

// "Found on This Mac" (ov-519, decision 5): the Git repositories in the folders
// people keep projects in, newest commit first, so the first run is one click
// instead of a file browser.
//
// Local only: it reads directory names and one date per repository, and
// nothing leaves this Mac. The walk is pure over `SuggestionFileSystem`, so
// `RepositorySuggestionsTests` reads every rule with a fake disk, and the Mac
// supplies the real one.

/// One repository found on disk.
public struct RepositorySuggestion: Equatable, Identifiable, Sendable {
    /// Absolute path.
    public let path: String
    public let name: String
    /// The newest commit's date, nil for a repository with no commit yet.
    public let lastCommit: Date?

    public var id: String { path }

    public init(path: String, lastCommit: Date?) {
        self.path = path
        self.name = (path as NSString).lastPathComponent
        self.lastCommit = lastCommit
    }

    /// "~/Dev/harbor": the path as a person writes it.
    public func displayPath(home: String) -> String {
        guard path == home || path.hasPrefix(home + "/") else { return path }
        return "~" + path.dropFirst(home.count)
    }

    /// "2 days ago", or "No commits yet". English, whatever the Mac's locale,
    /// like the rest of the first run's words.
    public func when(now: Date) -> String {
        guard let lastCommit else { return "No commits yet" }
        // Under a minute reads as "in 0 seconds" from the formatter, and a
        // clock that is a little behind puts a commit in the future.
        if now.timeIntervalSince(lastCommit) < 60 { return "Just now" }
        let formatter = RelativeDateTimeFormatter()
        formatter.locale = Locale(identifier: "en_US")
        formatter.unitsStyle = .full
        return formatter.localizedString(for: lastCommit, relativeTo: now)
    }
}

/// What the walk needs of a disk.
public protocol SuggestionFileSystem: Sendable {
    /// The names of `path`'s folders, any order. Empty when it can't be read.
    func subfolders(of path: String) -> [String]
    /// Whether `path` is a repository's own folder: it holds a `.git` folder.
    /// A linked worktree, whose `.git` is a file, is not one: it belongs to a
    /// repository that's listed on its own.
    func isRepository(_ path: String) -> Bool
    /// Whether `path` is itself a link, which is not followed: a link inside
    /// a root can lead anywhere, to Documents (a privacy prompt) or to a volume
    /// that isn't mounted (a stall). Only a root may be a link, as `~/Code`
    /// pointing at `~/Dev` is.
    func isLink(_ path: String) -> Bool
    /// When the repository was last touched, from one cheap look at its own
    /// log of where HEAD has been, never a process: what the walk ranks by
    /// to choose which few it spends a `lastCommit` on.
    func touched(_ path: String) -> Date?
    /// `path` with links resolved, so one repository reached by two roots
    /// (`~/Code` as a link to `~/Dev`) is listed once.
    func resolved(_ path: String) -> String
    /// The newest commit's date, nil with none or when it can't be read.
    func lastCommit(of path: String) async -> Date?
}

public enum RepositorySuggestions {
    /// Folders under the home directory where projects usually live, and
    /// which a first run can read without a privacy prompt (Documents,
    /// Desktop and Downloads would ask).
    public static let roots = ["Dev", "Code", "Projects", "src", "Developer"]
    /// Folders deep: `~/Dev/harbor` is one, `~/Dev/work/harbor` is two.
    public static let depth = 2
    /// How many to offer.
    public static let limit = 5
    /// How many of the most recently touched get their commit date asked for,
    /// side by side. Every repository found used to be dated, one process
    /// after another, so a `~/src` of a thousand clones took minutes; this
    /// is the most that is ever asked, whatever is on disk.
    public static let dated = 10
    /// Folders looked into before the walk stops, so a home with a huge
    /// `src` tree opens the Welcome page as fast as an empty one.
    public static let visitCap = 2_000
    /// Folders that hold other people's code, never somebody's project.
    static let skipped: Set<String> = ["node_modules", "vendor", "target", "build", "dist", "Pods", ".build"]

    /// The newest `limit` repositories under `home`'s usual folders.
    ///
    /// Bounded in time as well as in count: the walk reads at most `visitCap`
    /// folders and never follows a link below a root, and only the `dated`
    /// most recently touched repositories are asked for a commit date.
    public static func scan(home: String, fileSystem: some SuggestionFileSystem) async -> [RepositorySuggestion] {
        var found: [String] = []
        var seen: Set<String> = []
        var visited = 0

        func walk(_ folder: String, level: Int) {
            for name in fileSystem.subfolders(of: folder).sorted() {
                guard visited < visitCap else { return }
                // Hidden folders and the ones that only hold dependencies.
                guard !name.hasPrefix("."), !skipped.contains(name) else { continue }
                visited += 1
                let path = folder + "/" + name
                // A link below a root is somebody's shortcut, not a project here.
                guard !fileSystem.isLink(path) else { continue }
                if fileSystem.isRepository(path) {
                    // A repository's inside is its own: nothing nested is a
                    // second project.
                    if seen.insert(fileSystem.resolved(path)).inserted { found.append(path) }
                } else if level < depth {
                    walk(path, level: level + 1)
                }
            }
        }
        for root in roots { walk(home + "/" + root, level: 1) }

        // The most recently touched, by name between equals, then dated.
        let finalists = found
            .map { ($0, fileSystem.touched($0)) }
            .sorted { a, b in
                switch (a.1, b.1) {
                case (let x?, let y?) where x != y: return x > y
                case (nil, _?): return false
                case (_?, nil): return true
                default: return a.0 < b.0
                }
            }
            .prefix(dated).map(\.0)
        let dates = await withTaskGroup(of: (String, Date?).self) { group in
            for path in finalists { group.addTask { (path, await fileSystem.lastCommit(of: path)) } }
            var out: [String: Date?] = [:]
            for await (path, date) in group { out[path] = date }
            return out
        }
        let suggestions = finalists.map { RepositorySuggestion(path: $0, lastCommit: dates[$0] ?? nil) }
        return Array(ranked(suggestions).prefix(limit))
    }

    /// Newest commit first; one with none after the rest; a name, then a
    /// path, between equals so the order never flickers.
    public static func ranked(_ list: [RepositorySuggestion]) -> [RepositorySuggestion] {
        list.sorted { a, b in
            switch (a.lastCommit, b.lastCommit) {
            case (let x?, let y?) where x != y: return x > y
            case (nil, _?): return false
            case (_?, nil): return true
            default: return (a.name, a.path) < (b.name, b.path)
            }
        }
    }
}
