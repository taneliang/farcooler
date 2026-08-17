import Foundation

/// Far Cooler's fenced block in `~/.ssh/config`, so that Zed, git and Terminal
/// reach runners too.
///
/// **This file composes the block and decides the aliases. It does not write
/// the bytes.** The write goes through the same routine that owns
/// `~/.ssh/authorized_keys` — `crates/daemon/src/fence.rs`, which is generic
/// over its path and its markers for exactly this reason. That routine opens
/// every path component relative to a held directory descriptor with
/// `O_NOFOLLOW`, verifies each with `fstat`, takes an advisory lock, writes a
/// temp file, `fsync`s the file AND its directory before renaming, and leaves a
/// checksummed backup beside it. A second implementation in Swift would drift a
/// missing `fsync` into a corrupted `~/.ssh/config`, which breaks Zed, git and
/// plain `ssh` at once — so there is one implementation, in Rust, and it is
/// tested there.
///
/// See ``SshConfig/write(_:identity:)`` for the one FFI entry point this still
/// needs and does not have.
enum SshConfig {
    /// The file this writes into.
    static var path: URL {
        URL(fileURLWithPath: NSHomeDirectory())
            .appendingPathComponent(".ssh")
            .appendingPathComponent("config")
    }

    /// The fence, spelled exactly as `crates/daemon/src/fence.rs` spells it.
    ///
    /// A copy of a constant, which is a thing to be uncomfortable about — so it
    /// is here, named, rather than inline in three places. It exists because
    /// this file has to SKIP Far Cooler's own block when it scans for alias
    /// collisions, and the alias scan is on the Swift side. `#` opens a comment
    /// in `ssh_config` as it does in `authorized_keys`, which is why one pair of
    /// markers serves both files.
    ///
    /// **If the Rust constants ever change, these change with them** — an
    /// existing file's marker is matched literally, and a reworded one reads as
    /// a file with no fence at all.
    static let beginMarker = "# BEGIN FAR COOLER — do not edit inside this block"
    static let endMarker = "# END FAR COOLER"

    // MARK: - The block

    /// The lines of one runner's `Host` block.
    ///
    /// ```
    /// Host box
    ///   HostName box.tail-1234.ts.net
    ///   User you
    ///   Port 22
    ///   IdentityFile ~/.ssh/farcooler-macbook-air
    ///   IdentitiesOnly yes
    /// ```
    ///
    /// `HostName` is always explicit, so the alias is never resolved as a
    /// hostname — shadowing an existing pattern is the only risk the alias
    /// carries, and ``aliases(for:avoiding:)`` is what addresses that.
    ///
    /// One line per array element, and no blank ones. Both are the fence
    /// writer's rules: it refuses an entry containing a newline, and its
    /// read-back check counts non-empty lines, so a blank separator would make
    /// the file fail to read back as what was written.
    static func block(for runner: CeremonyRunner, alias: String, identity: URL) -> [String] {
        [
            "Host \(alias)",
            "  HostName \(runner.address)",
            "  User \(runner.user)",
            "  Port \(runner.port)",
            "  IdentityFile \(tildeCollapsed(identity))",
            // Bounds ssh to the identities named in the configuration and on
            // the command line, so an agent holding a dozen keys does not
            // exhaust MaxAuthTries before it offers this one.
            "  IdentitiesOnly yes",
        ]
    }

    /// `~/.ssh/farcooler-macbook-air`, not `/Users/you/.ssh/…`.
    ///
    /// The file is frequently in a dotfiles repository shared between machines,
    /// where an absolute home directory is wrong on every machine but one.
    static func tildeCollapsed(_ url: URL) -> String {
        let home = NSHomeDirectory()
        guard url.path.hasPrefix(home + "/") else { return url.path }
        return "~" + url.path.dropFirst(home.count)
    }

    // MARK: - Aliases

    /// An alias per runner, collision-checked against everything ssh will read.
    ///
    /// Three rules, each of which has cost somebody an afternoon:
    ///
    /// - **Per RUNNER, not per host.** `alice@box` and `bob@box` are two
    ///   runners on one machine and both would want `Host box`. The second gets
    ///   the user in its name, because the thing that differs is the user.
    /// - **Collision-checked against the config and everything it includes.** A
    ///   runner labeled `github.com` would otherwise take over git — the block
    ///   goes at the top of the file and `ssh_config` is first-match-wins, so it
    ///   would win `IdentityFile` for every push anybody makes.
    /// - **On a hit, suffix and say so.** Silently taking the name is what
    ///   breaks git; silently refusing is what leaves Zed unable to open
    ///   anything. The returned ``Resolution`` carries the message.
    static func aliases(for runners: [CeremonyRunner], avoiding taken: Set<String>) -> Resolution {
        var used = taken
        var chosen: [String: String] = [:]
        var renamed: [String] = []

        for runner in runners {
            let wanted = slug(runner.label.isEmpty ? runner.address : runner.label)
            var alias = wanted
            if used.contains(alias) {
                // The user first: two runners on one machine differ by user,
                // and `alice-box` is a name somebody can read.
                alias = "\(slug(runner.user))-\(wanted)"
            }
            var counter = 2
            while used.contains(alias) {
                alias = "\(wanted)-\(counter)"
                counter += 1
            }
            if alias != wanted { renamed.append("\(wanted) → \(alias)") }
            used.insert(alias)
            chosen[runner.id] = alias
        }

        return Resolution(aliases: chosen, message: message(renamed))
    }

