import Foundation

/// The Files tab's rules, as values (ov-189): which rows the tree draws, what
/// a path an agent wrote means here, where a link leads, what a find
/// matches, and what Copy puts on the pasteboard.
///
/// Pure, so `FilesLogicTests` pins them without a runner or a view.
enum FilesLogic {
    // MARK: - The tree

    /// One row of the tree as drawn: a name at a depth.
    struct TreeRow: Identifiable, Equatable {
        /// The path from the worktree's root, which is also the row's id.
        var id: String
        var name: String
        var depth: Int
        var kind: FileEntry.Kind
        var expanded: Bool
        var linkTarget: String
    }

    /// `parent`'s child named `name`, as a path from the root.
    static func join(_ parent: String, _ name: String) -> String {
        parent.isEmpty ? name : "\(parent)/\(name)"
    }

    /// The rows a tree draws: each directory read so far, its children under
    /// it when it's expanded, depth first. A directory not read yet draws as
    /// closed whatever `expanded` says, because there's nothing under it to
    /// draw.
    static func rows(listings: [String: FileListing], expanded: Set<String>) -> [TreeRow] {
        var out: [TreeRow] = []
        func walk(_ dir: String, depth: Int) {
            guard let listing = listings[dir] else { return }
            for entry in listing.entries {
                let path = join(dir, entry.name)
                let open = entry.kind == .directory && expanded.contains(path) && listings[path] != nil
                out.append(
                    TreeRow(
                        id: path, name: entry.name, depth: depth, kind: entry.kind,
                        expanded: entry.kind == .directory && expanded.contains(path),
                        linkTarget: entry.linkTarget))
                if open { walk(path, depth: depth + 1) }
            }
        }
        walk("", depth: 0)
        return out
    }

    /// Every directory above `path`, root first: what to expand to show it.
    static func ancestors(of path: String) -> [String] {
        let parts = path.split(separator: "/").map(String.init)
        guard parts.count > 1 else { return [] }
        return (1..<parts.count).map { parts.prefix($0).joined(separator: "/") }
    }

    // MARK: - Paths from elsewhere

    /// `location`, a path an agent's tool call named, as a path inside the
    /// worktree at `root`, or nil when it's somewhere else.
    ///
    /// Agents name absolute paths (ACP's `locations`). Lexical only, and the
    /// runner walks it again without following links, so a path that only
    /// looks inside is still refused there.
    static func relative(_ location: String, in root: String) -> String? {
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
    /// or nil when it leaves the worktree.
    static func linkDestination(_ path: String, target: String, root: String) -> String? {
        if target.hasPrefix("/") { return relative(target, in: root) }
        let parent = (path as NSString).deletingLastPathComponent
        return normalized(join(parent, target))
    }

    /// `path` with `.` and `..` worked out, or nil when `..` climbs above the
    /// root or nothing is left.
    static func normalized(_ path: String) -> String? {
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

    // MARK: - Text

    /// A file's lines, as the viewer numbers them: split on `\n`, a `\r`
    /// before it dropped, and no phantom empty line after a final newline.
    static func lines(of text: String) -> [String] {
        guard !text.isEmpty else { return [] }
        var lines = text.split(separator: "\n", omittingEmptySubsequences: false).map { line in
            line.hasSuffix("\r") ? String(line.dropLast()) : String(line)
        }
        if text.hasSuffix("\n") { lines.removeLast() }
        return lines
    }

    /// One place a find matched: a line, and a range in it.
    struct Match: Equatable {
        var line: Int
        var range: Range<String.Index>
    }

    /// Every place `query` appears, ignoring case, in order. Empty for an
    /// empty query. Capped, so a one-letter query in a long log doesn't
    /// build a quarter of a million ranges.
    static func matches(_ query: String, in lines: [String], limit: Int = 10_000) -> [Match] {
        guard !query.isEmpty else { return [] }
        var out: [Match] = []
        for (index, line) in lines.enumerated() {
            var from = line.startIndex
            while from < line.endIndex,
                let found = line.range(of: query, options: .caseInsensitive, range: from..<line.endIndex)
            {
                out.append(Match(line: index, range: found))
                if out.count == limit { return out }
                from = found.upperBound > found.lowerBound ? found.upperBound : line.index(after: found.lowerBound)
            }
        }
        return out
    }

    /// The line a go-to-line field names, 1-based as typed, clamped to the
    /// file: "120", "L120", ":120". Nil for anything else.
    static func line(from typed: String, count: Int) -> Int? {
        var text = typed.trimmingCharacters(in: .whitespaces)
        if text.hasPrefix(":") || text.hasPrefix("L") || text.hasPrefix("l") { text.removeFirst() }
        guard count > 0, let n = Int(text) else { return nil }
        return min(max(n, 1), count)
    }

    /// What Copy puts on the pasteboard for the lines `range` covers
    /// (0-based, inclusive): the lines, one per line.
    static func copied(_ lines: [String], range: ClosedRange<Int>) -> String {
        let clamped = range.clamped(to: 0...max(lines.count - 1, 0))
        guard !lines.isEmpty else { return "" }
        return lines[clamped].joined(separator: "\n")
    }

    /// `path:12` or `path:12-30` (1-based), what Copy Reference puts on the
    /// pasteboard: the form an agent reads a place in a file by.
    static func reference(_ path: String, range: ClosedRange<Int>?) -> String {
        guard let range else { return path }
        let first = range.lowerBound + 1
        let last = range.upperBound + 1
        return first == last ? "\(path):\(first)" : "\(path):\(first)-\(last)"
    }

    /// A selection grown to `line` from `anchor`, either way.
    static func extend(from anchor: Int, to line: Int) -> ClosedRange<Int> {
        min(anchor, line)...max(anchor, line)
    }

    /// A file's size the way Finder says it.
    static func size(_ bytes: UInt64) -> String {
        ByteCountFormatter.string(fromByteCount: Int64(clamping: bytes), countStyle: .file)
    }

    /// The width the longest line needs, in characters, for a horizontal
    /// scroller whose width is stated rather than measured (`ChangesPane`'s
    /// rule: measuring makes a lazy stack build every row).
    static func widest(_ lines: [String]) -> Int {
        lines.reduce(0) { max($0, $1.count) }
    }

    /// A `/`-query from the ⌘P field: the path fragment after the slash, or
    /// nil when the query isn't one.
    static func fileQuery(_ typed: String) -> String? {
        let trimmed = typed.trimmingCharacters(in: .whitespaces)
        guard trimmed.hasPrefix("/") else { return nil }
        return String(trimmed.dropFirst()).trimmingCharacters(in: .whitespaces)
    }
}
