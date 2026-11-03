import AgentKit
import SwiftUI

/// The existing workspace check-in command reports the runner's status. The
/// Mac never supplies caller identity; the runner decides who may change it.
struct CheckInSheetRequest: Identifiable {
    let host: String
    let workspace: WorkspaceSummary
    var id: String { "\(host)|\(workspace.id)" }
}

struct CheckInReading: Decodable {
    let configured: Bool
    let task: String?
    let generation: String?
    let state: String
    let reason: String
    let nextDue: Int64
    let occurrence: String?
    let occurrenceState: String
    let occurrenceReason: String
    let submittedAt: Int64?
    let confirmedAt: Int64?
    let receiptEvidence: String

    enum CodingKeys: String, CodingKey {
        case configured, task, generation, state, reason, occurrence
        case nextDue = "next_due_ms"
        case occurrenceState = "occurrence_state"
        case occurrenceReason = "occurrence_reason"
        case submittedAt = "submitted_at_ms"
        case confirmedAt = "confirmed_at_ms"
        case receiptEvidence = "receipt_evidence"
    }
}

struct CheckInAcknowledgementIdentity: Equatable {
    let occurrence: String
    let state: String
    let generation: String
}

struct CheckInAcknowledgement {
    private(set) var displayed: CheckInAcknowledgementIdentity? = nil
    private(set) var acknowledged: CheckInAcknowledgementIdentity? = nil

    func isAcknowledged(for identity: CheckInAcknowledgementIdentity?) -> Bool {
        identity != nil && displayed == identity && acknowledged == identity
    }

    mutating func show(_ identity: CheckInAcknowledgementIdentity?) {
        if displayed != identity { acknowledged = nil }
        displayed = identity
    }

    mutating func setAcknowledged(_ value: Bool) {
        acknowledged = value ? displayed : nil
    }

    mutating func clear() {
        acknowledged = nil
    }
}

extension DaemonClient {
    static func checkInArguments(_ workspace: WorkspaceSummary, action: [String]) -> [String]? {
        guard let repository = workspace.repository else { return nil }
        return ["workspace", "check-in", workspace.id, "--repo", repository] + action + ["--json"]
    }

    /// This is a person-operated app subprocess. The runner obtains its PID
    /// from the socket and independently checks the caller's seat and owner.
    func checkIn(_ workspace: WorkspaceSummary, action: [String]) async -> (data: Data?, message: String?) {
        guard let arguments = Self.checkInArguments(workspace, action: action) else {
            return (nil, "This workspace’s repository is unavailable. Refresh and try again.")
        }
        return await runRaw(arguments)
    }
}

struct CheckInSettings: View {
    let workspace: WorkspaceSummary
    let client: DaemonClient
    let canManage: Bool
    @Environment(\.dismiss) private var dismiss
    @State private var status: CheckInReading?
    @State private var tasks: [TaskRow] = []
    @State private var selected = ""
    @State private var acknowledgement = CheckInAcknowledgement()
    @State private var working = false
    @State private var failure: String?

    static func openTasks(_ rows: [TaskRow], workspace: String) -> [TaskRow] {
        rows.filter { !$0.status.isFinished && $0.workspaceID == workspace }
    }

    static func requiresAcknowledgement(_ occurrenceState: String) -> Bool {
        ["enter_admitted", "submitted", "uncertain", "resolved_uncertain"].contains(occurrenceState)
    }

    private var uncertain: Bool {
        status.map { Self.requiresAcknowledgement($0.occurrenceState) } ?? false
    }

