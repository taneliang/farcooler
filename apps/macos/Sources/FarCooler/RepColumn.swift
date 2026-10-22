import AgentKit
import SwiftUI

/// A member's conversation column (ov-526): their own rep, in the place the
/// orchestrator takes for the owner and the lead.
///
/// The states are worked out here as values, so `RepColumnTests` pins them;
/// the view draws them and decides nothing. A live rep is drawn by the
/// window as a terminal pane is; everything else is `RepPlaceholder`.
enum RepColumn {
    enum State: Equatable {
        /// `member list` hasn't answered: nothing to type into yet.
        case reading
        /// It couldn't be read, so there's no telling whom this Mac talks to.
        case unknown
        /// A device the runner has removed from the team.
        case neither
        /// The rep hasn't been started here.
        case none
        /// A start this Mac asked for, or the runner is making.
        case starting
        /// The rep's process ended: Resume starts it again.
        case stopped
        /// Running: its pane is the column.
        case live
    }

    /// The column's state for `seat`, from the rep's pane as the fleet has
    /// it (`pane`), and whether this Mac has a start in flight.
    static func state(seat: TalkSeat, pane: Terminal?, isStarting: Bool) -> State {
        switch seat {
        case .reading: return .reading
        case .unknown: return .unknown
        case .neither: return .neither
        case .orchestrator: return .live
        case .rep:
            if isStarting { return .starting }
            guard let pane else { return .none }
            switch StateKind.parse(pane.state) {
            case .starting: return .starting
            case .lost, .exited, .error: return .stopped
            default: return .live
            }
        }
    }

    /// Whether choosing Talk starts the rep: it isn't running and nothing
    /// is starting it, on a runner this Mac may act on.
    static func startsOnTalk(_ state: State, canAct: Bool) -> Bool {
        canAct && (state == .none || state == .stopped)
    }
}

/// Every state of a rep's column but a live one, centered in it.
struct RepPlaceholder: View {
    let state: RepColumn.State
    /// What a rep is for, then whose Claude account its work runs on:
    /// `RunnerPeople.repIntro`.
    let intro: [String]
    let refusal: String?
    let canAct: Bool
    var onStart: () -> Void
    var onTryAgain: () -> Void = {}
    var carded = true

    var body: some View {
        VStack(spacing: 10) {
            switch state {
            case .reading, .live:
                ProgressView().controlSize(.small)
            case .unknown:
                Text(RunnerPeople.unknownTitle).font(.headline)
                Text(RunnerPeople.unknownBody)
                    .font(.callout).foregroundStyle(.quietInk)
                if canAct { Button(RunnerPeople.tryAgain, action: onTryAgain) }
            case .neither:
                Text(RunnerPeople.neitherTitle).font(.headline)
                Text(RunnerPeople.neitherBody)
                    .font(.callout).foregroundStyle(.quietInk).multilineTextAlignment(.center)
            case .none, .starting, .stopped:
                if state == .starting {
                    StatusGlyph(status: .starting, size: .lone)
                    Text(RunnerPeople.repStarting).font(.headline)
                } else if state == .stopped {
                    Text(RunnerPeople.repStoppedTitle).font(.headline)
                    Text(RunnerPeople.repStoppedBody)
                        .font(.callout).foregroundStyle(.quietInk).multilineTextAlignment(.center)
                } else {
                    Text(RunnerPeople.repTitle).font(.headline)
                }
                ForEach(Array(intro.enumerated()), id: \.offset) { index, line in
                    Text(line)
                        .font(.callout)
                        .foregroundStyle(index == 0 ? Color.primary : Ink.quiet)
                        .multilineTextAlignment(.center)
                        .fixedSize(horizontal: false, vertical: true)
                        .accessibilityIdentifier(index == 0 ? "rep-purpose" : "rep-account")
                }
                if let refusal {
                    Label(refusal, systemImage: "exclamationmark.triangle.fill")
                        .font(.callout)
                        .foregroundStyle(.quietInk)
                        .fixedSize(horizontal: false, vertical: true)
                        .accessibilityIdentifier("rep-refusal")
                }
                if state != .starting {
                    if canAct {
                        Button(
                            refusal != nil ? RunnerPeople.tryAgain
                                : (state == .stopped ? RunnerPeople.resumeRep : RunnerPeople.startRep),
                            action: onStart
                        )
                        .accessibilityIdentifier("rep-start")
                    } else {
                        Text("This Mac can only look, so it can’t start your rep.")
                            .font(.callout).foregroundStyle(.quietInk)
                    }
                }
            }
        }
        .padding(20)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .modifier(CardIf(on: carded))
        .accessibilityIdentifier("rep-column")
    }
}
