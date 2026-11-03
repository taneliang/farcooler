import AgentKit
import Foundation
import Testing

@testable import Far_Cooler

@MainActor
struct CheckInSettingsTests {
    private let workspace = WorkspaceSummary(
        id: "workspace-id", name: "Main", taskPrefix: "ov", isMain: true, ordinal: 0, repository: "repo-id")

    @Test("The Mac names the workspace and repository and sends only a selected task key")
    func arguments() {
        #expect(DaemonClient.checkInArguments(workspace, action: ["show"]) == [
            "workspace", "check-in", "workspace-id", "--repo", "repo-id", "show", "--json",
        ])
        #expect(DaemonClient.checkInArguments(workspace, action: ["enable", "--task", "ov-42"]) == [
            "workspace", "check-in", "workspace-id", "--repo", "repo-id", "enable", "--task", "ov-42", "--json",
        ])
        let unknown = WorkspaceSummary(id: "w", name: "Other", taskPrefix: "", isMain: false, ordinal: 1)
        #expect(DaemonClient.checkInArguments(unknown, action: ["disable"]) == nil)
    }

    @Test("Only open tasks on this workspace are offered")
    func openTasks() {
        let now = Date()
        let open = TaskRow(id: "one", key: "ov-1", title: "Open", status: .inProgress, statusSince: now, workspaceID: "workspace-id")
        let done = TaskRow(id: "two", key: "ov-2", title: "Done", status: .done, statusSince: now, workspaceID: "workspace-id")
        let other = TaskRow(id: "three", key: "ov-3", title: "Other", status: .todo, statusSince: now, workspaceID: "other")
        let unknown = TaskRow(id: "four", key: "ov-4", title: "Unknown", status: .todo, statusSince: now)
        #expect(CheckInSettings.openTasks([open, done, other, unknown], workspace: "workspace-id") == [open])
    }

    @Test("An admitted or uncertain earlier key requires explicit acknowledgement")
    func acknowledgement() {
        for state in ["enter_admitted", "submitted", "uncertain", "resolved_uncertain"] {
            #expect(CheckInSettings.requiresAcknowledgement(state))
        }
        #expect(!CheckInSettings.requiresAcknowledgement("confirmed"))
        #expect(!CheckInSettings.requiresAcknowledgement("pending"))
    }

    @Test("Refreshing from acknowledged uncertainty A to B requires fresh acknowledgement")
    func acknowledgementBelongsToDisplayedOccurrence() {
        let occurrenceA = CheckInAcknowledgementIdentity(occurrence: "A", state: "uncertain", generation: "generation-A")
        let occurrenceB = CheckInAcknowledgementIdentity(occurrence: "B", state: "uncertain", generation: "generation-B")
        var acknowledgement = CheckInAcknowledgement()

        acknowledgement.show(occurrenceA)
        acknowledgement.setAcknowledged(true)
        #expect(acknowledgement.isAcknowledged(for: occurrenceA))
        #expect(CheckInSettings.enableAction(task: "ov-42", identity: occurrenceA) == ["enable", "--task", "ov-42", "--acknowledge-uncertain", "--expected-generation", "generation-A"])

        acknowledgement.show(occurrenceB)
        #expect(!acknowledgement.isAcknowledged(for: occurrenceB))
        acknowledgement.setAcknowledged(true)
        #expect(acknowledgement.isAcknowledged(for: occurrenceB))

        acknowledgement.clear()
        #expect(!acknowledgement.isAcknowledged(for: occurrenceB))
    }

    @Test("A confirmed receipt and an unavailable schedule decode from the runner")
    func status() throws {
        let data = Data(#"{"configured":true,"task":"task-id","state":"unavailable","reason":"scheduler_unavailable_unclean_or_storage","next_due_ms":3600000,"occurrence":"occ-id","occurrence_state":"confirmed","occurrence_reason":"","submitted_at_ms":100,"confirmed_at_ms":200,"receipt_evidence":"codex_rollout_line:4"}"#.utf8)
        let status = try JSONDecoder().decode(CheckInReading.self, from: data)
        #expect(status.task == "task-id")
        #expect(status.state == "unavailable")
        #expect(status.occurrenceState == "confirmed")
        #expect(status.confirmedAt == 200)
        #expect(status.receiptEvidence == "codex_rollout_line:4")
    }
}