    /// What aliases were chosen, and what to tell the person about it.
    struct Resolution: Equatable {
        /// Runner id → the alias actually used.
        var aliases: [String: String]
        /// Nil when nothing collided, which is the ordinary case.
        var message: String?
    }

    private static func message(_ renamed: [String]) -> String? {
        guard !renamed.isEmpty else { return nil }
        let list = renamed.joined(separator: ", ")
        return renamed.count == 1
            ? "Your ~/.ssh/config already had that name, so Far Cooler used \(list) instead."
            : "Your ~/.ssh/config already had those names, so Far Cooler used \(list) instead."
    }

    /// A label as an ssh alias: lowercase, and nothing that needs quoting.
    ///
    /// A label is user-supplied and arrives in a scanned code. A space in one
    /// would make `Host my runner` two patterns, and the second would match
    /// something.
    static func slug(_ label: String) -> String {
        let allowed = CharacterSet(charactersIn:
            "abcdefghijklmnopqrstuvwxyz0123456789-_.")
        let slug = label.lowercased()
            .replacingOccurrences(of: " ", with: "-")
            .unicodeScalars
            .filter { allowed.contains($0) }
            .map(String.init)
            .joined()
        return slug.isEmpty ? "runner" : slug
    }

    // MARK: - Reading what is already there

    /// Every `Host` pattern ssh would match before it reached Far Cooler's
    /// block, from the config and everything it includes.
    ///
    /// **Far Cooler's own block is skipped**, or every rewrite would find last
    /// time's aliases and suffix them again — `box`, `box-2`, `box-3`, once per
    /// enrollment.
    ///
    /// A textual scan is a snapshot and this says so out loud: `Match exec`,
    /// files included later, and hostname canonicalization can all change what
    /// ssh does afterwards. Far Cooler writes the clearest block it can and does
    /// not claim to own the file.
    static func patternsInUse(from file: URL? = nil, depth: Int = 0) -> Set<String> {
        // Includes can include includes. Bounded rather than trusted: a config
        // that includes itself is a config, not an attack, and either way this
        // must return.
        guard depth < 8 else { return [] }
        let file = file ?? path
        guard let text = try? String(contentsOf: file, encoding: .utf8) else { return [] }

        var patterns: Set<String> = []
        var insideOurBlock = false
        for raw in text.components(separatedBy: .newlines) {
            let line = raw.trimmingCharacters(in: .whitespaces)
            if line == beginMarker { insideOurBlock = true; continue }
            if line == endMarker { insideOurBlock = false; continue }
            if insideOurBlock || line.isEmpty || line.hasPrefix("#") { continue }

            // `ssh_config` accepts `Keyword value`, `Keyword=value` and any
            // case. Normalizing here is what keeps `host=github.com` from
            // sailing past a check for `Host `.
            let normalized = line.replacingOccurrences(of: "=", with: " ")
            let words = normalized.split(separator: " ", omittingEmptySubsequences: true)
            guard let keyword = words.first?.lowercased() else { continue }
            let arguments = words.dropFirst().map(String.init)

            if keyword == "host" {
                patterns.formUnion(arguments.map { $0.lowercased() })
            } else if keyword == "include" {
                for argument in arguments {
                    for included in expand(argument) {
                        patterns.formUnion(patternsInUse(from: included, depth: depth + 1))
                    }
                }
            }
        }
        return patterns
    }

    /// An `Include` argument as the files it names.
    ///
    /// Relative paths resolve against `~/.ssh` for a user config, which is what
    /// ssh does. Globs are expanded, because `Include ~/.ssh/config.d/*` is the
    /// common form and the whole reason the block has to go above it.
    private static func expand(_ argument: String) -> [URL] {
        var pattern = argument
        if pattern.hasPrefix("~/") {
            pattern = NSHomeDirectory() + pattern.dropFirst(1)
        } else if !pattern.hasPrefix("/") {
            pattern = path.deletingLastPathComponent().appendingPathComponent(pattern).path
        }

        var found = glob_t()
        defer { globfree(&found) }
        guard glob(pattern, 0, nil, &found) == 0 else { return [] }
        return (0..<Int(found.gl_pathc)).compactMap { index in
            found.gl_pathv[index].map { URL(fileURLWithPath: String(cString: $0)) }
        }
    }

    // MARK: - Writing

