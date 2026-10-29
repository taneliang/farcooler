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

    /// What the last write or test said, when it did not work.
    ///
    /// Held here rather than shown per control: every one of these is the same
    /// kind of failure — the runner could not be reached, or it refused — and
    /// one banner that says which is easier to read than five that might.
    @Published var failure: String?
    @Published private(set) var loading = false
    /// Who's on this runner and whose Claude account it runs on (ov-508), or
    /// nil until read, and on a runner that doesn't know its people.
    @Published private(set) var people: RunnerPeople?
    /// The devices that may log in to this runner and who added each (ov-855),
    /// or nil until read, and on a runner that won't list them to this Mac.
    @Published private(set) var enrolled: EnrolledDevices?
    /// Why the last revoke didn't take, in this app's words, or nil.
    @Published private(set) var revokeFailure: String?
    /// Each person's Access and devices (ov-856), or nil from a runner that
    /// hasn't recorded the people's access.
    @Published private(set) var access: PeopleAccess?
    func setAccess(_ new: PeopleAccess?) { access = new }
    func setPeople(_ new: RunnerPeople?) { people = new }
    /// The runner reached as this Mac's paired device, when the owner's ssh
    /// can't answer and this Mac is paired with it (ov-856): its admin actions
    /// are then signed with Touch ID. Nil otherwise.
    var remote: RemoteStepUp?
    /// Opens that connection. A test answers it.
    var stepUpOpener: (String) async -> RemoteStepUp? = { await MacRemoteStepUp.open(target: $0) }
    /// The six digits this Mac shows while its own key waits for an approval.
    @Published var shownCode: String?
    /// The owner's warning that their shell's startup files export a Claude
    /// key (ov-651), worded by the runner, or nil: none set, no members, or a
    /// runner from before.
    @Published private(set) var claudeKeyWarning: String?

    /// Answers every CLI call in place of the CLI, when set. For tests only.
    var cliForTesting: (([String]) async -> CLI.Result)?
    /// Answers a call that is fed standard input, with that input. For tests
    /// only: a pasted code goes on a pipe, and a stand-in that can't see the
    /// pipe can't tell it from an argument.
    var cliWithInputForTesting: (([String], String) async -> CLI.Result)?

    init(target: String) {
        self.target = target
    }

    /// `--host` for a remote runner, nothing for this one.
    ///
    /// Named for the flag it builds, and the flag is still spelled `--host`: it
    /// lives in shell history and in scripts, the CLI understands it forever,
    /// and a vocabulary change is no reason for this app to be what breaks it.
    var hostArguments: [String] {
        target.isEmpty ? [] : ["--host", target]
    }

    // MARK: - Reading

    func load() async {
        loading = true
        defer { loading = false }
        await withTaskGroup(of: Void.self) { group in
            group.addTask { await self.loadSettings() }
            group.addTask { await self.loadThemes() }
            group.addTask { await self.loadPeople() }
            group.addTask { await self.loadEnrolled() }
            group.addTask { await self.loadClaudeKeyWarning() }
        }
    }

    /// `member list`: a runner from before people answers that it doesn't
    /// know them, which isn't a failure of this screen, so nothing is said.
    func loadPeople() async {
        let result = await cli(hostArguments + ["--json", "member", "list"])
        guard result.ok else {
            // Not reachable as the owner (a teammate's Mac has no shell here):
            // as a paired device, if this Mac is paired with it.
            if !target.isEmpty { await loadAsPairedDevice() }
            return
        }
        remote = nil
        people = RunnerPeople(json: Data(result.output.utf8))
        await loadAccess(members: Data(result.output.utf8))
    }

    /// `client list`: quiet on a failure, like the people above. A runner that
    /// won't list its devices to this Mac has nothing to show, and the screen's
    /// other reads say when it can't be reached.
    func loadEnrolled() async {
        let result = await cli(hostArguments + ["--json", "client", "list"])
        guard result.ok else { return }
        enrolled = EnrolledDevices(json: Data(result.output.utf8))
    }

    /// `client revoke`, with `--with-added` to take the devices it added too
    /// (ov-855). The answer is the list as it now stands; a refusal leaves the
    /// list as it was and says so in `revokeFailure`.
    func revoke(_ device: EnrolledDevices.Device, withAdded: Bool) async {
        let flags = withAdded ? ["--with-added"] : []
        let result = await cli(hostArguments + ["--json", "client", "revoke", device.clientId] + flags)
        guard result.ok, let updated = EnrolledDevices(json: Data(result.output.utf8)) else {
            revokeFailure = EnrolledDevices.notRevoked
            return
        }
        revokeFailure = nil
        enrolled = updated
    }

    /// `status --json`'s `claudeKeyWarning`. Quiet on a failure: this line is
    /// extra, and the screen's other reads say when the runner can't be reached.
    func loadClaudeKeyWarning() async {
        let result = await cli(hostArguments + ["--json", "status"])
        guard result.ok, let data = result.output.data(using: .utf8),
            let body = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        else { return }
        claudeKeyWarning = body["claudeKeyWarning"] as? String
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
            failure = "Couldn’t read that theme."
            return
        }
        guard await run(["theme", "set", "--json-stdin"], stdin: payload) else { return }
        await loadThemes()
    }

    func delete(themeNamed name: String) async {
        guard await run(["theme", "delete", name]) else { return }
        await loadThemes()
    }

    /// Whether members sign in to `agent` with their own account here
    /// (`member own-login`, ov-652): the runner's admins only. The list comes
    /// back with the switches as they now are.
    func setOwnLogin(_ agent: MemberOwnLogins.Agent, _ on: Bool) async {
        let result = await cli(hostArguments + ["--json", "member", "own-login", agent.rawValue, on ? "on" : "off"])
        guard result.ok, let updated = RunnerPeople(json: Data(result.output.utf8)) else {
            failure = Self.tidy(result.said)
            return
        }
        failure = nil
        people = updated
    }

    /// Give a member a role (`member role`, ov-784): the runner's owner
    /// only. The list comes back with the role as it now is; a refusal
    /// leaves the list as it was, so the picker goes back to what the
    /// runner has, and says so in `failure`.
    func setRole(_ handle: String, _ role: RunnerPeople.Role) async {
        let result = await cli(hostArguments + ["--json", "member", "role", handle, role.rawValue])
        guard result.ok, let updated = RunnerPeople(json: Data(result.output.utf8)) else {
            failure = RunnerPeople.roleNotSaved
            return
        }
        failure = nil
        people = updated
    }

    // MARK: - Claude sign-in (ov-584)

    /// What the Claude section needs to sign a member in and out on this
    /// runner, or nil where it can't: a runner that doesn't offer it, or a
    /// list not yet read. The calls are the CLI's, one process each, since
    /// this window has no client of its own; the pasted code goes on a pipe
    /// to the process, never in its arguments.
    func claudeSignIn(runner: String) -> ClaudeSignInSupport? {
        guard let people, people.offersClaudeSignIn else { return nil }
        return ClaudeSignInSupport(
            runner: runner,
            calls: ClaudeSignInModel.Calls(
                start: { [weak self] handle in
                    guard let self else { throw ClaudeSignIn.CallError.unreachable }
                    let said = await self.signInCall(["claude-sign-in", handle])
                    guard let started = ClaudeSignInModel.Started(json: Data(said.output.utf8)) else {
                        throw said.ok ? ClaudeSignIn.CallError.refused(word: nil, what: nil) : ClaudeSignIn.CallError(cliErrors: said.errors)
                    }
                    return started
                },
                finish: { [weak self] session, code in
                    guard let self else { throw ClaudeSignIn.CallError.unreachable }
                    let said = await self.signInCall(["claude-sign-in-finish", session], stdin: code)
                    guard said.ok, let login = ClaudeLogin(json: Data(said.output.utf8)) else {
                        throw said.ok ? ClaudeSignIn.CallError.refused(word: nil, what: nil) : ClaudeSignIn.CallError(cliErrors: said.errors)
                    }
                    return login
                },
                cancel: { [weak self] session in
                    _ = await self?.signInCall(["claude-sign-in-cancel", session])
                }),
            signOut: { [weak self] handle in
                await self?.signInCall(["claude-sign-out", handle]).ok == true
            },
            changed: { [weak self] in await self?.loadPeople() })
    }

    /// `member <args>` on this runner, as JSON.
    private func signInCall(_ args: [String], stdin: String? = nil) async -> CLI.Result {
        await cli(hostArguments + ["--json", "member"] + args, stdin: stdin)
    }

    // MARK: - Plumbing

    func cli(_ args: [String], stdin: String? = nil) async -> CLI.Result {
        if let stdin, let cliWithInputForTesting { return await cliWithInputForTesting(args, stdin) }
        if let cliForTesting { return await cliForTesting(args) }
        return await CLI.run(args, stdin: stdin)
    }

    private func json(_ args: [String]) async -> [String: Any]? {
        let result = await cli(hostArguments + ["--json"] + args)
        guard result.ok, let data = result.output.data(using: .utf8),
            let body = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        else {
            failure = Self.tidy(result.said)
            return nil
        }
        return body
    }

    @discardableResult
    private func run(_ args: [String], stdin: String? = nil) async -> Bool {
        let result = await cli(hostArguments + args, stdin: stdin)
        if !result.ok {
            failure = Self.tidy(result.said)
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
    static func tidy(_ output: String) -> String {
        let line = output
            .split(separator: "\n")
            .map { $0.trimmingCharacters(in: .whitespaces) }
            .first { !$0.isEmpty && !$0.hasPrefix("Usage:") }
        guard let line, !line.isEmpty else { return "That runner couldn’t be reached." }
        return line.replacingOccurrences(of: "error: ", with: "")
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
