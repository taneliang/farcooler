import AgentKit
import SwiftUI

enum EngineCopy: String, CaseIterable {
    case engine
    case unavailableTitle
    case tryAgain
    case openEngine
    case showInEngine
    case showRunInEngine
    case unavailable
    case stations
    case runDetails
    case step
    case selectedAgentModel
    case agent
    case reviewState
    case currentReason
    case failureDetails
    case sendBacks
    case openTicket
    case closeDetails
    case landingGroups
    case agents
    case selectedModel
    case githubCI
    case jobs
    case reading
    case invalidResponse
    case openRunHint
    case stale
    case updated
    case used
    case queued
    case noSteps
    case noTickets
    case openDetailsHint
    case missingTicket
    case noLandings
    case missingLanding
    case noAgents
    case observed
    case sendBackCount

    private static let words: [String: String] = {
        PlanViewBridge.install()
        return SharedWords.engine() ?? [:]
    }()

    var text: String { Self.words[rawValue] ?? "" }

    func formatted(_ values: (String, String)...) -> String {
        values.reduce(text) { line, value in
            line.replacingOccurrences(of: "{\(value.0)}", with: value.1)
        }
    }
}

enum EngineFocus: Hashable {
    case ticket(String)
    case landing(String)

    var scrollID: String {
        switch self {
        case .ticket: "engine-inspector"
        case .landing(let id): "engine-landing-\(id)"
        }
    }
}

/// Reads through the Mac's authenticated CLI link, like Plan. The JSON only
/// contains the presentation; pinned definitions and raw receipts stay in Rust.
struct EnginePage: View {
    @ObservedObject var client: DaemonClient
    let workspace: WorkspaceSummary
    let focus: EngineFocus?
    let visible: Bool
    let openTicket: (String) -> Void

    @State private var snapshot: EngineSnapshot?
    @State private var observedAt: Date?
    @State private var problem: String?
    @State private var refreshRequest = 0

    private var readProblem: String? {
        if client.state == .unreachable(reason: HostState.runnerNotRespondingReason) {
            return HostState.runnerNotRespondingReason
        }
        return problem
    }

    var body: some View {
        Group {
            if let snapshot, let observedAt {
                EngineView(snapshot: snapshot, observedAt: observedAt, stale: readProblem != nil,
                           focus: focus, refresh: refresh, openTicket: openTicket)
            } else if let problem = readProblem {
                ContentUnavailableView {
                    Label(EngineCopy.unavailableTitle.text, systemImage: "gearshape.2")
                } description: {
                    Text(problem)
                } actions: {
                    Button(EngineCopy.tryAgain.text, action: refresh)
                }
            } else {
                ProgressView(EngineCopy.reading.text)
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            }
        }
        .task(id: visible ? refreshRequest : nil) {
            guard visible else { return }
            // Capacity and CI can change between workflow revisions. Only the
            // visible page polls, with one read in flight and a pause after each.
            while !Task.isCancelled {
                await read()
                try? await Task.sleep(for: .seconds(15))
            }
        }
    }

    private func refresh() { refreshRequest += 1 }

    private func read() async {
        let repository = workspace.repository ?? workspace.id
        let (data, message) = await client.runnerRead(
            ["plan", "engine", "--repo", repository]
                + (workspace.boardWorkspace.map { ["--workspace", $0] } ?? []) + ["--json"])
        guard !Task.isCancelled else { return }
        guard let data, let next = try? JSONDecoder().decode(EngineSnapshot.self, from: data) else {
            problem = message ?? EngineCopy.invalidResponse.text
            return
        }
        if let snapshot, snapshot.workspace == next.workspace,
           snapshot.epoch == next.epoch, snapshot.revision > next.revision { return }
        snapshot = next
        observedAt = .now
        problem = nil
    }
}

struct EngineSnapshot: Decodable {
    let workspace: String
    let epoch: UInt64
    let revision: UInt64
    let capacity: [Capacity]
    let stations: [Station]
    let landings: [Landing]
    let agents: [Agent]

    enum Tone: String, Decodable { case normal, waiting, failed }

    struct Capacity: Decodable, Identifiable {
        let id: String
        let label: String
        let used: Int?
        let limit: Int?
        let queued: Int?
    }

    struct Station: Decodable, Identifiable {
        let id: String
        let label: String
        let tickets: [Ticket]
    }

    struct Ticket: Decodable, Identifiable {
        let id: String
        let taskId: String?
        let title: String
        let step: String
        let status: String
        let tone: Tone
        let summary: String?
        let reason: String?
        let failure: String?
        let agent: String?
        let review: String
        let sendBacks: SendBacks?
    }

    struct SendBacks: Decodable {
        let remaining: Int
        let limit: Int
        let history: [SendBack]
    }

    struct SendBack: Decodable, Identifiable {
        let id: String
        let from: String
        let to: String
        let reason: String
    }

    struct Landing: Decodable, Identifiable {
        let id: String
        let label: String
        let status: String
        let tone: Tone
        let sinceMs: Int64
        let historical: Bool
        let members: [Subject]
        let tickets: [Ticket]
        let failure: String?
        let ci: CI?
    }

    struct Subject: Decodable, Identifiable {
        let id: String
        let title: String
    }

    struct CI: Decodable {
        let status: String
        let sha: String
        let fetchedMs: Int64
        let jobs: [Job]
    }

    struct Job: Decodable {
        let name: String
        let status: String
    }

    struct Agent: Decodable, Identifiable {
        let id: String
        let ticket: String
        let role: String
        let model: String?
        let status: String
        let tone: Tone
    }
}