    private var acknowledgementIdentity: CheckInAcknowledgementIdentity? {
        guard let status else { return nil }
        return Self.acknowledgementIdentity(for: status)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text("Hourly Check-In · \(workspace.name)").font(.headline)
            Text("A fixed reminder reaches this workspace’s idle Codex orchestrator. It does not grant new approval.")
                .foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
            if let status {
                statusView(status)
                if canManage {
                    Divider()
                    if tasks.isEmpty {
                        Text("No open tasks are available on this board.").foregroundStyle(.secondary)
                    } else {
                        Picker("Audit task", selection: $selected) {
                            Text("Choose a task").tag("")
                            ForEach(tasks) { task in Text("\(task.key) · \(task.title)").tag(task.id) }
                        }
                    }
                    if uncertain {
                        Text("An earlier key may still arrive. A receipt confirms input, not completed work.")
                            .foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
                        Toggle("I understand the earlier check-in may still arrive", isOn: Binding(
                            get: { acknowledgement.isAcknowledged(for: acknowledgementIdentity) },
                            set: { acknowledgement.show(acknowledgementIdentity); acknowledgement.setAcknowledged($0) }
                        ))
                    }
                    HStack {
                        Button(status.configured && status.state == "enabled" ? "Update Check-In" : "Enable Check-In") {
                            Task { await change(enable: true) }
                        }
                        .disabled(working || selected.isEmpty || (uncertain && !acknowledgement.isAcknowledged(for: acknowledgementIdentity)))
                        if status.configured && status.state != "disabled" {
                            Button("Disable Check-In") { Task { await change(enable: false) } }
                                .disabled(working)
                        }
                    }
                } else {
                    Text("Only the runner’s owner can enable or change an hourly check-in.")
                        .foregroundStyle(.secondary)
                }
            } else if working {
                ProgressView("Reading check-in…")
            }
            if let failure {
                Text(failure).foregroundStyle(.red).fixedSize(horizontal: false, vertical: true)
                    .accessibilityIdentifier("check-in-failure")
            }
            HStack {
                Button("Refresh") { Task { await load() } }.disabled(working)
                Spacer()
                Button("Close") { dismiss() }.keyboardShortcut(.cancelAction)
            }
        }
        .padding(20)
        .frame(minWidth: 440, maxWidth: 540)
        .task { await load() }
    }

    @ViewBuilder private func statusView(_ status: CheckInReading) -> some View {
        if !status.configured {
            Text("Off").font(.subheadline.weight(.semibold))
        } else {
            Text("Status: \(status.state.replacingOccurrences(of: "_", with: " "))")
                .font(.subheadline.weight(.semibold))
            if status.nextDue > 0 && status.state == "enabled" {
                Text("Next due: \(date(status.nextDue))")
            }
            if !status.reason.isEmpty { Text(reason(status.reason)) }
            if let occurrence = status.occurrence {
                Text("Latest check-in: \(status.occurrenceState.replacingOccurrences(of: "_", with: " "))")
                Text(occurrence).font(.caption.monospaced()).textSelection(.enabled)
                if let confirmed = status.confirmedAt, confirmed > 0 {
                    Text("Receipt observed: \(date(confirmed))")
                    Text("Receipt confirms input, not completed work.").font(.caption).foregroundStyle(.secondary)
                } else if let submitted = status.submittedAt, submitted > 0 {
                    Text("Submitted \(date(submitted)); receipt not yet confirmed.").font(.caption)
                }
                if !status.occurrenceReason.isEmpty {
                    Text(reason(status.occurrenceReason)).font(.caption)
                }
            }
        }
    }

    private func reason(_ value: String) -> String {
        switch value {
        case "scheduler_unavailable_unclean_or_storage":
            return "The runner can’t safely start check-ins after an interruption or storage problem."
        case "cleanup_completion_unknown":
            return "A previous cleanup may still be in flight, so check-ins are unavailable."
        case "eligibility_changed":
            return "The workspace or audit task changed. The owner must enable check-ins again."
        case "authorizer_changed":
            return "The authorizing owner’s access changed. Check-ins are suspended."
        case "waiting_for_idle_codex":
            return "Waiting for the Codex orchestrator to be idle."
        default:
            return value.replacingOccurrences(of: "_", with: " ")
        }
    }

    private func date(_ millis: Int64) -> String {
        Date(timeIntervalSince1970: Double(millis) / 1000).formatted(date: .abbreviated, time: .shortened)
    }

    @MainActor private func readStatus() async -> Bool {
        let (data, message) = await client.checkIn(workspace, action: ["show"])
        guard let data, let value = try? JSONDecoder().decode(CheckInReading.self, from: data) else {
            acknowledgement.clear()
            failure = message ?? "The runner’s check-in status could not be read."
            return false
        }
        acknowledgement.show(Self.acknowledgementIdentity(for: value))
        status = value
        return true
    }

    static func acknowledgementIdentity(for status: CheckInReading) -> CheckInAcknowledgementIdentity? {
        guard requiresAcknowledgement(status.occurrenceState), let occurrence = status.occurrence,
              let generation = status.generation else { return nil }
        return CheckInAcknowledgementIdentity(occurrence: occurrence, state: status.occurrenceState, generation: generation)
    }

    @MainActor private func load() async {
        working = true
        defer { working = false }
        failure = nil
        tasks = []
        selected = ""
        guard await readStatus() else { return }
        guard let repository = workspace.repository else {
            failure = "This workspace’s repository is unavailable. Refresh and try again."
            return
        }
        let (data, message) = await client.taskBoard(repository: repository, workspace: workspace.id)
        guard let data, let board = try? TaskBoardModel.decode(data) else {
            failure = message ?? "The workspace’s tasks could not be read."
            return
        }
        tasks = Self.openTasks(board.rows, workspace: workspace.id)
        if let task = status?.task, tasks.contains(where: { $0.id == task }) { selected = task }
    }

    static func enableAction(task: String, identity: CheckInAcknowledgementIdentity?) -> [String] {
        var action = ["enable", "--task", task]
        if let identity { action += ["--acknowledge-uncertain", "--expected-generation", identity.generation] }
        return action
    }

    @MainActor private func change(enable: Bool) async {
        guard canManage else { return }
        var action = ["disable"]
        if enable {
            guard let task = tasks.first(where: { $0.id == selected }) else { return }
            let identity = acknowledgementIdentity
            if uncertain && !acknowledgement.isAcknowledged(for: identity) { return }
            action = Self.enableAction(task: task.key, identity: uncertain ? identity : nil)
        }
        working = true
        defer { working = false }
        failure = nil
        let (data, message) = await client.checkIn(workspace, action: action)
        guard let data, let value = try? JSONDecoder().decode(CheckInReading.self, from: data) else {
            failure = message ?? "The check-in could not be changed."
            return
        }
        acknowledgement.show(Self.acknowledgementIdentity(for: value))
        acknowledgement.clear()
        status = value
    }
}
