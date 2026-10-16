import SwiftUI

// Replying to a review thread an agent escalated (ov-317): a sheet to write the
// reply, then a confirmation that names the pull request and the GitHub login it
// will be posted as, which the runner reads from its own `gh` before anything is
// sent (`pr.thread_reply` with `send` false). The reply is the person's own text;
// the runner posts it, and an agent never sees a button here.

/// What a PR_THREAD row can do about its thread, on a runner that allows it.
struct ThreadReplies {
    let task: String
    /// Where the reply would go and as whom, posting nothing.
    let preview: () async -> Result<PhonePrActions.ReplyPreview, PrActionFailure>
    /// Post it, to the thread the preview showed. The sentence for a refusal, nil
    /// when it was posted.
    let send: (String, PhonePrActions.ReplyPreview) async -> String?
}

struct ThreadReplySheet: View {
    let item: NeedsYouItem
    let replies: ThreadReplies
    @Environment(\.dismiss) private var dismiss

    @State private var draft = ""
    @State private var checking = false
    @State private var sending = false
    @State private var failure: String?
    @State private var confirming: PhonePrActions.ReplyPreview?
    @FocusState private var focused: Bool

    /// The reply as it would go, nil while it is empty or past the cap.
    private var body_: String? { PhonePrActions.reply(draft) }

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    Text(item.question)
                        .font(.subheadline)
                        .foregroundStyle(.secondary)
                        .accessibilityIdentifier("pr-reply-question")
                } header: {
                    Text("The Thread")
                }
                Section {
                    TextEditor(text: $draft)
                        .frame(minHeight: 140)
                        .focused($focused)
                        .accessibilityIdentifier("pr-reply-text")
                } header: {
                    Text("Your Reply")
                } footer: {
                    Text(footer)
                }
                if let failure {
                    Section {
                        Text(failure)
                            .font(.footnote)
                            .accessibilityIdentifier("pr-reply-failure")
                    }
                }
            }
            .navigationTitle("Reply")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { dismiss() }
                        .accessibilityIdentifier("pr-reply-cancel")
                }
                ToolbarItem(placement: .confirmationAction) {
                    if checking || sending {
                        ProgressView()
                    } else {
                        Button("Continue", action: check)
                            .disabled(body_ == nil)
                            .accessibilityIdentifier("pr-reply-continue")
                    }
                }
            }
            .onAppear { focused = true }
            .alert(
                confirming?.title ?? "", isPresented: Binding(get: { confirming != nil }, set: { if !$0 { confirming = nil } }),
                presenting: confirming
            ) { shown in
                Button(PhonePrActions.sendReply) { post(shown) }
                    .accessibilityIdentifier("pr-reply-send")
                Button("Cancel", role: .cancel) {}
            } message: { shown in
                Text("\(shown.message)\n\n\u{201C}\(Self.excerpt(body_ ?? ""))\u{201D}")
            }
        }
        .accessibilityIdentifier("pr-reply-sheet")
        .interactiveDismissDisabled(sending)
    }

    /// Past the cap it says so; near it, how much is left; otherwise nothing but
    /// where the text goes.
    private var footer: String {
        let used = draft.trimmingCharacters(in: .whitespacesAndNewlines).unicodeScalars.count
        if used > PhonePrActions.replyMax { return "That’s too long to post from here. Shorten it, or reply on GitHub." }
        return "The runner posts it with its GitHub login. You’ll see which before it’s sent."
    }

    /// A long reply is cut in the confirmation; the whole of it is sent.
    static func excerpt(_ text: String) -> String {
        text.count <= 280 ? text : String(text.prefix(279)) + "\u{2026}"
    }

    private func check() {
        guard !checking, !sending else { return }
        checking = true
        failure = nil
        Task {
            switch await replies.preview() {
            case .success(let shown): confirming = shown
            case .failure(let refused): failure = refused.sentence
            }
            checking = false
        }
    }

    private func post(_ shown: PhonePrActions.ReplyPreview) {
        guard let text = body_, !sending else { return }
        sending = true
        Task {
            let refused = await replies.send(text, shown)
            sending = false
            if let refused { failure = refused } else { dismiss() }
        }
    }
}
