import AgentKit
import Foundation
import Testing

@testable import Far_Cooler

/// Roles on the Mac (ov-784): Show Charter's edit, the wake switch a
/// Maintainer keeps, and the screen a removed member's Mac shows.
@MainActor
struct MaintainerMacTests {
    private static let billing = WorkspaceSummary(
        id: "ws-billing", name: "Billing and Invoicing", taskPrefix: "bil", isMain: false, ordinal: 1,
        repository: "repo", lead: "erin")

    @Test("Edit is offered for a charter or none, never while reading or after a failed read")
    func editIsOfferedForWhatCanBeEdited() {
        #expect(CharterSheet.canEdit(.text("## Who decides\n", sha256: "ab")))
        #expect(CharterSheet.canEdit(.missing))
        #expect(!CharterSheet.canEdit(.failed("Couldn’t read it.")))
        #expect(!CharterSheet.canEdit(nil))
    }

    @Test("Save waits for something written and changed")
    func saveWaitsForAChange() {
        let read = CharterReading.text("## Who decides\nThe lead.\n", sha256: "ab")
        #expect(!CharterSheet.canSave(draft: "## Who decides\nThe lead.\n", was: read), "unchanged")
        #expect(!CharterSheet.canSave(draft: " \n", was: .missing), "blank")
        #expect(CharterSheet.canSave(draft: "## Who decides\nCarol.\n", was: read))
        #expect(CharterSheet.canSave(draft: "## Who decides\n", was: .missing))
    }

    @Test("The edit is sent from a file, with the hash it was made to, and the hash comes back with the answer")
    func theEditAndItsHash() {
        #expect(
            DaemonClient.setCharterArguments(Self.billing, file: "/tmp/c.md", expected: "ab12")
                == ["workspace", "set-charter", "ws-billing", "--from", "/tmp/c.md", "--expect", "ab12", "--json"])
        let read = Data(###"{"workspace":"w","exists":true,"text":"## A\n","sha256":"ab12"}"###.utf8)
        #expect(CharterReading.parse(read)?.sha256 == "ab12")
        #expect(CharterReading.missing.sha256 == "", "no charter is made to the empty hash")
    }

    @Test("A refused edit says why in the app's words, and an edit made meanwhile keeps the draft")
    func refusals() {
        let changed = CharterReading.saveRefusal("code: resource-conflict\nwhat: charter_changed", workspace: Self.billing)
        #expect(changed.contains("Someone else changed Billing and Invoicing’s charter"))
        #expect(changed.contains("Your text is still here"))
        let notYours = CharterReading.saveRefusal("code: not-yours\nwhat: not_yours", workspace: Self.billing)
        #expect(notYours.hasSuffix("Only the runner’s owner, this workspace’s lead and Maintainers can edit it."))
        #expect(CharterReading.saveRefusal(nil, workspace: Self.billing) == CharterReading.notSaved(Self.billing))
    }

    /// The wake switch is the owner's and a Maintainer's: a lead who isn't
    /// one loses it, a Maintainer who doesn't lead gets it beside Show
    /// Charter. Without `keepsSettings` both read as before.
    @Test("The wake switch follows who keeps the workspace, not who leads it")
    func theWakeSwitch() {
        let lead = OrchestratorMenu.items(canAct: true, hasSeat: true, charter: .read, wakeOnAnswer: true, keepsSettings: false)
        #expect(lead == [.replaceOrchestrator, .showCharter])
        let maintainer = OrchestratorMenu.items(canAct: false, hasSeat: true, charter: .read, wakeOnAnswer: true, keepsSettings: true)
        #expect(maintainer == [.showCharter, .wakeOnAnswer])
        let member = OrchestratorMenu.items(canAct: false, hasSeat: true, charter: .read, wakeOnAnswer: true, keepsSettings: false)
        #expect(member == [.showCharter])
        #expect(OrchestratorMenu.items(canAct: false, hasSeat: true, charter: .read, wakeOnAnswer: nil, keepsSettings: true) == [.showCharter])
        #expect(OrchestratorMenu.items(canAct: true, hasSeat: true, charter: .read, wakeOnAnswer: true) == [.replaceOrchestrator, .showCharter, .wakeOnAnswer], "not known: as before")
        #expect(OrchestratorMenu.items(canAct: false, hasSeat: true, charter: .read, wakeOnAnswer: true) == [.showCharter], "not known: as before")
    }

    @Test("A refusal as someone else's is asked about; the list's answer decides")
    func aRemovalIsTold() {
        #expect(DaemonClient.mayBeRemoval("code: not-yours\nwhat: not_yours"))
        #expect(!DaemonClient.mayBeRemoval("ssh: connect to host studio port 22: Connection refused"))
        #expect(DaemonClient.saysRemoved(Data(#"{"members":[],"claude_account":"","you":"(released)"}"#.utf8)))
        #expect(!DaemonClient.saysRemoved(Data(#"{"members":[],"claude_account":"","you":"bob"}"#.utf8)))
        #expect(!DaemonClient.saysRemoved(Data("not json".utf8)))
        #expect(HostState.removed.refusal == RunnerPeople.removedBody)
    }

    @Test("The placeholder says a removal before anything else about that runner")
    func thePlaceholderSaysIt() {
        let restoring = FleetPlaceholder.phase(
            hasWorktrees: false, localLoaded: true, localError: nil, hasRepositories: true,
            restoringOn: "studio", trouble: RunnerPeople.removedBody, waited: 20, removedFrom: "studio")
        #expect(restoring == .removed("studio"))
        let elsewhere = FleetPlaceholder.phase(
            hasWorktrees: true, localLoaded: true, localError: nil, hasRepositories: true)
        #expect(elsewhere == .chooseWorkspace)
    }
}
