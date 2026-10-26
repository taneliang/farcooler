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
}

/// What Show Charter's sheet has to show.
enum CharterReading: Equatable {
    /// The charter, as written.
    case text(String)
    /// The workspace has none yet.
    case missing
    /// Why it couldn't be read, in a sentence.
    case failed(String)

    /// The runner's `--json` answer: `{"exists": …, "text": …}`.
    static func parse(_ data: Data) -> CharterReading? {
        struct Answer: Decodable {
            let exists: Bool
            let text: String?
        }
        guard let answer = try? JSONDecoder().decode(Answer.self, from: data) else { return nil }
        return answer.exists ? .text(answer.text ?? "") : .missing
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
