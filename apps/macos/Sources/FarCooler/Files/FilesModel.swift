import Foundation
import SwiftUI

/// One worktree's Files tab: the tree read so far, the file open, and the
/// find and the selection in it (ov-189).
///
/// Read-only, and read only on demand: a directory when it's expanded, a
/// file when it's opened, a filter when it's typed. Nothing polls and
/// nothing runs on a timer, so a Files tab left open costs nothing while
/// nobody touches it (ov-229). Reload reads again what's shown.
@MainActor
final class FilesModel: ObservableObject {
    /// Where the reads go: the runner, or a test.
    struct Source {
        var list: @MainActor (String) async -> Result<FileListing, FileReadFailure>
        var read: @MainActor (String) async -> Result<FileRead, FileReadFailure>
        var search: @MainActor (String) async -> [String]
    }

    /// A directory's state in the tree.
    enum Folder: Equatable {
        case loading
        case failed(FileReadFailure)
    }

    /// The file on screen.
    struct Opened: Equatable {
        enum Content: Equatable {
            case loading
            case text([String])
            case binary(UInt64)
            case tooLarge(UInt64)
            /// A link: where it points, and that path inside the worktree
            /// when it stays inside.
            case link(target: String, inside: String?)
            case failed(FileReadFailure)
        }

        var path: String
        var content: Content
    }

    /// A line to bring into view, and a count so asking for the same line
    /// twice still scrolls.
    struct ScrollRequest: Equatable {
        var line: Int
        var serial: Int
    }

    let worktree: Worktree
    private let source: Source

    @Published private(set) var listings: [String: FileListing] = [:]
    @Published private(set) var folders: [String: Folder] = [:]
    @Published var expanded: Set<String> = []
    @Published private(set) var opened: Opened?
    /// The lines chosen in the gutter, 0-based, inclusive.
    @Published var selection: ClosedRange<Int>?
    /// Where a shift-click grows the selection from.
    var selectionAnchor: Int?
    @Published private(set) var scroll: ScrollRequest?

    /// The tree's filter, and the paths it found.
    @Published var filter = ""
    @Published private(set) var found: [String] = []

    /// The find bar.
    @Published var finding = false
    @Published var findQuery = ""
    @Published private(set) var matches: [FilesLogic.Match] = []
    @Published private(set) var currentMatch: Int?

    /// The go-to-line field.
    @Published var goingToLine = false

    /// The longest line of the file on screen, in characters.
    @Published private(set) var widest = 0

    /// The lines of the file on screen, or none.
    var lines: [String] {
        if case .text(let lines)? = opened?.content { return lines }
        return []
    }

    init(worktree: Worktree, source: Source) {
        self.worktree = worktree
        self.source = source
    }

    convenience init(worktree: Worktree, client: DaemonClient) {
        self.init(
            worktree: worktree,
            source: Source(
                list: { [weak client] path in
                    await client?.listFiles(in: worktree, path: path) ?? .failure(.failed)
                },
                read: { [weak client] path in
                    await client?.readFile(in: worktree, path: path) ?? .failure(.failed)
                },
                search: { [weak client] query in
                    await client?.searchFiles(in: worktree, query: query) ?? []
                }))
    }

    // MARK: - The tree

    /// The rows the tree draws.
    var rows: [FilesLogic.TreeRow] { FilesLogic.rows(listings: listings, expanded: expanded) }

    /// Read the root, once.
    func loadIfNeeded() async {
        guard listings[""] == nil, folders[""] == nil else { return }
        await load("")
    }

    /// Read one directory.
    func load(_ dir: String) async {
        folders[dir] = .loading
        switch await source.list(dir) {
        case .success(let listing):
            listings[dir] = listing
            folders[dir] = nil
        case .failure(let why):
            folders[dir] = .failed(why)
        }
    }

