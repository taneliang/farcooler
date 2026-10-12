import Foundation

/// Far Cooler's block as a list of runners, so a later join adds to it.
///
/// The fence writer takes the whole block every time, and an array that held
/// only the runners of one reply deleted every earlier one, including a tunneled
/// runner whose only route this is. So each write starts from what is already
/// inside the fence.
extension SshConfig {
    /// One runner's `Host` entry.
    struct Entry: Equatable {
        var alias: String
        /// Who the entry is for, apart from what it is called: the local id of a
        /// tunneled runner, or user, host and port for a direct one. A runner
        /// granted again replaces its own entry, and a different runner that
        /// wants the same alias is renamed instead.
        var key: String
        /// The `Host` line and what follows it, without comments.
        var lines: [String]
        var tunneled: Bool
    }

    /// The key an entry for `runner` has.
    static func key(for runner: CeremonyRunner) -> String {
        switch runner.reach {
        case .tailcat: return runner.id
        case .direct(let host, let port): return "direct:\(runner.user)@\(host):\(port)"
        }
    }

    static func entry(for runner: CeremonyRunner, alias: String, identity: URL, program: String)
        -> Entry
    {
        Entry(
            alias: alias, key: key(for: runner),
            lines: block(for: runner, alias: alias, identity: identity, program: program),
            tunneled: runner.isTunneled)
    }

    /// What is inside the fence in `file` now, one ``Entry`` per `Host`.
    static func entries(in file: URL = SshConfig.path) -> [Entry] {
        guard let text = try? String(contentsOf: file, encoding: .utf8) else { return [] }
        var inside = false
        var found: [Entry] = []
        for raw in text.components(separatedBy: .newlines) {
            let line = raw.trimmingCharacters(in: .whitespaces)
            if line == beginMarker { inside = true; continue }
            if line == endMarker { break }
            guard inside, !line.isEmpty, !line.hasPrefix("#") else { continue }
            if line.hasPrefix("Host ") {
                found.append(Entry(
                    alias: String(line.dropFirst(5)).trimmingCharacters(in: .whitespaces),
                    key: "", lines: [], tunneled: false))
            }
            guard !found.isEmpty else { continue }
            found[found.count - 1].lines.append(raw.trimmingCharacters(in: .newlines))
        }
        return found.map(keyed)
    }

    private static func keyed(_ entry: Entry) -> Entry {
        var entry = entry
        func value(_ keyword: String) -> String? {
            entry.lines.compactMap { line -> String? in
                let words = line.split(separator: " ", omittingEmptySubsequences: true)
                return words.first?.lowercased() == keyword.lowercased() && words.count >= 2
                    ? words.dropFirst().joined(separator: " ") : nil
            }.first
        }
        if let proxy = value("ProxyCommand"), proxy.contains("runner pipe "),
            let id = proxy.split(separator: " ").last
        {
            entry.tunneled = true
            entry.key = String(id)
        } else {
            entry.key =
                "direct:\(value("User") ?? "")@\(value("HostName") ?? ""):\(value("Port") ?? "22")"
        }
        return entry
    }

    /// The block's lines: a short header saying whose it is, then each entry
    /// under a comment saying what it is for. The comments are written fresh
    /// every time, so nothing here has to survive a parse.
    static func render(_ entries: [Entry]) -> [String] {
        guard !entries.isEmpty else { return [] }
        let header = [
            "# Far Cooler wrote and manages this block.",
            "# Each Host below is a runner. One with a ProxyCommand is reached through Far Cooler's tunnel.",
            "# Removing a runner in Far Cooler removes its entry here.",
        ]
        return header + entries.flatMap { entry -> [String] in
            let what = entry.tunneled
                ? "a runner reached through the Far Cooler tunnel" : "a runner reached at its address"
            return ["# \(entry.alias): \(what)"] + entry.lines
        }
    }
}
