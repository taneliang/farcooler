import SwiftUI

/// Answering what an agent is waiting on, from a wrist.
///
/// The screen the whole feature is for. It asks the phone `pendingPermission`
/// when it opens — the fleet snapshot carries a headline, not a request id, so
/// this question cannot be answered from anything the watch already holds — and
/// then renders whatever came back.
///
/// **The buttons say what the agent said.** `WatchPermissionOption.name` is the
/// label, always. There is no hardcoded Allow/Deny pair here and there is none
/// anywhere upstream either: the daemon copies the agent's own wording through
/// (`Allow Bash(cargo test)`, `Allow for this session`, `No, and tell Claude
/// what to do differently`), and a watch that replaced that with two words of
/// its own would be answering a question the agent did not ask. `kind` is read
/// for emphasis and for nothing else — see `plainYes`.
///
/// **`.permission(nil)` and `.failed` are the two answers that must never look
/// alike.** They mean opposite things:
///
///   - `nil` is *the replay finished and there is nothing pending*. The agent
///     may still be blocked — on a trust gate, or a plain question — so this
///     screen says so and keeps the signal line on screen, because that line is
///     the only thing that tells the person what it actually is.
///   - `failed` is *we could not find out*. It is drawn in red, it says the
///     agent may still be waiting, and it offers a retry.
///
/// Collapsing the two is the worst bug this screen can carry: telling somebody
/// their agent is not waiting, when it is, is how they put their wrist down and
/// leave a blocked agent sitting for an hour.
///
/// **`toolCall` is deliberately not on screen.** The name reads like a
/// description and is not one: every producer fills it with an opaque
/// correlation id — `tool_use_id` in `claude/normalize.rs`, `itemId` in
/// `codex/normalize.rs`, `toolCall.toolCallId` in `acp/session.rs` — and each of
/// them defaults it to the empty string when the agent left it out. The phone
/// only ever compares it to a `ToolRow.id` and never draws it. Putting it on a
/// watch face would print `toolu_01…` where a sentence belongs, or nothing at
/// all. What the agent wants to do is already in the option names and in the
/// host's signal line, both of which are written for a person.
struct PermissionView<Client: FleetClient>: View {
    @ObservedObject var client: Client
    let terminal: String

    @Environment(\.dismiss) private var dismiss

    @State private var phase = Phase.asking
    /// The option whose answer is in flight, if one is.
    ///
    /// Kept apart from `phase` because the permission stays on screen while it
    /// runs: the person needs to see which of three buttons they pressed, and a
    /// phase that replaced the list with a spinner would take that away at the
    /// one moment it is being looked for.
    @State private var answering: WatchPermissionOption?
    /// Why the last answer did not go through. Cleared on the next attempt.
    @State private var answerFailure: String?

    private enum Phase {
        /// The round trip that runs when this screen opens.
        case asking
        case pending(WatchPermission)
        /// `.permission(nil)`: asked, answered, nothing pending.
        case nothing
        /// `.failed`: we do not know what is pending.
        case failed(String)
        case answered(WatchPermissionOption)
    }

    private var agent: FleetSnapshot.Agent? {
        client.state.snapshot?.agents.first { $0.id == terminal }
    }

    var body: some View {
        Group {
            switch phase {
            case .asking:
                asking
            case let .pending(permission):
                pending(permission)
            case .nothing:
                nothing
            case let .failed(reason):
                failed(reason)
            case let .answered(option):
                Confirmation(
                    title: "Answered",
                    // The option's own name, quoted, because "Answered" alone
                    // does not say which of three buttons landed — and on a
                    // wrist the tap and the confirmation are far enough apart in
                    // time to doubt it.
                    detail: "Your iPhone sent “\(option.name)”.",
                    done: { dismiss() })
            }
        }
        .navigationTitle("Answer")
        // Once per appearance, and canceled if the wrist drops mid-flight.
        // Nothing polls: the phone replays the agent's stream to answer this,
        // which is a real round trip on the runner, and asking it on a timer
        // would spend somebody's battery re-reading a conversation that has not
        // changed.
        .task { await ask() }
    }

    // MARK: - The five things this can be showing

