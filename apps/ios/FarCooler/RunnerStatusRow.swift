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
/// **Nothing draws this yet.** It is step 3 of the port recorded in
/// `.claude/agent/done/the-fifth-cost-of-the-multi-runner-port.md`, and the
/// screen that will draw a list of these is the shell overview once
/// `FleetStore` is what the app connects through. Placing it today would put a
/// second thing beside `LinkStatusChip` saying the same sentence about the one
/// runner there is, which is the drift it exists to prevent.
///
/// **Every distinct next move `FleetView` offers survives here**, including the
/// two Android's row does not have: "Authorize This Device" for a key the runner
/// has never seen, and "Add This Device Again" for a missing tunnel key. Losing
/// either would cost somebody the only action that fixes their runner —
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

    var body: some View {
        // Connected is the ordinary state and has nothing to report. Written as
        // an early return rather than a branch that draws an `EmptyView` inside
        // the stack, so a list of these adds no spacing for the runners that are
        // fine.
        if case .connected = connection.phase {
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
        switch connection.phase {
        case .connected:
            EmptyView()

        case .connecting:
            Text("Connecting…")
                .font(.footnote)
                .foregroundStyle(.secondary)

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
            Text("Check it on the host: ssh-keygen -lf /etc/ssh/ssh_host_ed25519_key.pub")
                .font(.caption)
                .foregroundStyle(.secondary)
                .textSelection(.enabled)
            HStack(spacing: 16) {
                Button("Trust This Runner") { onTrust(fingerprint) }
                Button("Edit…", action: onEdit)
            }
            .font(.footnote)

        case .failed(let message):
            failed(message)
        }
    }

    @ViewBuilder
    private func failed(_ message: String) -> some View {
        let kind = Connection.Failure(message: message)

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

    /// The move, as something to call.
    ///
    /// The mapping from a decision to a callback, and the only place this row
    /// makes one. Which move a failure gets is not decided here — see
    /// `RunnerTrouble.nextMove`.
    private func action(for move: RunnerTrouble.NextMove) -> () -> Void {
        switch move {
        case .authorizeThisDevice, .addThisDeviceAgain: return onAuthorize
        case .reviewTheNewKey, .showTheKeyAgain: return onReviewKey
        case .tryAgain: return onRetry
        }
    }
}
