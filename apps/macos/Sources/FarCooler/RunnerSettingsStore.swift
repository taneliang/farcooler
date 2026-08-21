import AgentKit
import Foundation
import SwiftUI

/// One runner's `config.toml`, as something a settings sheet can edit.
///
/// Per runner, and that is the whole point: a fleet spans several, each with
/// its own file, and "the branch prefix" is a different answer on each of them.
/// So this is constructed with a target — `""` for this Mac — and every call it
/// makes carries it.
///
/// Talks through the CLI rather than holding a `DaemonClient`, for the reason
/// `CLI` itself was extracted: the Settings window has no client of its own, and
/// a second path to the daemon would be a second answer to which binary this app
/// is driving.
@MainActor
final class RunnerSettingsStore: ObservableObject {
    /// The ssh target, or `""` for this Mac.
    let target: String

    @Published private(set) var branchPrefix = ""
    @Published private(set) var themes: [Theme] = []
    /// Which of `themes` shadow a theme Far Cooler ships.
    ///
    /// Reported by the CLI rather than worked out here: it holds the built-in
    /// table and the runner's list at the same moment, and a client deriving it
    /// would need a third call to find out what "shipped" even means.
    @Published private(set) var shadowsBuiltIn: Set<String> = []
    @Published private(set) var adapters: [AdapterInfo] = []

    /// What the last write or test said, when it did not work.
    ///
    /// Held here rather than shown per control: every one of these is the same
    /// kind of failure — the runner could not be reached, or it refused — and
    /// one banner that says which is easier to read than five that might.
    @Published var failure: String?
    @Published private(set) var loading = false

    init(target: String) {
        self.target = target
    }

    /// `--host` for a remote runner, nothing for this one.
    ///
    /// Named for the flag it builds, and the flag is still spelled `--host`: it
    /// lives in shell history and in scripts, the CLI understands it forever,
    /// and a vocabulary change is no reason for this app to be what breaks it.
    private var hostArguments: [String] {
        target.isEmpty ? [] : ["--host", target]
    }

    // MARK: - Reading

    func load() async {
        loading = true
        defer { loading = false }
        await withTaskGroup(of: Void.self) { group in
            group.addTask { await self.loadSettings() }
            group.addTask { await self.loadThemes() }
            group.addTask { await self.loadAdapters() }
        }
    }

    private func loadSettings() async {
        guard let body = await json(["settings", "show"]) else { return }
        branchPrefix = body["branchPrefix"] as? String ?? ""
    }

    /// The runner's own themes, which is what this screen edits.
    ///
    /// Deliberately not `theme list`, which merges the built-ins in: this list
    /// answers "what does this runner's file define", and a built-in shown here
    /// as if the file defined it would offer a Delete that does nothing.
    /// `RunnerSettings` merges the built-ins for display, marking each.
    private func loadThemes() async {
        guard let body = await json(["theme", "list", "--only-host"]) else { return }
        let items = body["themes"] as? [[String: Any]] ?? []
        themes = items.compactMap(Theme.init(json:))
        shadowsBuiltIn = Set(
            items.filter { $0["shadowsBuiltIn"] as? Bool == true }
                .compactMap { $0["name"] as? String })
    }

    private func loadAdapters() async {
        guard let body = await json(["adapter", "list"]) else { return }
        let items = body["adapters"] as? [[String: Any]] ?? []
        adapters = items.compactMap(AdapterInfo.init(json:))
    }

    // MARK: - Writing

    func setBranchPrefix(_ prefix: String) async {
        guard let body = await json(["settings", "set-branch-prefix", prefix]) else { return }
        // What the file now says, not what was typed: the writer trims.
        branchPrefix = body["branchPrefix"] as? String ?? prefix
    }

    /// Save a theme, whether it is new or replacing one.
    ///
    /// Goes through a temporary JSON file rather than nineteen arguments,
    /// because nineteen positional colors on a command line is a contract
    /// nobody can read and one transposition away from a theme nobody chose.
    func save(theme: Theme) async {
        guard let payload = theme.commandJSON else {
            failure = "That theme could not be described."
            return
        }
        guard await run(["theme", "set", "--json-stdin"], stdin: payload) else { return }
        await loadThemes()
    }

    func delete(themeNamed name: String) async {
        guard await run(["theme", "delete", name]) else { return }
        await loadThemes()
    }

    func save(adapter: AdapterInfo) async {
        guard let payload = adapter.commandJSON else {
            failure = "That adapter could not be described."
            return
        }
        guard await run(["adapter", "set", "--json-stdin"], stdin: payload) else { return }
        await loadAdapters()
    }

    func delete(adapterNamed preset: String) async {
        guard await run(["adapter", "delete", preset]) else { return }
        await loadAdapters()
    }

    /// Prove an adapter works, without saving it first.
    ///
    /// The unsaved form is exactly the input this wants: the question is "will
    /// this work", asked before committing it to the file.
    func test(adapter: AdapterInfo) async -> AdapterTestOutcome {
        guard let payload = adapter.commandJSON else { return .failed(.formUnusable) }
        let result = await CLI.run(hostArguments + ["--json", "adapter", "test", "--json-stdin"], stdin: payload)
        guard let data = result.output.data(using: .utf8),
            let body = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        else {
            // A non-zero exit with unparseable output is the CLI's own words —
            // usually "couldn't reach that runner" — and they are better than
            // anything invented here, so they are kept. They are kept as
            // OUTPUT, under a sentence this app wrote: this used to hand them
            // straight to `.failed` and the editor drew them in its own red,
            // which is the join `776d3e0` and `c42c352` took out everywhere
            // else.
            return .failed(.noAnswer(Self.tidy(result.output)))
        }
        if body["ok"] as? Bool == true {
            return .worked(body["reported"] as? String ?? "answered")
        }
        // Empty rather than a sentence when the field is missing: an outcome
        // with nothing to show says so by having no detail, and the words for
        // that case are `AdapterTestOutcome`'s to choose, not this call site's.
        return .failed(.refused(body["failure"] as? String ?? ""))
    }