    /// Replace Far Cooler's block with one `Host` entry per runner.
    ///
    /// `identity` is **Key B**, always. Key A is never in this file: Far Cooler
    /// passes it with `-i` on the command line (`crates/cli/src/remote.rs`), so
    /// deleting this block takes Zed's access away and cannot touch Far
    /// Cooler's. That property is the entire reason there are two keys, and
    /// ``assertNotKeyA(_:)`` is what keeps it true here.
    ///
    /// ## The entry point this needs, and does not have
    ///
    /// `fence::write(path, markers, entries, foreign)` is reachable from
    /// nothing but Rust today, and it also cannot place a NEW block at the top
    /// of a file — `fence::rebuilt` appends when it finds no existing fence,
    /// which for `~/.ssh/config` is the one placement that does not work.
    /// `ssh_config` is FIRST-match-wins, so an `Include ~/.ssh/config.d/*` or a
    /// `Host *` above this block silently wins every keyword in it.
    ///
    /// So what is wanted is one entry point, in `crates/client/src/ffi.rs`,
    /// over a `fence::write` that has learned where to put a new block:
    ///
    /// ```c
    /// /* Rewrite Far Cooler's fenced block in ~/.ssh/config.
    ///  *
    ///  * `entries_json` is a JSON array of lines — no newlines, no marker
    ///  * lines, no blanks — which become the block in order, at the TOP of the
    ///  * file for a file that has no fence yet, and in place for one that has.
    ///  * An empty array removes the block.
    ///  *
    ///  * Answers {"ok":true} or {"error":"damaged"|"missing"|"io"}: a word,
    ///  * never a Rust error string, exactly as the ceremony calls do.
    ///  */
    /// size_t farcooler_client_ssh_config_write(const char *path,
    ///                                          const char *entries_json,
    ///                                          uint8_t *out, size_t capacity);
    /// ```
    ///
    /// Until that lands this refuses rather than writing the file a second way.
    /// The six tests the plan names live beside plan 3's fence fixtures, in
    /// Rust, where they cover both files at once.
    static func write(_ entries: [String], identity: URL) throws {
        for line in entries { try assertNotKeyA(line) }
        throw SshConfigError.writerUnavailable
    }

    /// Key A must never appear in `~/.ssh/config`.
    ///
    /// Checked on the composed line rather than trusted from the call site,
    /// because the call site is where the mistake would be made. If this ever
    /// fires, the split between the two keys has been broken somewhere above:
    /// an `IdentityFile` pointing at Key A hands a shell client a key whose
    /// forced command makes it useless for a shell, and hands `~/.ssh/config`
    /// a key whose deletion WOULD break Far Cooler.
    static func assertNotKeyA(_ line: String) throws {
        guard line.lowercased().contains("identityfile") else { return }
        let keyDirectory = DeviceKey.directory.path
        if line.contains(keyDirectory) || line.contains(tildeCollapsed(DeviceKey.directory)) {
            throw SshConfigError.keyAInConfig
        }
    }
}

/// Which alias Far Cooler wrote for a runner, so editors can be handed it.
///
/// **This is the whole point of the block.** `Editors.swift` builds
/// `ssh://{host}{path}` for Zed and `vscode-remote://ssh-remote+{host}{path}`
/// for the VS Code family. Given `you@box.tail-1234.ts.net` — which is what a
/// runner added by hand is called here — ssh matches no `Host` entry, so the
/// `IdentityFile` this app just wrote is never offered and the remote open
/// fails with a key error. Given `box`, it matches, and Zed opens.
///
/// Stored rather than re-derived from the file on every menu: opening the
/// editor menu would otherwise parse `~/.ssh/config` and every file it
/// includes, and the answer is one this app wrote and therefore knows.
enum SshConfigAliases {
    private static let key = "ssh.aliases"

    static func remember(_ alias: String, for target: String) {
        var map = all
        map[target] = alias
        UserDefaults.standard.set(map, forKey: key)
    }

    static func forget(_ target: String) {
        var map = all
        map.removeValue(forKey: target)
        UserDefaults.standard.set(map, forKey: key)
    }

    static var all: [String: String] {
        UserDefaults.standard.dictionary(forKey: key) as? [String: String] ?? [:]
    }

    /// What to substitute into an editor's `{host}`: the alias when Far Cooler
    /// wrote one, and otherwise the target exactly as it always was.
    static func editorTarget(for target: String) -> String {
        all[target] ?? target
    }
}

enum SshConfigError: LocalizedError, Equatable {
    /// The shared Rust writer is not reachable from Swift yet. See
    /// ``SshConfig/write(_:identity:)``.
    case writerUnavailable
    case keyAInConfig
    case damaged

    var errorDescription: String? {
        switch self {
        case .writerUnavailable:
            return "Far Cooler couldn't update ~/.ssh/config on this Mac. "
                + "The keys were enrolled; Zed and git won't find the runners until it can."
        case .keyAInConfig:
            return "Far Cooler couldn't update ~/.ssh/config on this Mac."
        case .damaged:
            // Refuses rather than repairs, and says which file, because the way
            // out is a person looking at it. Never a suggestion to delete it.
            return "Far Cooler's block in ~/.ssh/config isn't the shape it wrote. "
                + "Nothing was changed. Open the file and check the two Far Cooler comment lines."
        }
    }
}
