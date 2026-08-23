import SwiftUI

/// Saying something to an agent, from a wrist.
///
/// The input is a plain watchOS `TextField` and deliberately nothing more.
/// Tapping it hands over to the system's input screen, which is dictation,
/// Scribble, the emoji grid and — when a phone is nearby — the phone's own
/// keyboard, all of it localized and all of it something the wearer already
/// knows how to use. Anything written here would be one of those, and worse.
///
/// The rest of this file is about the seconds after Send, which are the part
/// that actually goes wrong. `sendMessage` has to wake a phone that may be
/// asleep in a pocket, which then does an SSH round trip to the runner:
/// `WatchLinkHost` budgets eight seconds for the connection and eight more for
/// the call, and it spends them. **A button that looks inert for that long is a
/// button somebody presses again**, and the second press is a second prompt to a
/// live agent — real tokens, real work, and a conversation nobody meant to have.
/// So there is an in-flight state, the button is off while it is on, and the
/// screen does not accept another Send until the phone has answered one way or
/// the other.
///
/// **What was typed is never thrown away on failure.** Dictating on a moving
/// wrist is the expensive part of this screen; losing a sentence to a phone that
/// was briefly out of range would make the retry cost more than the first
/// attempt did.
struct ComposeView<Client: FleetClient>: View {
    @ObservedObject var client: Client
    let terminal: String

    @Environment(\.dismiss) private var dismiss

    @State private var text = ""
    @State private var phase = Phase.writing

    /// The four things this screen can be doing.
    ///
    /// `sending` and `failed` are separate cases rather than two booleans
    /// because the pair that cannot both be true is the pair that eventually
    /// both are: a spinner left up beside an error is how a screen ends up
    /// looking busy and broken at once.
    private enum Phase: Equatable {
        case writing
        case sending
        case sent
        case failed(String)
    }

    /// The agent this is going to, as of right now. Nil once it has left the
    /// fleet, which costs the header its name and nothing else.
    private var agent: FleetSnapshot.Agent? {
        client.state.snapshot?.agents.first { $0.id == terminal }
    }

