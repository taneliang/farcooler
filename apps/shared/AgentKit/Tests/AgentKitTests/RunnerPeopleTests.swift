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

/// What People says about each person's Claude login (ov-584).
struct RunnerPeopleClaudeTests {
    static let listed = Data(
        #"""
        {"members":[
          {"handle":"alice","name":"Alice Chen","devices":[],"reps":0,"rep_panes":[],
           "claude_login":{"state":"signed_in","email":"alice@claude.example"}},
          {"handle":"bob","name":"Bob","devices":[],"reps":0,"rep_panes":[],
           "claude_login":{"state":"none","email":""}},
          {"handle":"cy","name":"Cy","devices":[],"reps":0,"rep_panes":[],
           "claude_login":{"state":"stopped_working","email":"cy@claude.example"}}
        ],"claude_account":"","you":"YOU"}
        """#.utf8)

    func people(you: String) throws -> RunnerPeople {
        try #require(RunnerPeople(json: Data(String(decoding: Self.listed, as: UTF8.self).replacingOccurrences(of: "YOU", with: you).utf8)))
    }

    @Test("The owner reads each member's state, and the email of whoever is signed in")
    func ownerLines() throws {
        let owner = try people(you: "")
        #expect(owner.claudeLine(of: owner.members[0]) == "Claude · alice@claude.example")
        #expect(owner.claudeLine(of: owner.members[1]) == "Not signed in to Claude")
        #expect(owner.claudeLine(of: owner.members[2]) == "Claude login stopped working")
    }

    @Test("A member reads other members' state alone, never an email, and nothing on their own row")
    func memberLines() throws {
        let bob = try people(you: "bob")
        #expect(bob.claudeLine(of: bob.members[0]) == "Signed in to Claude")
        #expect(!(bob.claudeLine(of: bob.members[0]) ?? "").contains("alice"))
        #expect(bob.claudeLine(of: bob.members[1]) == nil)
        #expect(bob.claudeLine(of: bob.members[2]) == "Claude login stopped working")
    }

    @Test("The CLI says whether the runner signs a member in; the phones leave it to the handshake")
    func offersSignIn() throws {
        let on = Data(#"{"members":[],"claude_account":"","you":"bob","claude_sign_in":true}"#.utf8)
        #expect(try #require(RunnerPeople(json: on)).offersClaudeSignIn)
        #expect(try !#require(RunnerPeople(json: RunnerPeopleTests.listed)).offersClaudeSignIn)
    }

    @Test("A runner from before sign-in says nothing of logins")
    func oldRunner() throws {
        let old = try #require(RunnerPeople(json: RunnerPeopleTests.listed))
        #expect(old.members[0].claudeLogin == nil && old.claudeLine(of: old.members[0]) == nil)
    }

    @Test("Only a member on their own device, on a runner that signs in, gets the Claude section")
    func ownSection() throws {
        let bob = try people(you: "bob")
        #expect(bob.ownClaudeLogin(signInOffered: true) == ClaudeLogin(state: .none))
        #expect(bob.ownClaudeLogin(signInOffered: false) == nil)
        #expect(try people(you: "alice").ownClaudeLogin(signInOffered: true)?.email == "alice@claude.example")
        #expect(try people(you: "").ownClaudeLogin(signInOffered: true) == nil)
        #expect(try people(you: "(released)").ownClaudeLogin(signInOffered: true) == nil)
        // A member the list doesn't show is signed in to nothing.
        #expect(try people(you: "dee").ownClaudeLogin(signInOffered: true) == ClaudeLogin(state: .none))
    }

    @Test("The head of People says each member has their own login, but not to a member, who has a section")
    func accountLine() throws {
        #expect(try people(you: "").accountLine(signInOffered: true) == "Each member’s agents use that member’s own Claude login.")
        #expect(try people(you: "bob").accountLine(signInOffered: true) == nil)
        // A released device is nobody's, so it reads what it read before: the account, as someone else's.
        let released = try people(you: "(released)")
        #expect(released.accountLine(signInOffered: true) == released.accountLine)
        #expect(released.accountLine(signInOffered: true) != ClaudeSignIn.ownerTrustLine)
        // Without sign-in the old line stands.
        #expect(try people(you: "").accountLine(signInOffered: false) == (try people(you: "")).accountLine)
    }
}
