import SwiftUI

/// What one runner is doing, when that is not simply "answering".
///
/// Android's `RunnerStatusRow` (`FleetScreen.kt`), on the platform that put the
/// same logic in a full-screen phase. `FleetView`'s own header says what it is
/// — "One runner, and what stands in front of it until it answers" — and with N
/// connections that shape is wrong in both directions: a single failing runner
/// would blank the whole app, because the failure phase owns the screen; and a
/// newly added runner needing authorization would show its screen to nobody
/// whenever any other runner answers, which is the ordinary case and the one
/// flow onboarding cannot afford to lose.
///
/// A row that appears next to the runner it concerns is the shape that survives
/// N runners. The Mac reached for status-bar trouble dots instead
/// (`unhealthyHosts`, `staleHosts`); iOS takes Android's answer for the reason
/// the battery toggle already did, which is that both of them are phones.
///
/// **More than one screen draws a list of these**, and each decides for itself
/// what "Authorize This Device" means: Needs You, where the stack can push
/// `AuthorizeView`; and `FleetView`'s pre-fleet screen, which is these rows and
/// nothing else because there is nothing else to draw yet. That is why the
/// moves are callbacks rather than links — a row has no idea what it is inside.
///
/// It renders nothing at all for a connected runner, so a list of these over a
/// healthy fleet costs no height.
///
/// **Every distinct next move `FleetView` offers survives here**, including the
/// two Android's row does not have: "Authorize This Device" for a key the runner
/// has never seen, and "Add This Device Again" for a missing tunnel key. That
/// sentence was not true when this file was written — "Not Now" was dropped, so
/// the only ways past a fingerprint were agreeing to it and force-quitting —
/// which is why neither the list of answers nor the list of moves is a `switch`
/// in this file any more. Both are tables in AgentKit, under
/// `HostKeyQuestionTests`, so the next one to go missing goes missing in front
/// of a test. Losing one would cost somebody the only action that fixes their
/// runner —
/// Android answers both with "Try again", which for a missing node key is a
/// button that can only fail, every time, forever. Neither this nor the
/// full-screen phase decides which move a failure gets: both read
/// `RunnerTrouble.nextMove`, in AgentKit, where a test can read it back.
///
/// Renders nothing at all while the runner is connected, so a list of these
/// above a fleet is empty on the ordinary day.
struct RunnerStatusRow: View {
    @ObservedObject var connection: Connection

    /// The runner this row is about.
    ///
    /// Passed in rather than read off the connection, because a connection that
    /// has not reached `start` yet has no runner on it and this row's whole job
    /// is to say something about the ones that are not answering.
    let host: Runner

    /// Whether to name the runner above the sentence.
    ///
    /// False where the runner is already named by whatever this row sits under
    /// — a section header, or a card. Android's row takes the same argument for
    /// the same reason.
    var showsLabel: Bool = true

    /// Start this runner's connection again, from the beginning.
    var onRetry: () -> Void

    /// Stop waiting out the backoff and dial now. Distinct from `onRetry`: a
    /// reconnecting runner has a timer running and this is what shortcuts it.
    var onReconnectNow: () -> Void

    /// Record the fingerprint on screen as trusted, then connect.
    var onTrust: (String) -> Void

    /// Forget the pinned key and come back to the fingerprint question.
    ///
    /// One callback for two moves — `reviewTheNewKey` and `showTheKeyAgain` —
    /// because they are the same act from the caller's side. What differs is
    /// only what the app is admitting: a key that changed under us, or a
    /// question nobody answered.
    var onReviewKey: () -> Void

    /// Back out of the fingerprint question without answering it.
    ///
    /// **This is the move the port lost.** The full-screen approval phase this
    /// row replaced offered it, and without it the only ways past a fingerprint
    /// somebody does not recognize are agreeing to it and force-quitting —
    /// "Edit…" is not a third: a person who cannot vouch for a key has nothing
    /// to correct, because the address is right and that is the point. See
    /// `HostKeyQuestion`, which is where the list of answers lives so that
    /// losing one is an edit in front of a test.
    var onNotNow: () -> Void

    /// Correct this runner's details.
    var onEdit: () -> Void

    /// This device's own key, and the line to paste on the machine.
    ///
    /// A callback rather than a `NavigationLink`, unlike the full-screen phase,
    /// because a row has no idea what it is inside. `FleetView` can push
    /// `AuthorizeView` because `phases` wraps it in a `NavigationStack` of its
    /// own; the shell deliberately has none, so where this goes is the placing
    /// screen's decision and not this row's.
    var onAuthorize: () -> Void

