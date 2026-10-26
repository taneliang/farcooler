import AgentKit
import Foundation
import Testing

@testable import Far_Cooler

/// Show Charter on another machine's runner (ov-755): the item opens a
/// read-only sheet where the runner can send the text, and the answer the
/// sheet draws from.
@MainActor
struct CharterSheetTests {
    private static let billing = WorkspaceSummary(
        id: "ws-billing", name: "Billing", taskPrefix: "bil", isMain: false, ordinal: 1, repository: "repo")

    @Test("A charter on another machine is read, not opened, when the runner can send it")
    func aRemoteCharterIsRead() {
        #expect(CharterAccess.of(Self.billing, host: "build-box", canRead: true) == .read)
        guard case .unavailable(let why) = CharterAccess.of(Self.billing, host: "build-box", canRead: false) else {
            Issue.record("an older runner's charter was offered")
            return
        }
        #expect(why.contains("build-box"))
    }

    @Test("This Mac's own charter still opens in the editor, whatever the runner can send")
    func aLocalCharterStillOpens() {
        let local = WorkspaceSummary(
            id: "ws-b", name: "Billing", taskPrefix: "bil", isMain: false, ordinal: 1, repository: "repo",
            charter: "/Users/me/charter.md")
        #expect(
            CharterAccess.of(local, host: "", canRead: true, exists: { _ in true })
                == .open(URL(fileURLWithPath: "/Users/me/charter.md")))
    }

    @Test("The read item is in the workspace's menu, and asks the CLI by id")
    func theItemAndItsArguments() {
        #expect(
            OrchestratorMenu.menu(hasSeat: false, charter: .read, wakeOnAnswer: nil) == [.showCharter])
        #expect(DaemonClient.readCharterArguments(Self.billing) == ["workspace", "charter", "ws-billing", "--json"])
    }

    /// A member can't act on the orchestrator, but reads the charter: the
    /// menu a member gets is Show Charter alone, and only where it can be
    /// read. Without the member's branch there was no menu at all.
    @Test("A member's menu is Show Charter alone, and only for a charter that can be read")
    func aMembersMenuIsTheCharterAlone() {
        func items(_ canAct: Bool, _ charter: CharterAccess?) -> [WorkspaceMenu.Item] {
            OrchestratorMenu.items(canAct: canAct, hasSeat: true, charter: charter, wakeOnAnswer: true)
        }
        #expect(items(false, .read) == [.showCharter])
        #expect(items(false, .unavailable("No charter yet.")).isEmpty)
        #expect(items(false, nil).isEmpty)
        #expect(items(true, .read) == [.replaceOrchestrator, .showCharter, .wakeOnAnswer])
    }

    @Test("The runner's answer is a charter, none yet, or nothing readable")
    func theAnswerParses() {
        let written = Data(###"{"workspace":"w","exists":true,"text":"## Who decides\nThe lead.\n"}"###.utf8)
        #expect(CharterReading.parse(written) == .text("## Who decides\nThe lead.\n"))
        #expect(CharterReading.parse(Data(#"{"workspace":"w","exists":false,"text":""}"#.utf8)) == .missing)
        #expect(CharterReading.parse(Data("not json".utf8)) == nil)
    }

    @Test("A removed member is told so, and an older runner is told to update")
    func refusalsAreSaidInTheAppsWords() {
        let removed = CharterReading.refusal("code: not-yours", workspace: Self.billing)
        #expect(removed.hasPrefix("Couldn’t read Billing’s charter."))
        #expect(removed != CharterReading.unreachable(Self.billing), "the runner answered; it wasn't unreachable")
        let older = CharterReading.refusal("This runner needs an update to show a charter.", workspace: Self.billing)
        #expect(older != CharterReading.unreachable(Self.billing))
        #expect(CharterReading.refusal(nil, workspace: Self.billing) == CharterReading.unreachable(Self.billing))
    }
}
