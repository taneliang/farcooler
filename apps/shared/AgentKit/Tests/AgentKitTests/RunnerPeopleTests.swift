import Foundation
import Testing

@testable import AgentKit

/// Who's on a shared runner, and the one line that says whose Claude account
/// the work runs on (ov-508).
struct RunnerPeopleTests {
    /// `farcooler member list --json`, as the CLI prints it and the phones'
    /// bridge answers it (`members_json`).
    static let listed = Data(
        #"""
        {"members":[
          {"handle":"alice","name":"Alice Chen","devices":["phone-a"],"reps":2,
           "rep_panes":[{"terminal":"t1","workspace":"w-billing"},{"terminal":"t2","workspace":"w-main"}]},
          {"handle":"bob","name":"Bob","devices":[],"reps":0,"rep_panes":[]}
        ],"claude_account":"team@example.com","you":"bob"}
        """#.utf8)

    @Test("The list decodes each member with their reps' boards, the account and who's asking")
    func decodes() throws {
        let people = try #require(RunnerPeople(json: Self.listed))
        #expect(people.members.map(\.name) == ["Alice Chen", "Bob"])
        #expect(people.members[0].reps == [.init(terminal: "t1", workspace: "w-billing"), .init(terminal: "t2", workspace: "w-main")])
        #expect(people.claudeAccount == "team@example.com")
        #expect(people.isMember && people.isShared)
        #expect(people.isYou(people.members[1]) && !people.isYou(people.members[0]))
    }

    @Test("A list from a runner before ov-508, with a count and no panes, still decodes")
    func decodesWithoutPanes() throws {
        let old = Data(#"{"members":[{"handle":"a","name":"A","devices":[],"reps":1}],"claude_account":"","you":""}"#.utf8)
        let people = try #require(RunnerPeople(json: old))
        #expect(people.members[0].reps.isEmpty)
        #expect(RunnerPeople(json: Data("{}".utf8)) == nil)
    }

    @Test("The account line names the account for a member, says yours for the owner, and never a blank")
    func accountLine() {
        let member = RunnerPeople(members: [], claudeAccount: "alice@example.com", you: "bob")
        #expect(member.accountLine == "Work here runs on alice@example.com’s Claude account.")
        let owner = RunnerPeople(members: [.init(handle: "bob", name: "Bob")], claudeAccount: "alice@example.com", you: "")
        #expect(owner.accountLine == "Work here runs on your Claude account, alice@example.com.")
        // Below Control the runner doesn't say whose, and the line still says
        // the account is the runner's, not the reader's.
        let looking = RunnerPeople(members: [], claudeAccount: "", you: "bob")
        #expect(looking.accountLine == "Work here runs on the Claude account signed in on this runner.")
    }

    @Test("Only a runner with someone besides the owner is shared; a released device is nobody")
    func shared() {
        #expect(!RunnerPeople(members: [], claudeAccount: "a@b.c", you: "").isShared)
        #expect(RunnerPeople(members: [], claudeAccount: "a@b.c", you: "bob").isShared)
        let released = RunnerPeople(members: [.init(handle: "bob", name: "Bob")], claudeAccount: "", you: "(released)")
        #expect(!released.isMember && released.isShared)
        #expect(released.accountLine == "Work here runs on the Claude account signed in on this runner.")
        // A released device keeps Control, so it's told the email; it's not
        // the owner's, so it isn't told the account is its own.
        let releasedAtControl = RunnerPeople(members: [], claudeAccount: "alice@example.com", you: "(released)")
        #expect(releasedAtControl.accountLine == "Work here runs on alice@example.com’s Claude account.")
    }

    @Test("A member's reps are named by board, and counted when a board isn't known")
    func repLine() {
        let names = ["w-billing": "Billing", "w-main": "Main"]
        func line(_ reps: [String]) -> String {
            let member = RunnerPeople.Member(
                handle: "a", name: "A", reps: reps.enumerated().map { .init(terminal: "t\($0.offset)", workspace: $0.element) })
            return RunnerPeople.reps(of: member, workspaceNames: names)
        }
        #expect(line([]) == "No rep")
        #expect(line(["w-billing"]) == "Rep in Billing")
        #expect(line(["w-billing", "w-main"]) == "Reps in Billing and Main")
        #expect(line([""]) == "Has a rep")
        #expect(line(["w-billing", "gone"]) == "Has 2 reps")
    }

    @Test("A teammate added by QR carries their directory account and email, and the row says the email")
    func teammate() throws {
        let listed = Data(
            #"""
            {"members":[{"handle":"bob","name":"Bob Smith","devices":["farcooler-0a0b"],"reps":0,"rep_panes":[],
              "account":"user_bob","email":"bob@example.com"},
             {"handle":"carol","name":"Carol","devices":[],"reps":0,"rep_panes":[],"account":"","email":""}],
             "claude_account":"","you":""}
            """#.utf8)
        let people = try #require(RunnerPeople(json: listed))
        let bob = people.members[0]
        #expect((bob.account, bob.email, bob.devices) == ("user_bob", "bob@example.com", ["farcooler-0a0b"]))
        #expect(RunnerPeople.detail(of: bob, reps: "No rep") == "bob · bob@example.com · No rep")
        #expect(RunnerPeople.detail(of: people.members[1], reps: "No rep") == "carol · No rep")
        // A runner from before ov-525 sends neither field.
        let old = try #require(RunnerPeople(json: Self.listed))
        #expect(old.members[0].account.isEmpty && old.members[0].email.isEmpty)
        #expect(old.members[0].devices == ["phone-a"])
    }
}