    /// Move this runner to its address, for good, because a person answered
    /// "Connect directly to … instead?" (ov-485). Defaulted so a screen that
    /// can't move a runner offers nothing.
    var onConnectDirectly: (() -> Void)? = nil

    @ObservedObject private var routes = RunnerRoutes.shared

    var body: some View {
        // Connected is the ordinary state and has nothing to report. Written as
        // an early return rather than a branch that draws an `EmptyView` inside
        // the stack, so a list of these adds no spacing for the runners that are
        // fine.
        if case .connected = connection.phase, !connection.talk.removed {
            EmptyView()
        } else {
            VStack(alignment: .leading, spacing: 6) {
                if showsLabel {
                    Text(host.named)
                        .font(.subheadline.weight(.semibold))
                }
                content
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.horizontal, 16)
            .padding(.vertical, 10)
        }
    }

    @ViewBuilder
    private var content: some View {
        // The runner removed this phone's member (ov-784): connected, and
        // shown nothing. Said in place of everything else about it, with the
        // one move that helps once they're added back.
        if connection.talk.removed {
            removed
        } else if connection.accessRemoved {
            noAccess
        } else {
            phaseContent
        }
    }

    /// The runner refuses this phone's key after letting it in (ov-818):
    /// "You No Longer Have Access to This Runner", what it means, and Check
    /// Again, which dials it now rather than at the next slow check. Held
    /// through that dial, so the row doesn't flick to "Connecting…".
    @ViewBuilder
    private var noAccess: some View {
        Text(RunnerTrouble.accessRemoved.headline(host.words))
            .font(.subheadline.weight(.semibold))
            .accessibilityIdentifier("runner-no-access")
        Text(RunnerTrouble.accessRemoved.detail(message: "", words: host.words))
            .font(.footnote)
            .foregroundStyle(.secondary)
            .fixedSize(horizontal: false, vertical: true)
        Button(RunnerTrouble.accessRemoved.nextMove.label, action: action(for: RunnerTrouble.accessRemoved.nextMove))
            .font(.footnote)
    }

    /// "You Were Removed from This Runner", what it means, and Check Again,
    /// which reads the runner afresh.
    @ViewBuilder
    private var removed: some View {
        Text(RunnerPeople.removedTitle)
            .font(.subheadline.weight(.semibold))
            .accessibilityIdentifier("runner-removed")
        Text(RunnerPeople.removedBody)
            .font(.footnote)
            .foregroundStyle(.secondary)
            .fixedSize(horizontal: false, vertical: true)
        Button("Check Again") {
            connection.talk.checkAgain()
            onRetry()
        }
        .font(.footnote)
    }

    @ViewBuilder
    private var phaseContent: some View {
        switch connection.phase {
        case .connected:
            EmptyView()

        // A way out, which this phase had none of. A mistyped but ROUTABLE
        // address reads as "Connecting…" for the length of the OS's TCP
        // timeout — over a minute — and until this button existed the only
        // thing to do about it was wait the minute out and then correct it. The
        // address is exactly what is wrong in that case, so the editor is the
        // move; there is deliberately no per-row stop, because stopping is a
        // fleet-level offer and `FleetView` already makes it once, held back
        // four seconds so it does not flash up on a healthy launch. See
        // `StopWaiting`.
        case .connecting:
            Text("Connecting…")
                .font(.footnote)
                .foregroundStyle(.secondary)
            Button("Edit…", action: onEdit)
                .font(.footnote)

        // Not an error, and deliberately not worded as one: the rows above this
        // are this runner's last good answer and are still worth reading.
        //
        // The attempt number is left out, the same call the status chip makes.
        // "Reconnecting (4)" prices a wait nobody asked for and reads as an
        // error count; what somebody wants to know here is whether to keep
        // waiting or to tap, and the button beside it answers that.
        case .reconnecting:
            Text("Reconnecting…")
                .font(.footnote)
                .foregroundStyle(.secondary)
            directRoute(after: connection.lastTrouble)
            HStack(spacing: 16) {
                Button("Reconnect Now", action: onReconnectNow)
                Button("Edit…", action: onEdit)
            }
            .font(.footnote)

        case .needsApproval(let fingerprint):
            Text("This runner presented a key Far Cooler has never seen:")
                .font(.footnote)
                .foregroundStyle(.secondary)
            // Where every other piece of host output in this app goes, so the
            // runner's words do not read as the app's own prose. It keeps the
            // selection, so a fingerprint is still something to copy and
            // compare.
            DetailBox(text: fingerprint)
            Text("Check it on the runner: ssh-keygen -lf /etc/ssh/ssh_host_ed25519_key.pub")
                .font(.caption)
                .foregroundStyle(.secondary)
                .textSelection(.enabled)
            // Every answer the question has, drawn FROM the list rather than
            // written out here. Three `Button`s in a view is how one of them
            // went missing in the port, and how the loss stayed invisible: this
            // file lives in the iOS target, which CI compiles and never runs.
            HStack(spacing: 16) {
                ForEach(HostKeyQuestion.Answer.allCases, id: \.self) { answer in
                    Button(answer.label) { answered(answer, fingerprint: fingerprint) }
                        .fontWeight(answer.isTheAnswer ? .semibold : .regular)
                }
            }
            .font(.footnote)

        case .failed(let kind, let message):
            failed(kind, message)
        }
    }