    /// Open or close a directory, reading it the first time.
    func toggle(_ dir: String) async {
        if expanded.contains(dir) {
            expanded.remove(dir)
            return
        }
        expanded.insert(dir)
        if listings[dir] == nil { await load(dir) }
    }

    /// Read again everything shown: the directories open and the file.
    func reload() async {
        let shown = [""] + expanded.sorted()
        listings = [:]
        for dir in shown { await load(dir) }
        if let path = opened?.path { await open(path, line: nil, reveal: false) }
    }

    /// Run the filter, on the runner (`worktree file-search`): it searches
    /// the whole worktree, not just what the tree has read.
    func runFilter() async {
        let query = filter.trimmingCharacters(in: .whitespaces)
        guard !query.isEmpty else {
            found = []
            return
        }
        let paths = await source.search(query)
        // Typed on while this was out: the newer query's answer wins.
        guard query == filter.trimmingCharacters(in: .whitespaces) else { return }
        found = paths
    }

    // MARK: - The file

    /// Open `path`, at `line` (1-based) when one is named, and show it in the
    /// tree.
    func open(_ path: String, line: Int?, reveal: Bool = true) async {
        let same = opened?.path == path
        if !same {
            opened = Opened(path: path, content: .loading)
            selection = nil
            selectionAnchor = nil
            matches = []
            currentMatch = nil
        }
        if reveal {
            for dir in FilesLogic.ancestors(of: path) {
                expanded.insert(dir)
                if listings[dir] == nil { await load(dir) }
            }
        }
        let answer = await source.read(path)
        // Another file was opened while this one was being read.
        guard opened?.path == path else { return }
        let content: Opened.Content
        switch answer {
        case .success(let read):
            switch read.state {
            case .text: content = .text(FilesLogic.lines(of: read.text))
            case .binary: content = .binary(read.size)
            case .tooLarge: content = .tooLarge(read.size)
            case .link:
                content = .link(
                    target: read.linkTarget,
                    inside: FilesLogic.linkDestination(path, target: read.linkTarget, root: worktree.path))
            case .unknown: content = .failed(.failed)
            }
        case .failure(let why):
            content = .failed(why)
        }
        opened = Opened(path: path, content: content)
        widest = FilesLogic.widest(lines)
        if !findQuery.isEmpty { refind() }
        if let line { go(toLine: line) }
    }

    /// Bring 1-based `line` into view and select it.
    func go(toLine line: Int) {
        guard !lines.isEmpty else { return }
        let index = min(max(line, 1), lines.count) - 1
        selection = index...index
        selectionAnchor = index
        scroll = ScrollRequest(line: index, serial: (scroll?.serial ?? 0) + 1)
    }

    /// A click in the gutter: choose that line, or with Shift grow the
    /// selection to it.
    func click(line: Int, extending: Bool) {
        if extending, let anchor = selectionAnchor {
            selection = FilesLogic.extend(from: anchor, to: line)
        } else {
            selection = line...line
            selectionAnchor = line
        }
    }

    /// What Copy copies: the lines chosen.
    var copiedText: String? {
        guard let selection, !lines.isEmpty else { return nil }
        return FilesLogic.copied(lines, range: selection)
    }

    // MARK: - Find

    /// Find `findQuery` again, in the file on screen.
    func refind() {
        matches = FilesLogic.matches(findQuery, in: lines)
        currentMatch = matches.isEmpty ? nil : 0
        if let first = matches.first { reveal(first) }
    }

    /// The next match (`forward`) or the previous one, wrapping.
    func step(forward: Bool) {
        guard !matches.isEmpty else { return }
        let at = currentMatch ?? -1
        let next = ((at + (forward ? 1 : -1)) % matches.count + matches.count) % matches.count
        currentMatch = next
        reveal(matches[next])
    }

    private func reveal(_ match: FilesLogic.Match) {
        scroll = ScrollRequest(line: match.line, serial: (scroll?.serial ?? 0) + 1)
    }
}
