import AgentKit
import SwiftUI

/// Claude asking: a permission, a question, or a plan to approve (ov-370).
///
/// While the runner's hook holds the ask (`AgentRow.Ask.held`), the row
/// answers it: Allow or Deny; the question's options, then Send Answer; or
/// Approve Plan or Keep Planning. The answer goes to the hook, never as keys
/// into claude's dialog, and the first from any device wins. Once the hold
/// ends, only the terminal can answer, and the row offers Show Terminal.
struct NativeAskRow: View {
    let ask: AgentRow.Ask
    let answer: NativeAnswer?
    let showTerminal: () -> Void
    /// Each question's picked options, by the question's place.
    @State private var picked: [Int: Set<String>] = [:]
    /// Each question's Other, by the question's place.
    @State private var typed: [Int: String] = [:]
    /// The hold this row last answered, so what became of the answer is
    /// still said once the hold ends (review 1 M1).
    @State private var sentFor: String?

    /// Nobody has answered it yet.
    private var waiting: Bool { !ask.answered && ask.answeredBy == nil }
    private var canAnswer: Bool { answer != nil && AgentConversation.answerable(ask) }
    private var sending: Bool { ask.held != nil && answer?.answering == ask.held }

    var body: some View {
        VStack(alignment: .leading, spacing: Spacing.group) {
            HStack(alignment: .firstTextBaseline, spacing: Spacing.group) {
                Image(systemName: waiting ? "questionmark.bubble" : "checkmark")
                VStack(alignment: .leading, spacing: Spacing.tight) {
                    Text(AgentConversation.askTitle(ask, agent: answer?.agent ?? "Claude")).fontWeight(.medium)
                    // Not beside the whole question or plan drawn below.
                    if !(waiting && (ask.kind == "Question" ? !ask.questionList.isEmpty : ask.kind == "PlanExit" && ask.plan != nil)) {
                        Text(ask.text).foregroundStyle(.secondary).lineLimit(3)
                    }
                }
                Spacer(minLength: Spacing.group)
                if waiting, !canAnswer {
                    Button("Show Terminal", action: showTerminal).identified("native-ask-show-terminal")
                }
            }
            if waiting, ask.kind == "PlanExit", let plan = ask.plan {
                AgentReplyText(text: plan, trailingClearance: 0, streaming: false)
                    .padding(Spacing.group)
                    .surface(.inset, in: .control)
                    .identified("native-ask-plan")
            }
            if waiting, ask.kind == "Question" {
                ForEach(Array(ask.questionList.enumerated()), id: \.offset) { i, question in
                    questionView(i, question)
                }
            }
            if canAnswer { buttons }
            if let held = ask.held ?? sentFor, let issue = answer?.issues[held] {
                Text(issue)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .identified("native-ask-issue")
            }
        }
        .font(.callout)
        .padding(Spacing.group)
        .attentionSurface(in: .card, when: waiting)
        .identified("native-ask")
    }

    @ViewBuilder
    private func questionView(_ i: Int, _ question: AgentRow.Ask.Question) -> some View {
        VStack(alignment: .leading, spacing: Spacing.tight) {
            if !question.header.isEmpty {
                Text(question.header).font(.caption.weight(.medium)).foregroundStyle(.secondary)
            }
            Text(question.question)
            ForEach(Array(question.options.enumerated()), id: \.offset) { o, option in
                let on = picked[i]?.contains(option.label) == true
                Toggle(isOn: Binding(
                    get: { on },
                    set: { _ in picked[i] = AgentConversation.pick(option.label, in: question, picked: picked[i] ?? []) }
                )) {
                    VStack(alignment: .leading, spacing: 0) {
                        Text(option.label)
                        if !option.description.isEmpty {
                            Text(option.description).font(.caption).foregroundStyle(.secondary)
                        }
                    }
                }
                .toggleStyle(.checkbox)
                .disabled(!canAnswer || sending)
                .identified("native-ask-option-\(i)-\(o)")
            }
            if canAnswer {
                TextField("Other", text: Binding(get: { typed[i] ?? "" }, set: { typed[i] = $0 }))
                    .textFieldStyle(.roundedBorder)
                    .frame(maxWidth: 320)
                    .disabled(sending)
                    .identified("native-ask-other-\(i)")
            }
        }
    }

    @ViewBuilder
    private var buttons: some View {
        HStack(spacing: Spacing.group) {
            Spacer(minLength: 0)
            if sending { ProgressView().controlSize(.small) }
            Button("Show Terminal", action: showTerminal).identified("native-ask-show-terminal")
            switch ask.kind {
            case "Question":
                let given = AgentConversation.answers(for: ask.questionList, picked: picked, typed: typed)
                Button("Send Answer") { send(AgentConversation.AnswerOption.answer, given ?? [:]) }
                    .buttonStyle(.borderedProminent)
                    .disabled(given == nil || sending)
                    .identified("native-ask-send-answer")
            case "PlanExit":
                Button("Keep Planning") { send(AgentConversation.AnswerOption.deny) }
                    .disabled(sending)
                    .identified("native-ask-keep-planning")
                Button("Approve Plan") { send(AgentConversation.AnswerOption.allow) }
                    .buttonStyle(.borderedProminent)
                    .disabled(sending)
                    .identified("native-ask-approve")
            default:
                Button("Deny") { send(AgentConversation.AnswerOption.deny) }
                    .disabled(sending)
                    .identified("native-ask-deny")
                Button("Allow") { send(AgentConversation.AnswerOption.allow) }
                    .buttonStyle(.borderedProminent)
                    .disabled(sending)
                    .identified("native-ask-allow")
            }
        }
        .controlSize(.small)
    }

    private func send(_ option: String, _ answers: [String: String] = [:]) {
        sentFor = ask.held
        answer?.send(ask, option, answers)
    }
}
