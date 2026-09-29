import SwiftUI

/// Answering an ask from the watch's Needs You section.
///
/// `PermissionView`'s job, from the other end. That screen opens on an agent
/// and has to ask the phone what it's waiting on, because an agent row carries
/// no request id. An item carries its own: `askID` and the ask's options came
/// with the snapshot, so this screen asks nothing and draws them at once.
///
/// **The buttons are the item's actions**, in its order and under its titles
/// (`WatchItemAction.answer`), which are the agent's own words as the runner
/// sent them. Each sends `WatchRequest.answer`, the path `PermissionView` and
/// a pane's own card take, which refuses a stale or second answer.
///
/// The item is re-read from the current snapshot on every render, by key, for
/// `WatchRoute.agent`'s reason: an ask answered on the Mac or the phone leaves
/// the list, and this screen says so rather than going on offering it.
struct NeedsYouItemView<Client: FleetClient>: View {
    @ObservedObject var client: Client
    let key: String

    @Environment(\.dismiss) private var dismiss

    /// The option in flight, if one is. Everything goes off while it is: two
    /// answers to one ask is one of them landing on an ask already settled.
    @State private var answering: NeedsYouAction?
    /// Why the last answer didn't go through. Cleared on the next attempt.
    @State private var failure: String?
    /// The option that went through.
    @State private var answered: NeedsYouAction?

    private var item: NeedsYouItem? {
        client.state.snapshot?.needsYou?.first { $0.key == key }
    }

    var body: some View {
        Group {
            if let answered {
                Confirmation(
                    title: "Answered",
                    detail: "Your iPhone sent “\(answered.title)”.",
                    done: { dismiss() })
            } else if let item, let snapshot = client.state.snapshot,
                case let .answer(options) = WatchItemAction.of(item, in: snapshot)
            {
                pending(item, options)
            } else {
                gone
            }
        }
        .navigationTitle("Answer")
    }

    private func pending(_ item: NeedsYouItem, _ options: [NeedsYouAction]) -> some View {
        List {
            Section {
                // The runner's order, untouched, as `PermissionView` keeps the
                // agent's: order decides what's read first on a list this
                // narrow, and re-ranking the answers would be rewriting the
                // question.
                ForEach(options, id: \.id) { option in
                    Button {
                        answer(item, option)
                    } label: {
                        HStack(alignment: .firstTextBaseline, spacing: 6) {
                            // Never truncated, for `PermissionView`'s reason.
                            Text(option.title)
                                .multilineTextAlignment(.leading)
                                .frame(maxWidth: .infinity, alignment: .leading)
                            if answering == option { ProgressView().fixedSize() }
                        }
                    }
                    .buttonStyle(.bordered)
                    .tint(option.destructive ? .red : nil)
                    .disabled(answering != nil || !client.state.canAct)
                }
            } header: {
                VStack(alignment: .leading, spacing: 2) {
                    if !item.watchPlace.isEmpty { Text(item.watchPlace) }
                    note
                }
            }
            if let detail = item.detail, !detail.isEmpty {
                Section {
                    Text(detail)
                        .font(.caption2.monospaced())
                        .foregroundStyle(.secondary)
                }
            }
        }
    }

    /// The header's second line, swapped and never appended, for
    /// `PermissionView.note`'s reason: the options stay where they are
    /// whatever it says.
    @ViewBuilder private var note: some View {
        if let failure {
            Label(failure, systemImage: "exclamationmark.triangle.fill")
                .foregroundStyle(.red)
        } else if !client.state.canAct {
            Text("Can’t reach your iPhone, so these are off until it’s nearby.")
        } else {
            // `darkColor`: watchOS has no light appearance. See `PermissionView`.
            Text("Needs your approval")
                .foregroundStyle(GlancePalette.amber.darkColor)
        }
    }

    /// The ask left the list: answered somewhere else, or settled.
    private var gone: some View {
        VStack(alignment: .leading, spacing: 4) {
            Text("Nothing to Answer")
                .font(.headline)
            Text("This was answered, or it isn’t waiting anymore.")
                .font(.caption2)
                .foregroundStyle(.secondary)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(.horizontal, 8)
    }

    private func answer(_ item: NeedsYouItem, _ option: NeedsYouAction) {
        // Re-checked at the tap, not only at the render that drew the button.
        guard answering == nil, client.state.canAct,
            let request = item.watchRequest(answering: option)
        else { return }
        answering = option
        failure = nil
        Task {
            // `attempt`, never `send`: see `FleetClient.attempt`.
            let reply = await client.attempt(request)
            answering = nil
            switch reply {
            case .sent:
                answered = option
            case let .failed(reason):
                // Verbatim: `WatchLinkHost` already made it a sentence, and
                // the options stay live for a retry.
                failure = reason
            case .permission, .transcript:
                failure = WatchLinkClient.unreadableReply
            }
        }
    }
}