    private var asking: some View {
        VStack(spacing: 10) {
            // `fixedSize`, because a bare `ProgressView` takes every point it is
            // offered — in a full-height `VStack` that pushed the sentence to
            // the very bottom of the screen and left the spinner floating alone
            // in the middle, which reads as two separate things rather than one
            // labeled wait.
            ProgressView()
                .fixedSize()
            // Named rather than left as a bare spinner. This can take the ten
            // seconds `WatchLinkHost.replayBudget` allows, and a spinner with no
            // words is indistinguishable from a screen that has hung.
            Text("Checking with your iPhone…")
                .font(.caption2)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
        }
        .padding(.horizontal, 8)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }

    @ViewBuilder private func pending(_ permission: WatchPermission) -> some View {
        List {
            Section {
                if permission.options.isEmpty {
                    // The wire allows it: `acp/session.rs` defaults the options
                    // array to empty when the agent sent none. A section with a
                    // header and no buttons under it reads as a screen that
                    // failed to draw, so it says what happened instead.
                    Text("This agent didn’t offer any answers. Open it on your iPhone.")
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                } else {
                    // The agent's order, untouched. `WatchPermission.options` is
                    // documented as "in the order it offered them" and this is
                    // the surface that has to honor it: every option here is a
                    // full-width row in a scrolling list, so order decides what
                    // is read first. The phone reorders because it packs a yes
                    // and a no into one horizontal row and cannot fit the rest;
                    // there is no such row here, and re-ranking somebody's
                    // answers to fit a layout that does not exist would be
                    // rewriting the question.
                    ForEach(permission.options) { option in
                        OptionButton(
                            option: option,
                            emphasized: option.id == plainYes(permission)?.id,
                            inFlight: answering?.id == option.id,
                            // Everything goes off while one answer is in flight.
                            // Two answers to one permission is one of them
                            // landing against a request id the daemon has
                            // already retired, and no way to tell which.
                            enabled: answering == nil && client.state.canAct,
                            choose: { answer(permission, option) })
                    }
                }
            } header: {
                // Two short lines and NOT the full `Signal` block this screen
                // draws when nothing is pending. That block was here, and a 46mm
                // simulator showed what it cost: the emphasized option filled
                // what was left of the screen and the reject fell below the
                // fold. A permission screen whose yes is visible and whose no is
                // not is a screen arguing for yes, which is the one thing an
                // approval must never do. The agent's status is also the least
                // useful thing here — the phone has just been asked what this
                // agent is actually blocked on and answered, so a snapshot's
                // opinion about it adds nothing that the options do not say
                // better.
                //
                // **The second line is swapped, never appended.** A failed
                // answer or an unreachable phone used to add a whole `Section`
                // ABOVE this one — moved there from a footer below the buttons
                // for the reason the footer failed: a sentence below three
                // full-width options is a sentence somebody who just tapped one
                // of them will not see. But an extra Section on TOP of this
                // header pushes the very options it is reporting on further
                // down the screen, and the last of them — typically the reject
                // — is exactly the row that residual push costs the most. This
                // header is always exactly two lines, whatever it says, so
                // swapping its second line for the failure or the reachability
                // note costs the options nothing: they sit at the same offset
                // whether this line reads "Needs your approval" or something
                // went wrong.
                VStack(alignment: .leading, spacing: 2) {
                    // Which agent, because the nav title says "Answer" and its
                    // name is a screen back behind a chevron.
                    if let agent { Text(agent.label) }
                    note
                }
            }
        }
    }

    /// The header's second line: what went wrong, if anything did, or the
    /// phone's own words for why the options are grayed out — and otherwise
    /// the plain statement of what this screen is asking.
    @ViewBuilder private var note: some View {
        if let answerFailure {
            // Verbatim. `WatchLinkHost.reason` already turned whatever went
            // wrong into a sentence for a person, and the permission is still on
            // screen below with its buttons live again — the host clears its
            // replay cache only on success, precisely so this retry works.
            Label(answerFailure, systemImage: "exclamationmark.triangle.fill")
                .foregroundStyle(.red)
        } else if !client.state.canAct {
            Text("Can’t reach your iPhone, so these are off until it’s nearby.")
        } else {
            // The phone's words, and the phone's color. Orange means an agent
            // is waiting on a person, on every surface this app has — so this
            // is the one place on the watch that spends it.
            // `darkColor` rather than the scheme-resolved value: watchOS has
            // no light appearance at all, so §01's pale-backdrop palette
            // answers a question this platform never asks.
            Text("Needs your approval")
                .foregroundStyle(GlancePalette.amber.darkColor)
        }
    }

