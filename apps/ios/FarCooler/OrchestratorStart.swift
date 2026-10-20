import SwiftUI

// Starting a workspace's orchestrator from an iPhone (ov-616). A workspace
// with no orchestrator has the Plan as its screen, with Start Orchestrator
// above it (`OrchestratorInvitation`); once asked, the screen is the
// conversation, which says "Starting Orchestrator…" and, for an agent that
// isn't installed, says so (`OrchestratorSegment`). What was asked is one
// object both read, so the start outlives the screen that asked for it.

extension Connection {
    /// Whether `summary`'s screen on a phone is a conversation: a member's
    /// rep (ov-526), or an orchestrator that has a pane, running or not. A
    /// repository's own board, and a workspace whose orchestrator was never
    /// started, have none, and their screen is the Plan.
    func hasConversation(_ summary: WorkspaceSummary) -> Bool {
        guard !summary.isImplicit else { return false }
        switch talk.seat(for: summary, build: daemon) {
        case .orchestrator: return OrchestratorSegment.terminal(in: self, summary: summary) != nil
        case .rep, .reading, .unknown, .neither: return true
        }
    }
}

/// An orchestrator this phone asked for, from when it asked until its pane
/// is up, and what the runner said.
@MainActor
final class OrchestratorStart: ObservableObject {
    /// When this phone asked for one, until its pane appears.
    @Published var startedAt: Date?
    /// What the runner said when it refused, in this app's words.
    @Published var refusal: String?
    /// When this phone asked for the start, and which agent, kept past the
    /// pane's arrival so a quick exit 127 can be called "not installed".
    @Published var asked: Date?
    @Published var askedHarness: AgentHarness?
    /// How long after asking the pane was first seen ended, once it was.
    @Published var endedAfter: TimeInterval?

    /// Ask `connection` for `harness` as `workspace`'s orchestrator, or in
    /// place of the one it has.
    func begin(_ harness: AgentHarness, replace: Bool, connection: Connection, workspace: String) {
        refusal = nil
        startedAt = Date()
        asked = startedAt
        askedHarness = harness
        endedAfter = nil
        Task {
            do {
                _ = try await connection.startOrchestrator(workspace: workspace, harness: harness.rawValue, replace: replace)
            } catch {
                startedAt = nil
                refusal = ClientCore.trouble(error, after: "The runner didn’t start the orchestrator.").sentence
            }
        }
    }
}

/// Start Orchestrator, at the top of the Plan of a workspace that has none:
/// what one does, and the button that starts one.
struct OrchestratorInvitation: View {
    @ObservedObject var connection: Connection
    let summary: WorkspaceSummary
    @ObservedObject var start: OrchestratorStart

    private var availability: HarnessAvailability {
        connection.daemon?.availability ?? HarnessAvailability(agentsFound: nil)
    }

    /// Not on a Read grant (as Android's `mayControl`).
    private var mayAct: Bool { connection.daemon?.mayAct ?? true }

    var body: some View {
        Section {
            VStack(spacing: Spacing.inset) {
                Image(systemName: "person.crop.circle.badge.plus")
                    .font(.largeTitle)
                    .foregroundStyle(.secondary)
                    .accessibilityHidden(true)
                Text(FirstRunCopy.Phone.orchestratorTitle)
                    .font(.headline)
                    .multilineTextAlignment(.center)
                    .accessibilityAddTraits(.isHeader)
                PhoneEmptyRows(copy: PhoneEmptyStates.noOrchestrator, compact: true)
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
                if availability.isKnown && availability.installed.isEmpty {
                    Text(FirstRunCopy.Conversation.noneInstalled(on: .runner(connection.hostLabel)))
                        .font(.footnote)
                        .foregroundStyle(.secondary)
                        .multilineTextAlignment(.center)
                        .accessibilityIdentifier("orchestrator-none-installed")
                }
                if let refusal = start.refusal {
                    Label(refusal, systemImage: "exclamationmark.triangle.fill")
                        .font(.footnote)
                        .foregroundStyle(.secondary)
                        .accessibilityIdentifier("orchestrator-refusal")
                }
                if mayAct {
                    Menu(FirstRunCopy.Phone.start) {
                        ForEach(AgentHarness.allCases, id: \.rawValue) { harness in
                            if availability.isInstalled(harness) {
                                Button(harness.title) {
                                    start.begin(harness, replace: false, connection: connection, workspace: summary.id)
                                }
                            } else {
                                Button("\(harness.title) · \(FirstRunCopy.Navigator.notInstalled)") {}
                                    .disabled(true)
                            }
                        }
                    }
                    .buttonStyle(.borderedProminent)
                    .disabled(availability.isKnown && availability.installed.isEmpty)
                    .accessibilityIdentifier("start-orchestrator")
                }
            }
            .frame(maxWidth: .infinity)
            .padding(.vertical, Spacing.inset)
        }
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("orchestrator-invitation")
    }
}
