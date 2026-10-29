import Foundation
import Testing

@testable import AgentKit

/// The People screen's rows (ov-856, §8): each person with their Access,
/// their devices with who added them, a Viewer limit, a shell warning, and
/// the two sections that appear only when they have something in them.
struct PeopleAccessTests {
    /// The owner's view of a runner: Cosmo (Member) has an iPhone that added
    /// a pad, and the iPad is held at Viewer; Dan (Admin) has a phone that is
    /// waiting for approval and a Mac with a shell; a legacy line needs a look.
    static let members = Data(
        #"""
        {"members":[
          {"handle":"cosmo","name":"Cosmo","devices":["phone-c","pad-c"],"access":"member"},
          {"handle":"dan","name":"Dan","devices":["phone-d","mac-d"],"access":"admin"}
        ],"claude_account":"","you":"",
        "device_access":[
          {"client_id":"mac-o","person":"","access":"owner","cap":"","check":false,"step_up":"trusted","step_up_code":"","step_up_at":5},
          {"client_id":"legacy-t","person":"","access":"owner","cap":"member","check":true,"step_up":"","step_up_code":"","step_up_at":0},
          {"client_id":"phone-c","person":"cosmo","access":"member","cap":"","check":false,"step_up":"trusted","step_up_code":"","step_up_at":5},
          {"client_id":"pad-c","person":"cosmo","access":"member","cap":"viewer","check":false,"step_up":"","step_up_code":"","step_up_at":0},
          {"client_id":"phone-d","person":"dan","access":"admin","cap":"","check":false,"step_up":"proposed","step_up_code":"418207","step_up_at":9},
          {"client_id":"mac-d","person":"dan","access":"admin","cap":"","check":false,"step_up":"expired","step_up_code":"","step_up_at":1}
        ]}
        """#.utf8)

    static let clients = Data(
        #"""
        {"clients":[
          {"clientId":"phone-c","label":"farcooler-Cosmos-iPhone-t7xq9vd8","shellAccess":false,"foreign":false,"addedBy":""},
          {"clientId":"pad-c","label":"farcooler-iPad-abcd1234","shellAccess":false,"foreign":false,"addedBy":"phone-c"},
          {"clientId":"phone-d","label":"farcooler-iPhone-k3k3k3k3","shellAccess":false,"foreign":false,"addedBy":""},
          {"clientId":"mac-d","label":"farcooler-Dans-Mac-zzzz9999","shellAccess":false,"foreign":false,"addedBy":""},
          {"clientId":"mac-d","label":"farcooler-shell-Dans-Mac-zzzz9999.mac-d","shellAccess":true,"foreign":false,"addedBy":""},
          {"clientId":"","label":"someone@laptop","shellAccess":false,"foreign":true,"addedBy":""}
        ]}
        """#.utf8)

    func board(you: String = "") throws -> PeopleAccess {
        try #require(PeopleAccess(members: Self.members, clients: Self.clients, thisDevice: you))
    }

    @Test("Each person says their Access, and the owner comes first")
    func access() throws {
        let people = try board().people
        #expect(people.map(\.access) == [.owner, .member, .admin])
        #expect(people.map(\.name) == ["You", "Cosmo", "Dan"])
        #expect(people[0].isYou && !people[1].isYou)
    }

    @Test("A device is named as a person would, and says who added it")
    func devices() throws {
        let cosmo = try #require(try board().people.first { $0.handle == "cosmo" })
        #expect(cosmo.devices.map(\.name) == ["Cosmos-iPhone", "iPad"])
        #expect(cosmo.devices[0].addedBy == nil)
        #expect(cosmo.devices[1].addedBy == "Added by Cosmos-iPhone")
    }

    @Test("A device held at Viewer wears the badge; one that isn't doesn't")
    func viewerBadge() throws {
        let cosmo = try #require(try board().people.first { $0.handle == "cosmo" })
        #expect(cosmo.devices.map(\.viewerOnThisDevice) == [false, true])
    }

    @Test("A plain line beside a device's own is a shell, and only that device has it")
    func shell() throws {
        let dan = try #require(try board().people.first { $0.handle == "dan" })
        #expect(dan.devices.map(\.hasShell) == [false, true])
        #expect(PeopleAccess.shellWarning(runner: "studio") == "This device has a shell on studio, so it can do everything.")
    }

    @Test("Removing a device counts what it added, and those they added")
    func removal() throws {
        let cosmo = try #require(try board().people.first { $0.handle == "cosmo" })
        #expect(cosmo.devices.map(\.removal) == [2, 1])
    }

    @Test("The two sections hold exactly the devices that belong in them")
    func sections() throws {
        let b = try board()
        #expect(b.toCheck.map(\.id) == ["legacy-t"])
        #expect(b.waiting.map(\.id) == ["phone-d"], "an expired proposal is not waiting: the device has to ask again")
        #expect(b.waiting[0].code == "418207")
    }

    @Test("A device may approve another of the same person's, once it has a trusted key")
    func approvers() throws {
        let b = try board(you: "phone-c")
        let theirs = try #require(b.waiting.first)
        #expect(!b.mayApprove(theirs, from: b.thisDevice), "Cosmo's phone can't approve Dan's")
        let dan = try #require(PeopleAccess(members: Self.members, clients: Self.clients, thisDevice: "mac-d"))
        #expect(!dan.mayApprove(theirs, from: dan.thisDevice), "an expired key doesn't sign")
        let sameOwner = PeopleAccess.Device(
            id: "phone-d2", name: "x", addedBy: nil, viewerOnThisDevice: false, hasShell: false, needsCheck: false,
            stepUp: .trusted, code: "", added: 0, person: "dan", personName: "Dan")
        #expect(b.mayApprove(theirs, from: sameOwner))
        #expect(!b.mayApprove(theirs, from: nil))
    }

    @Test("Allow shows on the owner's Mac for any request, and on a phone only for its own person's other device")
    func allowOnlyWhereTheRunnerWillTakeIt() throws {
        let theirs = try #require(try board().waiting.first)
        let mac = try #require(PeopleAccess(members: Self.members, clients: Self.clients, isSocket: true))
        #expect(mac.shownWaiting.map(\.id) == ["phone-d"] && mac.mayAllow(theirs), "the socket takes the owner's word")
        // A phone whose own id the runner doesn't list has no device row, and
        // that is not the socket: it must offer no Allow, and list no one's.
        let unlisted = try board(you: "not-in-the-list")
        #expect(unlisted.thisDevice == nil && !unlisted.isSocket)
        #expect(!unlisted.mayAllow(theirs) && unlisted.shownWaiting.isEmpty)
        let cosmo = try board(you: "phone-c")
        #expect(!cosmo.mayAllow(theirs) && cosmo.shownWaiting.isEmpty, "Dan's request is not Cosmo's phone's to list or allow")
        #expect(PeopleAccess.waitingFooter(isSocket: true) != PeopleAccess.waitingFooter(isSocket: false))
    }

    @Test("The approval sheet reads the code in two halves and says to hold the device")
    func approvalWords() throws {
        let waiting = try #require(try board().waiting.first)
        #expect(PeopleAccess.spoken("418207") == "418 207")
        #expect(
            PeopleAccess.approvalPrompt(for: waiting)
                == "Only allow this if you’re holding iPhone right now. Check that it shows 418 207.")
    }

    @Test("What a menu offers depends on who is choosing")
    func choices() {
        #expect(PersonAccess.choices(for: .owner) == [.admin, .member, .viewer])
        #expect(PersonAccess.choices(for: .admin) == [.member, .viewer], "an Admin never makes an Admin")
        #expect(PersonAccess.choices(for: .member).isEmpty)
        #expect(PersonAccess.admin.summary == "Can add and remove people and their devices.")
    }

    @Test("A runner before person access has no Access to show")
    func oldRunner() throws {
        let old = try #require(
            PeopleAccess(members: Data(#"{"members":[{"handle":"a","name":"A","devices":["x"]}],"claude_account":"","you":"a"}"#.utf8)))
        #expect(!old.recorded && old.toCheck.isEmpty && old.waiting.isEmpty)
        #expect(old.people.map(\.name) == ["A"])
        #expect(PeopleAccess(members: Data("not json".utf8)) == nil)
    }
}