    /// Asked, answered, and there is nothing to answer.
    ///
    /// Not an error, and not drawn as one. What it must not do is stop there:
    /// an agent can be blocked on something this vocabulary has no word for, so
    /// the signal line stays on screen and the sentence names the possibility
    /// rather than implying the agent is fine.
    private var nothing: some View {
        List {
            Section {
                VStack(alignment: .leading, spacing: 4) {
                    Text("Nothing to Answer")
                        .font(.headline)
                    Text("This agent isn’t waiting on a permission right now.")
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                }
            } footer: {
                // Independent of `agent` on purpose. This is a claim about the
                // vocabulary, not about this agent specifically — "blocked on
                // something this watch has no word for" is true whether or not
                // the agent is still in the snapshot the watch is holding — so
                // it must not vanish along with `Signal` just because the agent
                // it was asked about has since left the fleet.
                Text("If it’s stuck on something else — a trust prompt, or a question — "
                    + "open it on your iPhone.")
                    .font(.caption2)
            }
            if let agent, let snapshot = client.state.snapshot {
                Section {
                    Signal(agent: agent, snapshot: snapshot)
                }
            }
        }
    }

    /// We could not find out, which is not the same as nothing being pending.
    ///
    /// Red, a warning mark, and a sentence saying the agent may still be
    /// waiting: three signals that this is a problem, against a "Nothing to
    /// Answer" that carries none of them. Being able to tell these two apart at
    /// a glance is the whole reason they are drawn differently at all.
    private func failed(_ reason: String) -> some View {
        List {
            Section {
                VStack(alignment: .leading, spacing: 4) {
                    Label("Couldn’t Check", systemImage: "exclamationmark.triangle.fill")
                        .font(.headline)
                        .foregroundStyle(.red)
                    // The phone's sentence, unedited.
                    Text(reason)
                        .font(.caption2)
                }
            } footer: {
                // The one thing the phone's sentence cannot say, because the
                // phone does not know either. It is also the reason this screen
                // must never borrow "Nothing to Answer" for a failure.
                Text("So this agent may still be waiting.")
                    .font(.caption2)
            }
            Section {
                Button("Try Again") { Task { await ask() } }
                    .disabled(!client.state.canAct)
            }
        }
    }

    // MARK: - Asking, and answering

    private func ask() async {
        phase = .asking
        answering = nil
        answerFailure = nil
        // `attempt`, never `send`: see `FleetClient.attempt`. It refuses an
        // unreachable phone with the same sentence the transport would, so this
        // screen renders that as the failure it is rather than sending into
        // nothing.
        switch await client.attempt(.pendingPermission(terminal: terminal)) {
        case let .permission(permission):
            phase = permission.map(Phase.pending) ?? .nothing
        case let .failed(reason):
            phase = .failed(reason)
        case .sent, .transcript:
            // A receipt, or a conversation, in answer to a question about what
            // is pending: a phone speaking a dialect this build does not know.
            // Emphatically not `nothing` — nothing was established about what
            // this agent is waiting on.
            phase = .failed(WatchLinkClient.unreadableReply)
        }
    }

    private func answer(_ permission: WatchPermission, _ option: WatchPermissionOption) {
        // Re-checked at the tap, not only at the render that drew the button.
        // On a watch those are far enough apart for a phone to leave the room,
        // or for a second tap to land on a row already on its way.
        guard answering == nil, client.state.canAct else { return }
        answering = option
        answerFailure = nil
        Task {
            let reply = await client.attempt(
                .answer(terminal: terminal, request: permission.id, option: option.id))
            answering = nil
            switch reply {
            case .sent:
                phase = .answered(option)
            case let .failed(reason):
                // The permission stays on screen with its buttons live again.
                // A failed answer is the case `WatchLinkHost` keeps its replay
                // cache for; re-asking from scratch here would be slower and
                // would throw away the id we already hold.
                answerFailure = reason
            case .permission, .transcript:
                answerFailure = WatchLinkClient.unreadableReply
            }
        }
    }

    /// Which option gets the filled button — and nothing beyond that.
    ///
    /// The same derivation `ApprovalControls` runs on the phone: the
    /// `allow_once` if the agent offered one, otherwise the first thing that
    /// allows at all. Copied rather than reinvented so a wrist and a phone
    /// looking at one permission agree about which answer is the plain yes.
    ///
    /// It returns nil for an agent whose vocabulary this build has never met,
    /// and that is the correct outcome: every button is then drawn the same and
    /// the person reads all of them, which is better than a filled button on
    /// whichever option happened to be first.
    private func plainYes(_ permission: WatchPermission) -> WatchPermissionOption? {
        let allowing = permission.options.filter(allows)
        return allowing.first { $0.kind.lowercased().contains("once") } ?? allowing.first
    }

