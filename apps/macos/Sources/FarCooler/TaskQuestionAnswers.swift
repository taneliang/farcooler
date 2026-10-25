import AgentKit
import SwiftUI

/// A question waiting on the person reading, and the ways to answer it:
/// `TaskCard.offer`'s, drawn and nothing decided here.
struct QuestionAnswers: View {
    @Environment(\.colorScheme) private var scheme

    let offer: TaskCard.Offer
    let onAnswer: (String) async -> Bool
    let draft: TaskCard.Draft
    /// Who the decision is for, said above its answers (ov-737).
    var whose: String?
    /// Asked before an answer goes to someone else's decision; nil answers at once.
    var answerForOthersPrompt: String?
    /// An answer held until the person confirms it.
    @State private var confirming: String?

    /// The answer on its way, which shows a spinner in place of the buttons.
    @State private var sending: String?
    @State private var failed = false
    @State private var writing = false
    @State private var typed = ""
    /// The option chosen in the radio list, not yet sent.
    @State private var picked: String?
    @Environment(\.taskKeyLinker) private var linker

    var body: some View {
        VStack(alignment: .leading, spacing: ColumnGrid.rhythm) {
            Text("Question")
                .font(TaskTypography.label)
                .foregroundStyle(Color.accentColor)
            // Markdown, as the intent and notes are: an ask is written with
            // short paragraphs and lists, and one run of text ran them
            // together. Task keys still link (`MarkdownText` reads the linker).
            MarkdownText(text: offer.question.body, spacing: .document)
                .frame(maxWidth: TaskTypography.measure, alignment: .leading)
                .accessibilityIdentifier("task-card-question-body")
            if let whose {
                Label(whose, systemImage: "person")
                    .font(TaskTypography.meta)
                    .foregroundStyle(.quietInk)
                    .fixedSize(horizontal: false, vertical: true)
                    .accessibilityIdentifier("task-card-whose")
            }
            if sending != nil {
                HStack(spacing: 6) {
                    ProgressView().controlSize(.small)
                    Text("Answering…")
                        .font(TaskTypography.meta)
                        .foregroundStyle(.quietInk)
                }
            } else if offer.typed {
                freeAnswer
            } else if !offer.options.isEmpty {
                optionList
            }
            if failed {
                Text("Your answer wasn’t sent. Try again.")
                    .font(TaskTypography.meta)
                    .foregroundStyle(.quietInk)
            }
        }
        .padding(1.5 * ColumnGrid.rhythm)
        .frame(maxWidth: .infinity, alignment: .leading)
        // Amber, as every "needs you" is; the accent is for the answering buttons.
        .attentionSurface(in: .card)
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("task-card-question")
        .confirmationDialog(
            answerForOthersPrompt ?? "", isPresented: Binding(get: { confirming != nil }, set: { if !$0 { confirming = nil } }),
            titleVisibility: .visible
        ) {
            Button("Answer") {
                if let answer = confirming { send(answer) }
                confirming = nil
            }
            Button("Cancel", role: .cancel) { confirming = nil }
        }
        // The field comes back open, with what was in it, whenever this view
        // is made again: the draft is the store's.
        .onAppear {
            typed = draft.read(offer.question)
            if !typed.isEmpty { writing = true }
        }
        .onChange(of: typed) { _, text in draft.write(offer.question, text) }
    }

    /// The options as a radio list, each on its own row with its whole text
    /// wrapping, and Answer under it, off until one is chosen. Choosing sends
    /// nothing; Answer sends the chosen option's text, as the buttons did.
    private var optionList: some View {
        VStack(alignment: .leading, spacing: ColumnGrid.rhythm) {
            VStack(alignment: .leading, spacing: ColumnGrid.rhythm / 2) {
                ForEach(Array(offer.options.enumerated()), id: \.offset) { index, option in
                    let chosen = picked == option
                    Button {
                        picked = option
                    } label: {
                        HStack(alignment: .firstTextBaseline, spacing: 8) {
                            Image(systemName: chosen ? "largecircle.fill.circle" : "circle")
                                .foregroundStyle(chosen ? Color.accentColor : Color.quietInk)
                                .accessibilityHidden(true)
                            Text(option)
                                .font(TaskTypography.body)
                                .foregroundStyle(.primary)
                                .multilineTextAlignment(.leading)
                                .fixedSize(horizontal: false, vertical: true)
                                .frame(maxWidth: .infinity, alignment: .leading)
                        }
                        .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                    .accessibilityAddTraits(chosen ? .isSelected : [])
                    .identified("task-card-option-\(index)")
                }
            }
            .accessibilityElement(children: .contain)
            Button("Answer") {
                if let picked { request(picked) }
            }
            .disabled(picked == nil)
            .identified("task-card-answer")
        }
    }

    @ViewBuilder private var freeAnswer: some View {
        if writing {
            HStack(spacing: 8) {
                TextField("Your answer", text: $typed)
                    .textFieldStyle(.roundedBorder)
                    .onSubmit { sendTyped() }
                Button("Send") { sendTyped() }
                    .disabled(typed.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
            }
        } else {
            Button("Answer…") { writing = true }
        }
    }

    private func sendTyped() {
        let text = typed.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty else { return }
        request(text)
    }

    /// Send `answer`, asking first when the decision is someone else's (ov-737).
    private func request(_ answer: String) {
        if answerForOthersPrompt == nil { send(answer) } else { confirming = answer }
    }

    private func send(_ answer: String) {
        guard sending == nil else { return }
        sending = answer
        failed = false
        Task {
            let written = await onAnswer(answer)
            sending = nil
            failed = !written
            if written {
                typed = ""
                writing = false
            }
        }
    }
}