    @ViewBuilder
    private func failed(_ kind: Connection.Failure, _ message: String) -> some View {
        // Red for the one failure that is genuinely alarming and the app's
        // ordinary text for the rest — `RunnerTrouble.isAlarming` names which,
        // and its doc says why every kind being red was wrong.
        Text(kind.headline(host.words))
            .font(.footnote.weight(.medium))
            .foregroundStyle(kind.isAlarming ? AnyShapeStyle(.red) : AnyShapeStyle(.primary))
        Text(kind.detail(message: message, words: host.words))
            .font(.caption)
            .foregroundStyle(.secondary)
            .textSelection(.enabled)

        if kind.showsTheRunnersOwnWords, !message.isEmpty {
            DetailBox(text: message)
        }

        directRoute(after: kind)

        HStack(spacing: 16) {
            Button(kind.nextMove.label, action: action(for: kind.nextMove))
            // Below the primary move rather than beside it in the failure that
            // wants both, for `FleetView`'s reason: retrying is the answer to a
            // runner that was asleep and not to a key it has never seen, so it
            // is the alternative and never the offer.
            if kind.worthRetryingAsAlternative {
                Button("Try Again", action: onRetry)
            }
            if kind.offersEditingTheRunner {
                Button("Edit…", action: onEdit)
            }
        }
        .font(.footnote)
    }

    /// The owner's question when Tailcat fails for a runner with an address
    /// beside it (ov-485): said, and answered only by a tap. Nothing moves the
    /// runner on its own; Try Again stays below for the tunnel.
    @ViewBuilder
    private func directRoute(after trouble: Connection.Failure?) -> some View {
        if let onConnectDirectly, let offer = routes.offer(for: host, after: trouble) {
            // Under the trouble's own heading rather than a second heading:
            // that line already says what failed. The question, what choosing
            // it does, and the one primary action on the card.
            VStack(alignment: .leading, spacing: 4) {
                Text(offer.question)
                    .font(.footnote)
                Text(DirectRouteOffer.afterwards)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                Button(DirectRouteOffer.connectDirectly, action: onConnectDirectly)
                    .font(.footnote.weight(.semibold))
                    .accessibilityIdentifier("connect-directly")
            }
            .padding(.vertical, 2)
            .accessibilityElement(children: .contain)
            .accessibilityIdentifier("direct-route-offer")
        }
    }

    /// One answer to the fingerprint question, as something to call.
    ///
    /// Wiring, and nothing more. WHICH answers exist is `HostKeyQuestion`'s.
    private func answered(_ answer: HostKeyQuestion.Answer, fingerprint: String) {
        switch answer {
        case .trustThisRunner: onTrust(fingerprint)
        case .notNow: onNotNow()
        case .editTheRunner: onEdit()
        }
    }

    /// The move, as something to call.
    ///
    /// The mapping from a decision to a callback, and the only place this row
    /// makes one. Which move a failure gets is not decided here, and neither is
    /// what carrying it out consists of — see `RunnerTrouble.nextMove` and
    /// `NextMove.acts`.
    ///
    /// A move is a LIST of acts and not one, because of the case that used to
    /// be silently wrong here: "Show the Key Again" mapped to `onReviewKey`
    /// alone, and forgetting a key that was never pinned changes nothing, so
    /// the reconcile kept the connection and the button did nothing at all. It
    /// dials as well now, and the reason it must is written where a test can
    /// read it.
    private func action(for move: RunnerTrouble.NextMove) -> () -> Void {
        let acts = move.acts
        return {
            for act in acts {
                switch act {
                case .offerThisDevicesKey: onAuthorize()
                case .forgetThePinnedKey: onReviewKey()
                case .dialAgain: onRetry()
                }
            }
        }
    }
}
