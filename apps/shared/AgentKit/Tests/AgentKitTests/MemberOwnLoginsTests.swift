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
                == "Members can’t open Cursor panes here until they can connect their own account.")
    }

    @Test("The footer says a shared login is the owner's account, quota, history and shell keys, but not their connected apps")
    func footer() {
        #expect(
            MemberOwnLogins.footer(owner: true, mayChange: true)
                == "A member on this runner’s login uses your account, its quota and its history, including "
                + "Claude’s memory, Codex sessions and Cursor chats, and any API keys your shell exports. Their Claude "
                + "and Codex panes don’t get your connected apps or MCP servers, but Cursor panes do get your MCP "
                + "servers. A member’s shell uses this runner’s Claude login only while all three are off. Codex’s "
                + "switch also keeps members’ shells and Claude panes off this runner’s Codex login, but Cursor run "
                + "from a member’s shell still uses this runner’s Cursor login. A change applies the next time each "
                + "pane starts.")
        let member = MemberOwnLogins.footer(owner: false, mayChange: false)
        #expect(member.contains("uses the runner owner’s account, its quota and its history"))
        #expect(member.contains("any API keys the owner’s shell exports") && member.contains("the owner’s connected apps"))
        #expect(member.hasSuffix(" Only this runner’s admins can change these."))
    }

    @Test("A member's Claude section says whose account their panes use until they sign in, and what a sign-out does")
    func memberCopy() throws {
        let shared = try #require(
            RunnerPeople(json: Self.listed(#"{"claude":false,"codex":false,"cursor":false,"may_change":false}"#, you: "bob")))
        #expect(shared.sharedClaudeAccount == "owner@example.com")
        #expect(
            ClaudeSignIn.notSignedInBody(sharedAccount: shared.sharedClaudeAccount)
                == "Until you sign in, your rep and the lanes you start use this runner’s own Claude account, "
                + "owner@example.com. Sign in to use your own account and its connectors.")
        #expect(ClaudeSignIn.notSignedInBody(sharedAccount: "").contains("use this runner’s own Claude account. Sign in"))
        #expect(
            ClaudeSignIn.signOutMessage(shared: true)
                == "Your rep and lanes there use this runner’s own Claude account from the next time each starts.")
        let separate = try #require(
            RunnerPeople(json: Self.listed(#"{"claude":true,"codex":false,"cursor":false,"may_change":false}"#, you: "bob")))
        #expect(separate.sharedClaudeAccount == nil)
        #expect(ClaudeSignIn.notSignedInBody(sharedAccount: nil) == ClaudeSignIn.notSignedInBody)
        #expect(ClaudeSignIn.signOutMessage(shared: false) == ClaudeSignIn.signOutMessage)
        #expect(try #require(RunnerPeople(json: Self.listed(nil, you: "bob"))).sharedClaudeAccount == nil)
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
