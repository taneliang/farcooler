import SwiftUI

/// What a row's buttons need from its pane (ov-370): where an answer goes,
/// and how the last one went.
struct NativeAskAnswer {
    /// The agent asking, as the row's title names it.
    var agent = "Claude"
    var answering: String?
    var issues: [String: String]
    let send: (_ ask: AgentRow.Ask, _ option: String, _ answers: [String: String]) -> Void
}

extension NativePaneModel {
    /// The rows' answering, where the runner takes answers.
    var nativeAskAnswer: NativeAskAnswer? {
        guard answers != nil else { return nil }
        return NativeAskAnswer(agent: agent, answering: answering, issues: answerIssues) { [weak self] ask, option, answers in
            Task { await self?.answer(ask, option: option, answers: answers) }
        }
    }
}

/// Claude asking: a permission, a question, or a plan to approve (ov-370),
/// as the Mac's `NativeAskRow` draws it, laid out for a phone.
///
/// While the runner's hook holds the ask, the row answers it: Allow or Deny;
/// the question's options, then Send Answer; or Approve Plan or Keep
/// Planning. The answer goes to the hook, never as keys into claude's dialog,
/// and the first from any device wins. Once the hold ends, only the terminal
/// can answer, and the row offers Show Terminal.
struct NativeAskRow: View {
    let ask: AgentRow.Ask
    let answer: NativeAskAnswer?
    let showTerminal: () -> Void
    @State private var picked: [Int: Set<String>] = [:]
    @State private var typed: [Int: String] = [:]
    /// The hold this row last answered, so what became of the answer is
    /// still said once the hold ends (review 1 M1).
    @State private var sentFor: String?

    private var waiting: Bool { !ask.answered && ask.answeredBy == nil }
    private var canAnswer: Bool { answer != nil && AgentConversation.answerable(ask) }
    private var sending: Bool { ask.held != nil && answer?.answering == ask.held }

    var body: some View {
        VStack(alignment: .leading, spacing: Spacing.group) {
            HStack(alignment: .firstTextBaseline, spacing: Spacing.group) {
                Image(systemName: waiting ? "questionmark.bubble" : "checkmark")
                Text(AgentConversation.askTitle(ask, agent: answer?.agent ?? "Claude")).fontWeight(.medium)
            }
            if !(waiting && (ask.kind == "Question" ? !ask.questionList.isEmpty : ask.kind == "PlanExit" && ask.plan != nil)) {
                Text(ask.text).foregroundStyle(.secondary).lineLimit(4)
            }
            if waiting, ask.kind == "PlanExit", let plan = ask.plan {
                AgentReplyText(text: plan, trailingClearance: 0, streaming: false)
                    .padding(Spacing.group)
                    .surface(.inset, in: .control)
                    .accessibilityElement(children: .contain)
                    .accessibilityIdentifier("native-ask-plan")
            }
            if waiting, ask.kind == "Question" {
                ForEach(Array(ask.questionList.enumerated()), id: \.offset) { i, question in
                    questionView(i, question)
                }
            }
            if canAnswer {
                buttons
            } else if waiting {
                Button("Show Terminal", action: showTerminal)
                    .buttonStyle(.bordered)
                    .accessibilityIdentifier("native-ask-show-terminal")
            }
            if let held = ask.held ?? sentFor, let issue = answer?.issues[held] {
                Text(issue)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .accessibilityIdentifier("native-ask-issue")
            }
        }
        .font(.callout)
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(Spacing.inset)
        .attentionSurface(in: .card, when: waiting)
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
                Button {
                    picked[i] = AgentConversation.pick(option.label, in: question, picked: picked[i] ?? [])
                } label: {
                    HStack(alignment: .firstTextBaseline, spacing: Spacing.group) {
                        Image(systemName: question.multiSelect
                            ? (on ? "checkmark.square.fill" : "square") : (on ? "largecircle.fill.circle" : "circle"))
                            // The control's own tint for what's picked, as a system radio draws it.
                            .foregroundStyle(on ? AnyShapeStyle(.tint) : AnyShapeStyle(.secondary))
                        VStack(alignment: .leading, spacing: 0) {
                            Text(option.label).foregroundStyle(.primary)
                            if !option.description.isEmpty {
                                Text(option.description).font(.caption).foregroundStyle(.secondary)
                            }
                        }
                        Spacer(minLength: 0)
                    }
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .disabled(!canAnswer || sending)
                .accessibilityAddTraits(on ? .isSelected : [])
                .accessibilityIdentifier("native-ask-option-\(i)-\(o)")
            }
            if canAnswer {
                TextField("Other", text: Binding(get: { typed[i] ?? "" }, set: { typed[i] = $0 }))
                    .textFieldStyle(.roundedBorder)
                    .disabled(sending)
                    .accessibilityIdentifier("native-ask-other-\(i)")
            }
        }
    }

    @ViewBuilder
    private var buttons: some View {
        // Two rows of buttons rather than one too wide for the phone.
        VStack(alignment: .leading, spacing: Spacing.group) {
            HStack(spacing: Spacing.group) {
                switch ask.kind {
                case "Question":
                    let given = AgentConversation.answers(for: ask.questionList, picked: picked, typed: typed)
                    Button("Send Answer") { send(AgentConversation.AnswerOption.answer, given ?? [:]) }
                        .buttonStyle(.borderedProminent)
                        .disabled(given == nil || sending)
                        .accessibilityIdentifier("native-ask-send-answer")
                case "PlanExit":
                    Button("Approve Plan") { send(AgentConversation.AnswerOption.allow) }
                        .buttonStyle(.borderedProminent)
                        .disabled(sending)
                        .accessibilityIdentifier("native-ask-approve")
                    Button("Keep Planning") { send(AgentConversation.AnswerOption.deny) }
                        .buttonStyle(.bordered)
                        .disabled(sending)
                        .accessibilityIdentifier("native-ask-keep-planning")
                default:
                    Button("Allow") { send(AgentConversation.AnswerOption.allow) }
                        .buttonStyle(.borderedProminent)
                        .disabled(sending)
                        .accessibilityIdentifier("native-ask-allow")
                    Button("Deny") { send(AgentConversation.AnswerOption.deny) }
                        .buttonStyle(.bordered)
                        .disabled(sending)
                        .accessibilityIdentifier("native-ask-deny")
                }
                if sending { ProgressView() }
            }
            Button("Show Terminal", action: showTerminal)
                .buttonStyle(.bordered)
                .accessibilityIdentifier("native-ask-show-terminal")
        }
    }

    private func send(_ option: String, _ answers: [String: String] = [:]) {
        sentFor = ask.held
        answer?.send(ask, option, answers)
    }
}