    // MARK: - Plumbing

    private func json(_ args: [String]) async -> [String: Any]? {
        let result = await CLI.run(hostArguments + ["--json"] + args)
        guard result.ok, let data = result.output.data(using: .utf8),
            let body = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        else {
            failure = Self.tidy(result.output)
            return nil
        }
        return body
    }

    @discardableResult
    private func run(_ args: [String], stdin: String? = nil) async -> Bool {
        let result = await CLI.run(hostArguments + args, stdin: stdin)
        if !result.ok {
            failure = Self.tidy(result.output)
            return false
        }
        failure = nil
        return true
    }

    /// The CLI's own message, trimmed to something a sheet can show.
    ///
    /// Never a raw multi-line dump: the first meaningful line is the one that
    /// says what went wrong, and the rest is usage text nobody in a settings
    /// window asked for.
    private static func tidy(_ output: String) -> String {
        let line = output
            .split(separator: "\n")
            .map { $0.trimmingCharacters(in: .whitespaces) }
            .first { !$0.isEmpty && !$0.hasPrefix("Usage:") }
        guard let line, !line.isEmpty else { return "That runner couldn’t be reached." }
        return line.replacingOccurrences(of: "error: ", with: "")
    }
}

/// One adapter, plus where it came from.
struct AdapterInfo: Identifiable, Equatable {
    enum Origin: String {
        /// Compiled into the daemon, with no table in the file.
        case builtIn
        /// A table shadowing a shipped adapter of the same name.
        case override
        /// A table for an agent Far Cooler does not ship.
        case user
        case unknown
    }

    /// Which protocol Far Cooler speaks to this agent.
    ///
    /// `native` means the agent's own — `codex app-server`, or the Claude
    /// CLI's stream-json control protocol — rather than an ACP adapter
    /// wrapping it. Only some agents have one; see `nativeIsAvailable`.
    enum Backend: String {
        case acp
        case native
    }

    var preset: String
    var backend: Backend
    var program: String
    var args: [String]
    var env: [String: String]
    var commands: [String]
    var identity: [String]
    var blocked: [String]
    var working: [String]
    var origin: Origin

    var id: String { preset }

    /// Whether this agent has a native backend at all.
    ///
    /// cursor has no protocol Far Cooler speaks first-party, opencode is
    /// already a native subcommand behind ACP with nothing to gain, and an
    /// adapter you added yourself has nothing compiled in for it. The editor
    /// hides the control rather than offering a choice that cannot work.
    var nativeIsAvailable: Bool { preset == "claude" || preset == "codex" }

    /// Whether Far Cooler can host this agent as a chat at all.
    ///
    /// An adapter with no program is a recognized agent that stays a terminal,
    /// which is a real supported state rather than a gap.
    var chatCapable: Bool { !program.trimmingCharacters(in: .whitespaces).isEmpty }

    init(
        preset: String, backend: Backend = .acp, program: String = "", args: [String] = [],
        env: [String: String] = [:],
        commands: [String] = [], identity: [String] = [], blocked: [String] = [],
        working: [String] = [], origin: Origin = .user
    ) {
        self.preset = preset
        self.backend = backend
        self.program = program
        self.args = args
        self.env = env
        self.commands = commands
        self.identity = identity
        self.blocked = blocked
        self.working = working
        self.origin = origin
    }

    init?(json: [String: Any]) {
        guard let preset = json["preset"] as? String else { return nil }
        self.init(
            preset: preset,
            // Absent means acp, matching an omitted `backend` key in the file.
            backend: Backend(rawValue: json["backend"] as? String ?? "") ?? .acp,
            program: json["program"] as? String ?? "",
            args: json["args"] as? [String] ?? [],
            env: json["env"] as? [String: String] ?? [:],
            commands: json["commands"] as? [String] ?? [],
            identity: json["identity"] as? [String] ?? [],
            blocked: json["blocked"] as? [String] ?? [],
            working: json["working"] as? [String] ?? [],
            origin: Origin(rawValue: json["origin"] as? String ?? "") ?? .unknown)
    }

    var commandJSON: String? {
        let body: [String: Any] = [
            "preset": preset,
            "backend": backend.rawValue,
            "program": program,
            "args": args,
            "env": env,
            "commands": commands,
            "identity": identity,
            "blocked": blocked,
            "working": working,
        ]
        guard let data = try? JSONSerialization.data(withJSONObject: body) else { return nil }
        return String(decoding: data, as: UTF8.self)
    }
}

extension Theme {
    init?(json: [String: Any]) {
        guard let name = json["name"] as? String,
            let ansi = json["ansi"] as? [NSNumber], ansi.count == 16
        else { return nil }
        self.init(
            name: name,
            dark: json["dark"] as? Bool ?? true,
            background: (json["background"] as? NSNumber)?.uint32Value ?? 0,
            foreground: (json["foreground"] as? NSNumber)?.uint32Value ?? 0,
            cursor: (json["cursor"] as? NSNumber)?.uint32Value ?? 0,
            ansi: ansi.map(\.uint32Value))
    }

    var commandJSON: String? {
        let body: [String: Any] = [
            "name": name,
            "dark": dark,
            "background": background,
            "foreground": foreground,
            "cursor": cursor,
            "ansi": ansi,
        ]
        guard let data = try? JSONSerialization.data(withJSONObject: body) else { return nil }
        return String(decoding: data, as: UTF8.self)
    }
}
