import SwiftUI

/// A member's Talk (ov-526): their own rep, in the place the orchestrator's
/// pane takes for the owner and the lead.
///
/// The runner refuses a member who isn't the lead the orchestrator's composer
/// (`not_lead`), so this never draws one. It draws the rep's pane, the same
/// terminal and conversation views, starting the rep first when it isn't
/// running: choosing Talk is the ask. What the person reads while it starts
/// is the rep's first screen: what a rep is for, and whose Claude account the
/// work runs on, in the words Runner Settings uses (`RunnerPeople`).
struct RepSeat: View {
    @ObservedObject var connection: Connection
    let summary: WorkspaceSummary
    let place: PhoneWorkspace
    let shown: Bool
    let handle: String
    /// The rep the runner lists for this person here, if it has one.
    let existing: RunnerPeople.Rep?
    /// Taps on the Talk segment so far: each is an ask to start the rep.
    let talkTaps: Int

    @StateObject private var pastes = ImagePasteQueue()
    @State private var starting = false
    @State private var refusal: String?

    /// The rep's pane, as the fleet has it.
    private var pane: Terminal? {
        guard let existing else { return nil }
        return connection.fleet.worktrees.lazy.flatMap(\.terminals).first { $0.id == existing.terminal }
    }

    /// Whether this phone may start a rep: not on a Read grant.
    private var mayAct: Bool { connection.daemon?.mayAct ?? true }

    private var people: RunnerPeople? { connection.talk.people }

    var body: some View {
        Group {
            if let pane, !Self.isDown(pane) {
                if StateKind.parse(pane.state) == .starting {
                    intro(starting: true)
                } else {
                    OrchestratorPane(
                        terminal: pane, connection: connection, pastes: pastes, place: place,
                        shown: shown, identifier: "rep-pane")
                        .id(pane.id)
                }
            } else if starting {
                intro(starting: true)
            } else {
                intro(starting: false)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .overlay(alignment: .bottom) { ImagePasteChips(queue: pastes) }
        .onChange(of: talkTaps) { _, _ in Task { await startIfAsked() } }
    }

    /// A rep whose process has ended, or that was never started.
    static func isDown(_ pane: Terminal) -> Bool {
        switch StateKind.parse(pane.state) {
        case .lost, .exited, .error: true
        default: false
        }
    }

    /// A tap on the Talk segment starts the rep when it isn't running, and
    /// nothing else does: a remembered segment on opening, or a rep the
    /// person stopped, is never started behind their back. Where nothing was
    /// tapped (the iPad's chat column), the button waits to be pressed.
    private func startIfAsked() async {
        guard shown, !starting, mayAct, pane.map(Self.isDown) ?? true else { return }
        await start()
    }

    private func start() async {
        refusal = nil
        starting = true
        refusal = await connection.talk.start(connection, workspace: summary.id, handle: handle)
        starting = false
    }

    /// The rep's first screen, and while it starts.
    private func intro(starting: Bool) -> some View {
        let stopped = pane.map(Self.isDown) ?? false
        return ScrollView {
            VStack(spacing: 14) {
                Image(systemName: "person.crop.circle")
                    .font(.system(size: 44))  // style-exempt: an empty state's symbol, sized as ContentUnavailableView sizes its own
                    .foregroundStyle(.secondary)
                    .accessibilityHidden(true)
                Text(stopped && !starting ? RunnerPeople.repStoppedTitle : RunnerPeople.repTitle)
                    .font(.title2.weight(.semibold))
                    .multilineTextAlignment(.center)
                    .accessibilityAddTraits(.isHeader)
                    .accessibilityIdentifier("rep-title")
                if starting {
                    HStack(spacing: 8) {
                        ProgressView()
                        Text(RunnerPeople.repStarting)
                            .foregroundStyle(.secondary)
                            .accessibilityIdentifier("rep-starting")
                    }
                } else if stopped {
                    Text(RunnerPeople.repStoppedBody)
                        .foregroundStyle(.secondary)
                        .multilineTextAlignment(.center)
                        .accessibilityIdentifier("rep-stopped")
                }
                if let people {
                    Text(RunnerPeople.repPurpose)
                        .multilineTextAlignment(.center)
                        .accessibilityIdentifier("rep-purpose")
                    Text(people.accountLine)
                        .font(.footnote)
                        .foregroundStyle(.secondary)
                        .multilineTextAlignment(.center)
                        .accessibilityIdentifier("rep-account")
                }
                if let refusal {
                    Label(refusal, systemImage: "exclamationmark.triangle.fill")
                        .font(.footnote)
                        .foregroundStyle(.secondary)
                        .multilineTextAlignment(.leading)
                        .accessibilityIdentifier("rep-refusal")
                }
                if !starting {
                    if mayAct {
                        Button(refusal != nil ? RunnerPeople.tryAgain : (stopped ? RunnerPeople.resumeRep : RunnerPeople.startRep)) {
                            Task { await start() }
                        }
                        .buttonStyle(.borderedProminent)
                        .accessibilityIdentifier("rep-start")
                    } else {
                        Text("This device can only look, so it can’t start your rep.")
                            .font(.footnote)
                            .foregroundStyle(.secondary)
                            .multilineTextAlignment(.center)
                    }
                }
            }
            .frame(maxWidth: 420)
            .padding(.horizontal, 24)
            .padding(.vertical, 32)
            .frame(maxWidth: .infinity)
        }
        .scrollBounceBehavior(.basedOnSize)
    }
}
