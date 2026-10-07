import AgentKit
import Foundation

/// A held ask answered from its row (ov-370, R-33): the runner writes it to
/// the hook claude is waiting on, and the first answer from any device wins.
/// Nothing is typed into claude's dialog.
extension RunnerCore: AgentAnswerSink {
    func answer(terminal: String, ask: String, option: String, answers: [String: String]) async throws {
        var args: [String: any Sendable] = ["terminal": terminal, "requestId": ask, "optionId": option]
        if !answers.isEmpty { args["answers"] = answers }
        _ = try await call("terminal.agent_answer", args)
    }
}

/// What a row's buttons need from its pane: where an answer goes, and how
/// the last one went.
struct NativeAnswer {
    /// The held ask whose answer is on its way.
    var answering: String?
    /// Why an ask's answer didn't land, by the ask's id.
    var issues: [String: String]
    let send: (_ ask: AgentRow.Ask, _ option: String, _ answers: [String: String]) -> Void
}

extension NativePaneModel {
    /// The rows' answering, where the runner takes answers.
    var nativeAnswer: NativeAnswer? {
        guard answers != nil else { return nil }
        return NativeAnswer(answering: answering, issues: answerIssues) { [weak self] ask, option, answers in
            Task { await self?.answer(ask, option: option, answers: answers) }
        }
    }

    /// Answer the held ask on `ask`'s row: `option`, and a question's
    /// `answers`. One at a time; refused, the row says why.
    func answer(_ ask: AgentRow.Ask, option: String, answers given: [String: String] = [:]) async {
        guard let sink = answers, let id = ask.held, answering == nil, AgentConversation.answerable(ask) else { return }
        answering = id
        answerIssues[id] = nil
        defer { answering = nil }
        do {
            try await sink.answer(terminal: terminal, ask: id, option: option, answers: given)
        } catch {
            answerIssues[id] = Self.answerIssue(for: error)
        }
    }

    static func answerIssue(for error: Error) -> String {
        switch error as? RunnerCore.Failure {
        case .timedOut?, .lost(_, notSent: false)?: AgentConversation.answerIssue(what: nil, timedOut: true)
        case .refused(_, _, let what)?: AgentConversation.answerIssue(what: what)
        default: AgentConversation.answerIssue(what: nil)
        }
    }
}