    private func allows(_ option: WatchPermissionOption) -> Bool {
        let kind = option.kind.lowercased()
        return kind.contains("allow") || kind.contains("accept")
    }
}

/// One answer, as the agent worded it.
///
/// **Never truncated.** The phone clips its secondary options to one line with a
/// middle ellipsis, which it can afford because the full text is a tap away in a
/// transcript beside it. Here there is no transcript and no phone: an option
/// shortened to `No, and tell Claude…` and `No, and tell Claude wha…` is two
/// answers a person cannot tell apart, and picking the wrong one is not
/// something they get to undo from a wrist. Vertical space is what this screen
/// has — the list scrolls — so it spends it.
private struct OptionButton: View {
    let option: WatchPermissionOption
    let emphasized: Bool
    let inFlight: Bool
    let enabled: Bool
    let choose: () -> Void

    var body: some View {
        Group {
            if emphasized {
                button.buttonStyle(.borderedProminent)
            } else {
                button.buttonStyle(.bordered)
            }
        }
        .disabled(!enabled)
    }

    private var button: some View {
        Button(action: choose) {
            HStack(alignment: .firstTextBaseline, spacing: 6) {
                Text(option.name)
                    .multilineTextAlignment(.leading)
                    .frame(maxWidth: .infinity, alignment: .leading)
                // On the row that was pressed, so the answer in flight is the
                // one being looked at. A spinner at the top of the screen would
                // say something is happening without saying to what.
                //
                // `fixedSize` because a bare `ProgressView` takes every point
                // it is offered: without it the spinner claimed a third of the
                // row and truncated the option's name — which is precisely what
                // this view exists not to do.
                if inFlight { ProgressView().fixedSize() }
            }
        }
    }
}

/// What the host last said this agent is on.
///
/// The same three fields the fleet row and the agent screen draw, under the same
/// staleness rule — including the `TimelineView`, because this screen can sit
/// open while an agent crosses `staleAfter` and no news will arrive to move it
/// off `working`. Nothing is composed: `glyph`, `headline` and `line` come off
/// the snapshot, and the only string written here is `stated`'s "last seen"
/// prefix, which is the widget's.
private struct Signal: View {
    let agent: FleetSnapshot.Agent
    let snapshot: FleetSnapshot

    var body: some View {
        TimelineView(.explicit([Date.now] + snapshot.stalenessMoments(after: .now))) { context in
            let confidence = snapshot.confidence(in: agent, at: context.date)
            VStack(alignment: .leading, spacing: 2) {
                HStack(alignment: .firstTextBaseline, spacing: 5) {
                    GlanceMarkView(
                        GlanceMark(agent: agent, confidence: confidence), size: .watchRow)
                        .alignmentGuide(.firstTextBaseline) { $0[.bottom] - 1 }
                    Text(stated(agent, confidence))
                        .font(.caption.weight(.medium))
                }
                if !agent.line.isEmpty, agent.line != agentTitle(agent) {
                    Text(agent.line)
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                }
            }
            .opacity(confidence == .lastSeen ? 0.6 : 1)
        }
    }
}

#if DEBUG
    #Preview("Permission") {
        NavigationStack {
            PermissionView(
                client: PreviewFleetClient(
                    .live(PreviewFleet.snapshot),
                    reply: { _ in .permission(PreviewFleet.claudePermission) }),
                terminal: "a")
        }
    }

    #Preview("Permission, three options") {
        NavigationStack {
            PermissionView(
                client: PreviewFleetClient(
                    .live(PreviewFleet.snapshot),
                    reply: { _ in .permission(PreviewFleet.codexPermission) }),
                terminal: "a")
        }
    }

    #Preview("Permission, nothing pending") {
        NavigationStack {
            PermissionView(
                client: PreviewFleetClient(
                    .live(PreviewFleet.snapshot), reply: { _ in .permission(nil) }),
                terminal: "a")
        }
    }

    #Preview("Permission, couldn’t check") {
        NavigationStack {
            PermissionView(
                client: PreviewFleetClient(
                    .live(PreviewFleet.snapshot),
                    reply: { _ in
                        .failed("Couldn’t read that agent in time. Open it on your iPhone.")
                    }),
                terminal: "a")
        }
    }
#endif
