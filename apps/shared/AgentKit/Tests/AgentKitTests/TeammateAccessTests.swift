import Foundation
import Testing

@testable import AgentKit

/// Add Teammate with a person and an access (ov-856), and the Access menu and
/// Limit to Viewer on a phone: what each caller is offered, what is sent, and
/// what a refusal says.
struct TeammateAccessTests {
    /// A runner where the asker is `you` ("" is the Owner, "dan" an Admin,
    /// "bob" a Member): Dan (Admin) and Bob (Member, account user_bob) and
    /// Cy (added by hand, no account).
    static func members(you: String) -> Data {
        Data(
            #"""
            {"members":[
              {"handle":"dan","name":"Dan","devices":["phone-d"],"access":"admin","account":"user_dan"},
              {"handle":"bob","name":"Bob","devices":["phone-b"],"access":"member","account":"user_bob"},
              {"handle":"cy","name":"Cy","devices":[],"access":"viewer","account":""}
            ],"claude_account":"","you":"\#(you)",
            "device_access":[
              {"client_id":"phone-o","person":"","access":"owner","cap":"","check":false,"step_up":"trusted","step_up_code":"","step_up_at":5},
              {"client_id":"phone-d","person":"dan","access":"admin","cap":"","check":false,"step_up":"trusted","step_up_code":"","step_up_at":5},
              {"client_id":"phone-b","person":"bob","access":"member","cap":"","check":false,"step_up":"","step_up_code":"","step_up_at":0}
            ]}
            """#.utf8)
    }

    static func facts(you: String, device: String, account: String = "user_bob") throws -> TeammateRunnerFacts {
        try #require(TeammateRunnerFacts(members: members(you: you), account: account, thisDevice: device))
    }

    private func choice(access: PersonAccess = .member, person: String? = nil) -> TeammateChoice {
        var choice = TeammateChoice(
            who: CeremonyTeammate(account: "user_eve", email: "eve.jones@example.com", team: "Platform"),
            directory: "{}", scanner: "user_alice", grantedBy: "alice@example.com")
        choice.access = access
        choice.person = person
        return choice
    }

    // MARK: Who the caller is, who is there

    @Test("The Owner, an Admin and a Member are told apart, with the people the device could join")
    func facts() throws {
        #expect(try Self.facts(you: "", device: "phone-o").caller == .owner)
        let admin = try Self.facts(you: "dan", device: "phone-d")
        #expect(admin.caller == .admin && admin.signs)
        #expect(try Self.facts(you: "bob", device: "phone-b").caller == .member)
        // Bob's account matches Bob; Cy has none, so the device could join Cy too; Dan's is another's.
        #expect(admin.candidates.map(\.handle) == ["bob", "cy"] && admin.matching == "bob")
        #expect(admin.taken == ["dan", "bob", "cy"])
        #expect(try Self.facts(you: "", device: "phone-o", account: "user_new").matching == nil)
    }

    @Test("Only an Owner or an Admin can tick a runner, and the sentence says who can")
    func blocked() throws {
        let owner = try Self.facts(you: "", device: "phone-o")
        let member = try Self.facts(you: "bob", device: "phone-b")
        #expect(TeammateAccessRules.blocked(owner, runner: "studio") == nil)
        #expect(TeammateAccessRules.blocked(member, runner: "studio") == "Only the Owner or an Admin can add people to studio.")
        #expect(TeammateAccessRules.blocked(nil, runner: "studio") == "Update Far Cooler on studio to add people from here.")
    }

    @Test("Admin is offered only when the Owner is adding on every runner")
    func adminIsTheOwnersToGive() {
        #expect(TeammateAccessRules.choices(callers: [.owner, .owner]) == [.admin, .member, .viewer])
        #expect(TeammateAccessRules.choices(callers: [.owner, .admin]) == [.member, .viewer])
        #expect(TeammateAccessRules.choices(callers: [.admin]) == [.member, .viewer])
    }

    // MARK: What is sent

    @Test("A New Person is sent with the access chosen and a handle no one has; Viewer is a read line")
    func newPerson() throws {
        let facts = TeammateRunnerFacts(caller: .owner, taken: ["eve-jones"])
        let args = TeammateAccessRules.arguments(
            publicKey: "KEY", label: "Eve’s iPhone", clientId: "phone-e", nodeKey: "N", teammate: choice(access: .viewer), facts: facts)
        let person = try #require(args["teammate"] as? [String: Any])
        #expect(person["handle"] as? String == "eve-jones-2")
        #expect(person["access"] as? String == "viewer" && person["account"] as? String == "user_eve")
        #expect(args["scope"] as? String == "read" && args["forTeammate"] as? Bool == true)
        let admin = TeammateAccessRules.arguments(
            publicKey: "KEY", label: "x", clientId: "p", nodeKey: "", teammate: choice(access: .admin), facts: facts)
        #expect(admin["scope"] as? String == "control", "an Admin's line is control, never host_admin")
        #expect((admin["teammate"] as? [String: Any])?["access"] as? String == "admin")
        #expect(args["shellAccess"] == nil)
    }

    @Test("An existing person is sent by their handle with the access they have")
    func existingPerson() throws {
        let facts = try Self.facts(you: "", device: "phone-o", account: "user_bob")
        let args = TeammateAccessRules.arguments(
            publicKey: "K", label: "x", clientId: "p", nodeKey: "", teammate: choice(access: .admin, person: "bob"), facts: facts)
        let person = try #require(args["teammate"] as? [String: Any])
        #expect(person["handle"] as? String == "bob" && person["name"] as? String == "Bob")
        #expect(person["access"] as? String == "member", "they keep what they have, whatever the menu said")
        #expect(args["scope"] as? String == "control")
    }

    // MARK: Words

    @Test("The prompt and the refusals are Apple-style, and the add limit says five a day")
    func words() {
        #expect(TeammateAccessRules.reason(device: "Eve’s iPhone", runner: "studio", access: "member") == "Add Eve’s iPhone to studio as a Member.")
        #expect(TeammateAccessRules.reason(device: "Eve’s iPad", runner: "studio", access: "admin") == "Add Eve’s iPad to studio as an Admin.")
        #expect(
            TeammateAccessRules.refusal("add_limit", runner: "studio", device: "iPhone")
                == "You can add five devices a day from this iPhone. Try again tomorrow.")
        #expect(
            TeammateAccessRules.refusal("admin_is_owners", runner: "studio", device: "iPhone")
                == "Only the Owner can make someone an Admin.")
        #expect(
            TeammateAccessRules.refusal("not_admin", runner: "studio", device: "iPhone")
                == "Only the Owner or an Admin can add people to studio.")
        #expect(StepUpReason.limit("Cy’s iPad", runner: "studio") == "Limit Cy’s iPad on studio to Viewer.")
        let bob = ExistingPerson(handle: "bob", name: "Bob", access: .member)
        #expect(TeammateAccessRules.keeps(bob, runner: "studio") == "Bob is already on studio as a Member, and keeps that access.")
        for word in ["add_limit", "admin_is_owners", "access_exists", "handle_taken", "scope_access", "not_admin"] {
            let said = TeammateAccessRules.refusal(word, runner: "studio", device: "iPhone")
            #expect(!said.contains("_") && !said.contains("Err("), "\(word): \(said)")
        }
    }

    // MARK: The Access menu and Limit to Viewer

    @Test("The Access menu offers what the runner would allow, by who is asking")
    func accessMenu() throws {
        let owner = try #require(PeopleAccess(members: Self.members(you: ""), thisDevice: "phone-o"))
        let dan = try #require(owner.people.first { $0.handle == "dan" })
        let bob = try #require(owner.people.first { $0.handle == "bob" })
        #expect(owner.accessChoices(for: dan) == [.admin, .member, .viewer])
        #expect(owner.accessChoices(for: owner.people[0]).isEmpty, "nobody changes the Owner")

        let admin = try #require(PeopleAccess(members: Self.members(you: "dan"), thisDevice: "phone-d"))
        #expect(admin.accessChoices(for: try #require(admin.people.first { $0.handle == "bob" })) == [.member, .viewer])
        #expect(admin.accessChoices(for: try #require(admin.people.first { $0.handle == "dan" })) == [.member, .viewer], "themselves, down")
        #expect(admin.accessChoices(for: owner.people[0]).isEmpty)

        let member = try #require(PeopleAccess(members: Self.members(you: "bob"), thisDevice: "phone-b"))
        #expect(member.accessChoices(for: dan).isEmpty && member.accessChoices(for: bob).isEmpty)
    }

    @Test("Limit to Viewer is offered on a device the caller manages")
    func limit() throws {
        let admin = try #require(PeopleAccess(members: Self.members(you: "dan"), thisDevice: "phone-d"))
        let device = { (id: String) in admin.people.flatMap(\.devices).first { $0.id == id }! }
        #expect(admin.mayLimit(device("phone-b")), "a Member's")
        #expect(admin.mayLimit(device("phone-d")), "their own")
        #expect(!admin.mayLimit(device("phone-o")), "the Owner's")
        let owner = try #require(PeopleAccess(members: Self.members(you: ""), thisDevice: "phone-o"))
        #expect(owner.mayLimit(owner.people.flatMap(\.devices).first { $0.id == "phone-d" }!))
        let member = try #require(PeopleAccess(members: Self.members(you: "bob"), thisDevice: "phone-b"))
        #expect(!member.mayLimit(member.people.flatMap(\.devices).first { $0.id == "phone-b" }!))
    }
}
