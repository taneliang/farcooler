import AgentKit
import Foundation

/// The first run's charter, on the runner (ov-519 phase 2b): the survey the
/// Reading screen waits on, and the charter the Charter screen writes.
extension DaemonClient {
    /// Whether this runner can survey a repository and write a charter from
    /// it (`repo_survey`). An older one starts the orchestrator as before,
    /// and the orchestrator asks.
    var surveysRepositories: Bool { daemonBuild?.can(.repoSurvey) == true }

    /// What `runRaw` answers for a run its task's cancellation ended.
    static let cancelledMessage = "Cancelled."

    /// `farcooler repo survey --repo <id> --json`: the survey, or the stderr
    /// to word a refusal from. Through `runRaw`, so a failure is the Reading
    /// screen's to say rather than the window's banner.
    func surveyRepository(_ repository: String) async -> (data: Data?, message: String?) {
        await runRaw(["repo", "survey", "--repo", repository, "--json"])
    }

    /// `farcooler workspace write-charter …`, as `CharterForm.writeArguments`
    /// makes it.
    func writeCharter(_ arguments: [String]) async -> (data: Data?, message: String?) {
        await runRaw(arguments)
    }

    /// The Reading screen's sentence for a survey that didn't come back.
    static func surveyRefusal(_ message: String?, repository: String) -> String {
        let context = FirstRunCopy.Charter.readFailed(repository)
        if let shared = RefusalCopy.sentence(after: context, message) { return shared }
        if TaskFailure.code(in: message) != nil { return "\(context) Try again." }
        return "\(context) Check that the runner is reachable, then try again."
    }

    /// The Charter screen's sentence for a charter that wasn't written.
    static func charterRefusal(_ message: String?, repository: String) -> String {
        let context = FirstRunCopy.Charter.writeFailed(repository)
        if let shared = RefusalCopy.sentence(after: context, message) { return shared }
        if TaskFailure.code(in: message) != nil { return "\(context) Try again." }
        return "\(context) Check that the runner is reachable, then try again."
    }

    /// Whether this runner sends a charter's text (`workspace_charter`,
    /// ov-755), so a Mac that isn't its runner can show it.
    var readsCharters: Bool { daemonBuild?.can(.workspaceCharter) == true }

    /// `farcooler workspace charter <id> --json`, by the workspace's id.
    static func readCharterArguments(_ workspace: WorkspaceSummary) -> [String] {
        ["workspace", "charter", workspace.id, "--json"]
    }

    /// A workspace's charter from this runner, for Show Charter's sheet.
    func readCharter(_ workspace: WorkspaceSummary) async -> CharterReading {
        let (data, message) = await runRaw(Self.readCharterArguments(workspace))
        if let data, let reading = CharterReading.parse(data) { return reading }
        return .failed(CharterReading.refusal(data == nil ? message : nil, workspace: workspace))
    }

    /// Whether Show Charter's sheet lets this Mac edit `workspace`'s charter
    /// (ov-784): on a runner that takes an edit (`member_roles`), for its
    /// owner, the workspace's lead and a Maintainer
    /// (`RunnerPeople.editsCharter`). The owner's Mac knows it's the owner
    /// before the people are read (`TalkRoster.viewer`).
    func editsCharter(of workspace: WorkspaceSummary) -> Bool {
        guard daemonBuild?.can(.memberRoles) == true else { return false }
        if talk.viewer == "" { return true }
        return talk.people?.editsCharter(of: workspace) == true
    }

    /// Whether this Mac may rename this runner's workspaces and set their
    /// settings that act as nobody, the wake switch among them (ov-784): the
    /// owner always, and a Maintainer on a runner that lets one
    /// (`workspace_maintainers`). Nil while it isn't known: who this Mac is
    /// hasn't been read.
    var keepsWorkspaces: Bool? {
        if talk.viewer == "" { return true }
        guard let people = talk.people else { return nil }
        return daemonBuild?.can(.workspaceMaintainers) == true && people.keepsWorkspaces
    }

    /// `farcooler workspace set-charter <id> --from <file> --expect <hash>
    /// --json`: the edit, made to the charter whose hash was read.
    static func setCharterArguments(_ workspace: WorkspaceSummary, file: String, expected: String) -> [String] {
        ["workspace", "set-charter", workspace.id, "--from", file, "--expect", expected, "--json"]
    }

