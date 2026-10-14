import AgentKit
import Foundation

/// This Mac's disk, for "Found on This Mac" (ov-519): folder names, a `.git`
/// folder, and one date per repository. Nothing is read from a file's
/// contents, and nothing leaves this Mac.
struct LocalSuggestionFileSystem: SuggestionFileSystem {
    func subfolders(of path: String) -> [String] {
        let names = (try? FileManager.default.contentsOfDirectory(atPath: path)) ?? []
        return names.filter { name in
            // By what the entry is, not what it points at: a link is listed (and
            // then skipped by the walk) without ever being opened, so one to a
            // volume that isn't there can't stall the page.
            let type = (try? FileManager.default.attributesOfItem(atPath: path + "/" + name))?[.type] as? FileAttributeType
            return type == .typeDirectory || type == .typeSymbolicLink
        }
    }

    func isLink(_ path: String) -> Bool {
        (try? FileManager.default.attributesOfItem(atPath: path))?[.type] as? FileAttributeType == .typeSymbolicLink
    }

    /// The mtime of the log of where HEAD has been, else of HEAD itself: one
    /// stat, no process.
    func touched(_ path: String) -> Date? {
        for file in ["/.git/logs/HEAD", "/.git/HEAD"] {
            if let date = (try? FileManager.default.attributesOfItem(atPath: path + file))?[.modificationDate] as? Date {
                return date
            }
        }
        return nil
    }

    func isRepository(_ path: String) -> Bool {
        var isFolder: ObjCBool = false
        return FileManager.default.fileExists(atPath: path + "/.git", isDirectory: &isFolder) && isFolder.boolValue
    }

    func resolved(_ path: String) -> String {
        URL(fileURLWithPath: path).resolvingSymlinksInPath().path
    }

    func lastCommit(of path: String) async -> Date? {
        guard let git = Self.git else { return Self.reflogDate(path) }
        let ran = await ProcessRunner.run(
            git, ["-C", path, "log", "-1", "--format=%ct"],
            environment: Self.environment, deadline: 2, discardStderr: true)
        let said = String(decoding: ran.stdout, as: UTF8.self).trimmingCharacters(in: .whitespacesAndNewlines)
        guard ran.succeeded, let seconds = TimeInterval(said) else { return Self.reflogDate(path) }
        return Date(timeIntervalSince1970: seconds)
    }

    /// The git that's really installed: never `/usr/bin/git`, which on a Mac
    /// without the developer tools is a stub that opens an install dialog the
    /// first time anything runs it.
    static let git: String? = [
        "/opt/homebrew/bin/git", "/usr/local/bin/git",
        "/Library/Developer/CommandLineTools/usr/bin/git",
        "/Applications/Xcode.app/Contents/Developer/usr/bin/git",
    ].first { FileManager.default.isExecutableFile(atPath: $0) }

    /// Without git, or when it can't answer: the last line of the repository's
    /// own log of where HEAD has been, which holds a Unix time in seconds.
    static func reflogDate(_ path: String) -> Date? {
        guard let text = try? String(contentsOfFile: path + "/.git/logs/HEAD", encoding: .utf8),
            let last = text.split(separator: "\n").last
        else { return nil }
        // "<old> <new> Name <email> 1700000000 -0700\tcommit: message"
        let head = last.split(separator: "\t", maxSplits: 1).first ?? last
        let fields = head.split(separator: " ")
        guard fields.count >= 2, let seconds = TimeInterval(fields[fields.count - 2]) else { return nil }
        return Date(timeIntervalSince1970: seconds)
    }

    /// Git asked not to take the repository's lock, so scanning a repository
    /// somebody is working in never makes their own `git status` wait.
    static let environment = ["GIT_OPTIONAL_LOCKS": "0", "PATH": "/usr/bin:/bin"]
}

/// What the Welcome page knows about the repositories on this Mac.
@MainActor
final class WelcomeModel: ObservableObject {
    @Published private(set) var suggestions: [RepositorySuggestion] = []
    @Published private(set) var scanned = false
    /// The suggestion chosen, by path; the newest to begin with.
    @Published var selected: String?
    /// The path being added, while it is.
    @Published private(set) var adding: String?
    @Published private(set) var failure: SheetFailure?

    let home: String
    private let scan: () async -> [RepositorySuggestion]

    init(
        home: String = NSHomeDirectory(),
        scan: (() async -> [RepositorySuggestion])? = nil
    ) {
        self.home = home
        self.scan =
            scan ?? {
                await RepositorySuggestions.scan(home: home, fileSystem: LocalSuggestionFileSystem())
            }
    }

    /// Look once, off the main thread's way, and choose the newest.
    func look() async {
        guard !scanned else { return }
        suggestions = await scan()
        if selected == nil { selected = suggestions.first?.path }
        scanned = true
    }

    /// Add `path` with `register`, which answers nil or the words it failed with.
    func use(_ path: String, register: (String) async -> String?) async {
        guard adding == nil else { return }
        adding = path
        failure = nil
        defer { adding = nil }
        if let words = await register(path) {
            failure = SheetFailure(sentence: FirstRunCopy.Welcome.couldNotAdd, transcript: words)
        }
    }

    /// A folder somebody chose by hand: a repository, or a sentence saying
    /// it isn't one.
    func choose(_ folder: URL, register: (String) async -> String?) async {
        guard LocalSuggestionFileSystem().isRepository(folder.path) else {
            failure = SheetFailure(sentence: FirstRunCopy.Welcome.notARepository)
            return
        }
        await use(folder.path, register: register)
    }

    /// The suggestion chosen.
    var chosen: RepositorySuggestion? { suggestions.first { $0.path == selected } }
}
