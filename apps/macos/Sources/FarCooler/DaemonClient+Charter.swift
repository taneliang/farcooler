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
}
