import AgentKit
import Foundation
import Testing

@testable import Far_Cooler

/// Every Mac refusal says what AgentKit's `RunnerRefusal` says (ov-160).
///
/// Seven tables once spelled their own English for the same code words, and
/// `scope-denied` sent a start to "Check that it’s reachable". The loop below
/// is over `RunnerRefusal.allCases`, so a word added there is checked against
/// every table without anyone remembering to; a table that goes back to its own
/// sentence for a word fails by the table's name and the word's.
@MainActor
struct RefusalCopyTests {
    private static let worktree = Worktree(
        id: "w-1", short: "w1", task: "fix-it", branch: "fix-it", repository: "repo",
        host: "", path: "/tmp/fix-it", state: "active", terminals: [])
    private static let billing = WorkspaceSummary(
        id: "0198f2c0-0000-7000-8000-0000000000dd", name: "Billing", taskPrefix: "bil", isMain: false,
        ordinal: 1, repository: "0198f2c0-0000-7000-8000-0000000000aa", orchestrator: nil)

    private static func stderr(_ word: String) -> String { "error: whatever the runner said\ncode: \(word)" }

    /// Words a table says its own way, and why. Everything else must carry the
    /// shared sentence.
    private static let ownSentence: [String: Set<RunnerRefusal>] = [
        // The start panel picks the name itself, so "pick another name" would
        // point at a field that isn't there.
        "start": [.branchExists, .worktreeExists],
        // The one argument a person types into New Workspace is its prefix.
        "create workspace": [.invalidArgument],
    ]

    @Test(arguments: RunnerRefusal.allCases)
    func everyTableSaysTheSharedSentence(_ refusal: RunnerRefusal) async {
        let word = refusal.rawValue
        let message = Self.stderr(word)
        let client = DaemonClient(target: "", notifications: NotificationCenter())
        client.commandRunnerForTesting = { _ in (nil, message) }
        let made = await client.createWorkspace(repository: "repo", name: "Billing", prefix: "").refusal

        let said: [(table: String, sentence: String?)] = [
            ("start", TaskFailure.sentence(for: message)),
            ("banner", ActionCopy.reason(message)),
            ("create workspace", made),
            ("assign", DaemonClient.assignRefusal(message, worktree: Self.worktree, workspace: Self.billing)),
            ("wake on answer", DaemonClient.wakeOnAnswerRefusal(message, workspace: Self.billing)),
            (
                "orchestrator",
                DaemonClient.orchestratorRefusal(message, workspace: Self.billing, replace: false)
            ),
            ("role", OrchestratorAdoption.refusal(message, terminal: "claude", workspace: "Billing")),
        ]
        for (table, sentence) in said {
            if Self.ownSentence[table]?.contains(refusal) == true { continue }
            #expect(
                sentence?.contains(refusal.sentence) == true,
                "\(table) says \(sentence ?? "nothing") for \(word)")
        }
    }

    /// A runner that answered is never said to be unreachable, and a link that
    /// dropped is.
    @Test func onlyNoWordMeansUnreachable() {
        for table in [
            { TaskFailure.sentence(for: $0) },
            { DaemonClient.assignRefusal($0, worktree: Self.worktree, workspace: Self.billing) },
            { DaemonClient.wakeOnAnswerRefusal($0, workspace: Self.billing) },
            { DaemonClient.orchestratorRefusal($0, workspace: Self.billing, replace: false) },
            { OrchestratorAdoption.refusal($0, terminal: "claude", workspace: "Billing") },
        ] as [(String?) -> String] {
            #expect(!table(Self.stderr("some-word-from-a-newer-runner")).contains("reachable"))
            #expect(table("ssh: connect to host box port 22: Connection refused").contains("reachable"))
        }
    }

    /// The confirmation flows read the code word, not the CLI's English.
    @Test func confirmationIsTheCodeWordNotTheWording() {
        #expect(TaskFailure.isConfirmationRequired("error: x\ncode: confirmation-required"))
        #expect(!TaskFailure.isConfirmationRequired("error: confirmation needed, they said"))
        #expect(!TaskFailure.isConfirmationRequired("error: x\ncode: resource-conflict"))
    }
}