    private var message: String {
        text.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    /// Empty is not a prompt.
    ///
    /// Whitespace counts as empty, which matters more here than on a keyboard:
    /// dictation that picked up nothing returns an empty string, and a tap on a
    /// live Send would deliver a blank turn to an agent that then has to answer
    /// it.
    private var canSend: Bool {
        !message.isEmpty && phase != .sending && client.state.canAct
    }

    var body: some View {
        Group {
            if case .sent = phase {
                Confirmation(
                    title: "Sent",
                    // Exactly what `WatchReply.sent` promises and not a word
                    // more. The phone handed the prompt on through the path its
                    // own composer uses; whether the agent has read it, or will,
                    // is not something this watch was told.
                    detail: "Your iPhone passed it on.",
                    done: { dismiss() })
            } else {
                form
            }
        }
        .navigationTitle("Reply")
    }

    private var form: some View {
        List {
            Section {
                // One line, and no `axis: .vertical` — that was tried on a 46mm
                // simulator and does nothing at all. A watchOS `TextField` is
                // not an editable field; it is a button that opens the system
                // input screen, and the row only ever shows a single truncated
                // summary of what is in it however it is configured. Proofread-
                // ing happens on the input screen, where the whole message is
                // visible and where a second tap reopens it to edit. Anybody
                // adding a wrapping modifier here is about to spend a build
                // cycle finding that out again.
                TextField("Message", text: $text)
                    .disabled(phase == .sending)
            } header: {
                // Which agent this is going to, said on the screen that sends
                // it. The nav title here is "Reply" and the agent's name is one
                // screen back behind a chevron — and a prompt delivered to the
                // wrong agent costs exactly what a duplicate one does: real
                // tokens, real work, and a conversation nobody meant to have.
                if let agent { Text("To \(agent.label)") }
            } footer: {
                // ABOVE the Send button and not under it, which is where this
                // was until a 46mm simulator showed the red sentence starting
                // at the last row of pixels and wrapping off the bottom of the
                // screen. A failure somebody has to scroll to find is a failure
                // they will read as "nothing happened", and this one is the
                // difference between a prompt that landed and one that did not.
                note
            }

            Section {
                Button(action: send) {
                    if phase == .sending {
                        // The label changes, not just the enabled state. A
                        // dimmed button still reads as a button; a spinner with
                        // "Sending…" beside it is the only version of this that
                        // says why nothing has happened yet.
                        //
                        // A `Label` and not an `HStack`, which was tried and
                        // measured on a simulator. A bare `ProgressView` takes
                        // every point of width it is offered, so an `HStack` put
                        // the spinner near the left edge and the word near the
                        // right and read as two unrelated things — and a
                        // `Spacer` does not fix it, because the greedy view is
                        // the spinner. `Label`'s icon slot is fixed width, so
                        // this lands exactly where the paperplane below does.
                        Label { Text("Sending…") } icon: { ProgressView() }
                    } else {
                        Label("Send", systemImage: "paperplane.fill")
                    }
                }
                .disabled(!canSend)
            }
        }
    }

    @ViewBuilder private var note: some View {
        if case let .failed(reason) = phase {
            // The phone's own sentence, verbatim. Every one of them is written
            // for a person and none of them carries a core error —
            // `WatchLinkHost.reason` is the whole point of that. Rewording it
            // here would mean this file having an opinion about a failure it
            // did not observe.
            Label(reason, systemImage: "exclamationmark.triangle.fill")
                .font(.caption2)
                .foregroundStyle(.red)
        } else if !client.state.canAct {
            // Reaching this screen with an unreachable phone should not be
            // possible — `AgentDetailView` disables the link — but the link can
            // be tapped a moment before the phone leaves range, and then this is
            // the screen somebody is looking at.
            Text("Can’t reach your iPhone, so this can’t be sent until it’s nearby.")
                .font(.caption2)
        }
    }

    private func send() {
        // Re-checked here and not only in `canSend`. `canSend` is what draws the
        // button; this is what runs when it is pressed, and on a watch the two
        // are far enough apart in time for a second tap to land while the first
        // is still in flight.
        guard canSend else { return }
        let text = message
        phase = .sending
        Task {
            // `attempt`, never `send`: see `FleetClient.attempt`.
            switch await client.attempt(.prompt(terminal: terminal, text: text)) {
            case .sent:
                phase = .sent
            case let .failed(reason):
                phase = .failed(reason)
            case .permission, .transcript:
                // A permission or a transcript in answer to a prompt is a phone
                // speaking a dialect this build does not know. Reported as a
                // failure rather than as a success, because nothing here can
                // confirm the prompt was carried — and the wording is
                // `WatchLinkClient`'s own for the same condition.
                phase = .failed(WatchLinkClient.unreadableReply)
            }
        }
    }
}

/// The screen after something worked.
///
/// A full-screen state rather than a checkmark next to a still-live Send button,
/// so there is nothing left to press twice. Shared by both action screens
/// because "the phone took it" is one fact with one shape, and two confirmations
/// that looked different would suggest the two actions succeeded differently.
struct Confirmation: View {
    let title: String
    let detail: String
    let done: () -> Void

    var body: some View {
        ScrollView {
            VStack(spacing: 8) {
                Image(systemName: "checkmark.circle.fill")
                    .font(.title2)
                    // Green, and it is the only green on the watch. Orange is
                    // spoken for — it means an agent is waiting on a person, on
                    // every surface — so success cannot borrow it without making
                    // "done" and "needs you" the same color at a glance.
                    .foregroundStyle(.green)
                Text(title)
                    .font(.headline)
                Text(detail)
                    .font(.caption2)
                    .foregroundStyle(.secondary)
                    .multilineTextAlignment(.center)
                Button("Done", action: done)
                    .padding(.top, 4)
            }
            .frame(maxWidth: .infinity)
            .padding(.top, 8)
        }
    }
}

#if DEBUG
    #Preview("Compose") {
        NavigationStack {
            ComposeView(client: PreviewFleetClient(.live(PreviewFleet.snapshot)), terminal: "a")
        }
    }

    #Preview("Compose, unreachable") {
        NavigationStack {
            ComposeView(client: PreviewFleetClient(.cached(PreviewFleet.snapshot)), terminal: "a")
        }
    }

    #Preview("Compose, failed") {
        NavigationStack {
            ComposeView(
                client: PreviewFleetClient(
                    .live(PreviewFleet.snapshot),
                    reply: { _ in .failed("Open Far Cooler on your iPhone, then try again.") }),
                terminal: "a")
        }
    }
#endif