    /// Replace `workspace`'s charter with `text`, made to the one whose hash
    /// was `expected` ("" for none): the charter as now written, or why not.
    /// The text goes through a file of this Mac's, never the command line.
    func saveCharter(_ workspace: WorkspaceSummary, text: String, expected: String) async -> CharterReading {
        let file = FileManager.default.temporaryDirectory
            .appendingPathComponent("farcooler-charter-\(UUID().uuidString).md")
        do {
            try Data(text.utf8).write(to: file, options: [.atomic])
        } catch {
            return .failed(CharterReading.notSaved(workspace))
        }
        defer { try? FileManager.default.removeItem(at: file) }
        let (data, message) = await runRaw(Self.setCharterArguments(workspace, file: file.path, expected: expected))
        guard let data else { return .failed(CharterReading.saveRefusal(message, workspace: workspace)) }
        struct Answer: Decodable { let sha256: String }
        let sha = (try? JSONDecoder().decode(Answer.self, from: data))?.sha256 ?? ""
        return .text(text, sha256: sha)
    }
}

/// What Show Charter's sheet has to show.
enum CharterReading: Equatable {
    /// The charter, as written, and its hash, which an edit is made to
    /// ("" from a runner before ov-757).
    case text(String, sha256: String = "")
    /// The workspace has none yet.
    case missing
    /// Why it couldn't be read, in a sentence.
    case failed(String)

    /// The runner's `--json` answer: `{"exists": …, "text": …}`.
    static func parse(_ data: Data) -> CharterReading? {
        struct Answer: Decodable {
            let exists: Bool
            let text: String?
            let sha256: String?
        }
        guard let answer = try? JSONDecoder().decode(Answer.self, from: data) else { return nil }
        return answer.exists ? .text(answer.text ?? "", sha256: answer.sha256 ?? "") : .missing
    }

    /// The hash an edit of this is made to: "" for no charter yet.
    var sha256: String {
        if case .text(_, let sha) = self { return sha }
        return ""
    }

    /// The sentence for an edit the runner didn't take (ov-784): one made
    /// meanwhile is kept, and this one isn't lost; a device that may not edit
    /// it is told who may.
    static func saveRefusal(_ message: String?, workspace: WorkspaceSummary) -> String {
        let context = "Couldn’t save \(workspace.name)’s charter."
        switch TaskFailure.what(in: message) {
        case "charter_changed":
            return "Someone else changed \(workspace.name)’s charter while you were editing. "
                + "Your text is still here: copy it, then close this and open the charter again to see theirs."
        case "not_yours":
            return "\(context) Only the runner’s owner, this workspace’s lead and Maintainers can edit it."
        case "charter_empty":
            return "\(context) A charter needs something written in it."
        case "charter_too_long":
            return "\(context) A charter can be at most 256 KB."
        default:
            if let shared = RefusalCopy.sentence(after: context, message) { return shared }
            if TaskFailure.code(in: message) != nil { return "\(context) Try again." }
            return notSaved(workspace)
        }
    }

    /// An edit that never reached the runner.
    static func notSaved(_ workspace: WorkspaceSummary) -> String {
        "Couldn’t save \(workspace.name)’s charter. Check that the runner is reachable, then try again."
    }

    /// The sentence for a read that failed: the runner's own refusal where
    /// it gave one (a removed member is told so), else the CLI's.
    static func refusal(_ message: String?, workspace: WorkspaceSummary) -> String {
        let context = "Couldn’t read \(workspace.name)’s charter."
        let said = (message ?? "").lowercased()
        if let shared = RefusalCopy.sentence(after: context, message) { return shared }
        if said.contains("needs an update") { return "\(context) \(RunnerRefusal.capabilityUnsupported.sentence)" }
        if said.contains("no workspace matching") { return "\(context) \(RunnerRefusal.notFound.sentence)" }
        return unreachable(workspace)
    }

    /// A read that never reached the runner, or came back unreadable.
    static func unreachable(_ workspace: WorkspaceSummary) -> String {
        "Couldn’t read \(workspace.name)’s charter. Check that the runner is reachable, then try again."
    }
}
