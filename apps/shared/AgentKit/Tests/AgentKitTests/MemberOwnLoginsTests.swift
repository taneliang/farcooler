import Foundation
import Testing

@testable import AgentKit

/// Whether members share the runner's logins, per agent (ov-652): the wire's
/// shape, and the words the Mac and the iPhone both say.
struct MemberOwnLoginsTests {
    static func listed(_ ownLogins: String?, you: String = "") -> Data {
        let own = ownLogins.map { #","own_logins":\#($0)"# } ?? ""
        return Data(
            #"{"members":[{"handle":"bob","name":"Bob","devices":[]}],"claude_account":"owner@example.com","you":"\#(you)","claude_sign_in":true\#(own)}"#
                .utf8)
    }

    @Test("The switches decode from member.list, and a runner before them sends none")
    func decodes() throws {
        let people = try #require(
            RunnerPeople(json: Self.listed(#"{"claude":false,"codex":true,"cursor":false,"may_change":true}"#)))
        #expect(people.ownLogins == MemberOwnLogins(claude: false, codex: true, cursor: false, mayChange: true))
        #expect(people.ownLogins?.requiresOwn(.codex) == true)
        #expect(people.ownLogins?.requiresOwn(.claude) == false)
        #expect(people.ownLogins?.setting(.claude, true).requiresOwn(.claude) == true)
        #expect(try #require(RunnerPeople(json: Self.listed(nil))).ownLogins == nil)
    }

    @Test("Each switch says what it means, on and off")
    func words() {
        #expect(MemberOwnLogins.sectionTitle == "Member Logins")
        #expect(MemberOwnLogins.Agent.allCases.map(MemberOwnLogins.toggleTitle) == [
            "Require Own Claude Account", "Require Own Codex Account", "Require Own Cursor Account",
        ])
        #expect(MemberOwnLogins.stateLine(.claude, on: false) == "Members who haven’t signed in use this runner’s Claude login.")
        #expect(MemberOwnLogins.stateLine(.claude, on: true) == "Each member signs in to Claude here with their own account.")
        #expect(MemberOwnLogins.stateLine(.codex, on: false) == "Members use this runner’s Codex login.")
        #expect(
            MemberOwnLogins.stateLine(.cursor, on: true)
                == "Members can’t use Cursor here until they can connect their own account.")
    }

    @Test("The footer says a shared login is the owner's account, quota and history, and who may change it")
    func footer() {
        #expect(
            MemberOwnLogins.footer(owner: true, mayChange: true)
                == "A member on this runner’s login uses your account, its quota and its history, including "
                + "Claude’s memory, Codex sessions and Cursor chats. A change applies the next time each pane starts.")
        let member = MemberOwnLogins.footer(owner: false, mayChange: false)
        #expect(member.contains("uses the runner owner’s account, its quota and its history"))
        #expect(member.hasSuffix(" Only this runner’s admins can change these."))
    }

    @Test("The owner's head of People says members share their Claude account while Claude's switch is off")
    func ownerLine() throws {
        let shared = try #require(RunnerPeople(json: Self.listed(#"{"claude":false,"codex":false,"cursor":false,"may_change":true}"#)))
        #expect(
            shared.accountLine(signInOffered: true)
                == "Members who haven’t signed in to Claude here use your Claude account, owner@example.com.")
        #expect(ClaudeSignIn.ownerSharedLine(email: "") == "Members who haven’t signed in to Claude here use your Claude account.")
        let separate = try #require(RunnerPeople(json: Self.listed(#"{"claude":true,"codex":false,"cursor":false,"may_change":true}"#)))
        #expect(separate.accountLine(signInOffered: true) == ClaudeSignIn.ownerTrustLine)
        let before = try #require(RunnerPeople(json: Self.listed(nil)))
        #expect(before.accountLine(signInOffered: true) == ClaudeSignIn.ownerTrustLine, "a runner before ov-652 required its members' own")
        let member = try #require(RunnerPeople(json: Self.listed(#"{"claude":false,"codex":false,"cursor":false,"may_change":false}"#, you: "bob")))
        #expect(member.accountLine(signInOffered: true) == nil, "a member's Claude section says it")
    }
}
