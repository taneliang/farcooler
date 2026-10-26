import Foundation
import Testing

@testable import AgentKit

/// Roles on a shared runner (ov-784): who sees the role picker, who edits a
/// charter, and how a removed member's device is told.
struct RunnerPeopleRolesTests {
    /// `member.list` from a runner with roles, as Bob, a member.
    static func listed(you: String) -> Data {
        Data(
            #"""
            {"members":[
              {"handle":"alice","name":"Alice Chen","devices":[],"reps":0,"rep_panes":[],"role":"maintainer"},
              {"handle":"bob","name":"Bob","devices":[],"reps":0,"rep_panes":[],"role":"member"},
              {"handle":"erin","name":"Erin","devices":[],"reps":0,"rep_panes":[],"role":"member"}
            ],"claude_account":"","you":"\#(you)"}
            """#.utf8)
    }

    static let billing = WorkspaceSummary(
        id: "w", name: "Billing", taskPrefix: "bil", isMain: false, ordinal: 1, repository: "r", lead: "erin")

    @Test("Each member's role decodes, and a runner before roles leaves it nil")
    func decodes() throws {
        let people = try #require(RunnerPeople(json: Self.listed(you: "bob")))
        #expect(people.members.map(\.role) == [.maintainer, .member, .member])
        let old = try #require(RunnerPeople(json: Data(#"{"members":[{"handle":"a","name":"A"}],"claude_account":"","you":""}"#.utf8)))
        #expect(old.members[0].role == nil)
    }

    @Test("Only the owner is offered the picker, and only on a runner with roles")
    func picker() throws {
        let owner = try #require(RunnerPeople(json: Self.listed(you: "")))
        let alice = try #require(RunnerPeople(json: Self.listed(you: "alice")))
        let released = try #require(RunnerPeople(json: Self.listed(you: RunnerPeople.released)))
        #expect(owner.offersRoles(runnerHasRoles: true))
        #expect(!owner.offersRoles(runnerHasRoles: false))
        #expect(!alice.offersRoles(runnerHasRoles: true), "a Maintainer gives no roles")
        #expect(!released.offersRoles(runnerHasRoles: true))
    }

    @Test("The owner, a Maintainer and the workspace's lead edit its charter; a member and a removed device don't")
    func charter() throws {
        let edits = { (you: String) in try #require(RunnerPeople(json: Self.listed(you: you))).editsCharter(of: Self.billing) }
        #expect(try edits(""), "the owner")
        #expect(try edits("alice"), "a Maintainer")
        #expect(try edits("erin"), "Billing's lead")
        #expect(try !edits("bob"), "a member")
        #expect(try !edits(RunnerPeople.released), "a removed member's device")
        let ownersLead = WorkspaceSummary(
            id: "w", name: "Main", taskPrefix: "m", isMain: true, ordinal: 0, repository: "r", lead: "owner")
        #expect(try !#require(RunnerPeople(json: Self.listed(you: "erin"))).editsCharter(of: ownersLead), "leading another")
    }

    @Test("A Maintainer and the owner keep the workspaces; nobody else does")
    func keeps() throws {
        #expect(try #require(RunnerPeople(json: Self.listed(you: ""))).keepsWorkspaces)
        #expect(try #require(RunnerPeople(json: Self.listed(you: "alice"))).keepsWorkspaces)
        #expect(try !#require(RunnerPeople(json: Self.listed(you: "erin"))).keepsWorkspaces, "a lead isn't one")
        #expect(try !#require(RunnerPeople(json: Self.listed(you: RunnerPeople.released))).keepsWorkspaces)
    }

    @Test("A removed member's device is told so, and only it")
    func removed() throws {
        let released = try #require(RunnerPeople(json: Data(#"{"members":[],"claude_account":"","you":"(released)"}"#.utf8)))
        #expect(released.isReleased && !released.isMember && !released.isOwner)
        #expect(try !#require(RunnerPeople(json: Self.listed(you: "bob"))).isReleased)
        #expect(RunnerPeople.removedTitle == "You Were Removed from This Runner")
        #expect(RunnerPeople.removedTitle(from: "studio") == "You Were Removed from studio")
        #expect(RunnerPeople.removedTitle(from: "") == RunnerPeople.removedTitle)
        #expect(!RunnerPeople.removedBody.contains("owner took"), "anyone who manages the runner may have done it")
    }

    @Test("A Maintainer's row says so where no picker does, and a member's row doesn't")
    func detail() throws {
        let people = try #require(RunnerPeople(json: Self.listed(you: "")))
        #expect(RunnerPeople.detail(of: people.members[0], reps: "No rep") == "alice · Maintainer · No rep")
        #expect(RunnerPeople.detail(of: people.members[1], reps: "No rep") == "bob · No rep")
        #expect(RunnerPeople.detail(of: people.members[0], reps: "No rep", role: false) == "alice · No rep", "the picker says it")
    }
}
