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

    /// ov-974: the owner's devices whose keys their Mac has yet to approve
    /// with Touch ID, each with its code, listed on the Mac alone. A member's,
    /// one vouched for, and one with no code (a runner from before) aren't.
    @Test("The Mac lists the owner's devices to approve for the cutover")
    func toVouch() throws {
        let members = Data(
            #"""
            {"members":[{"handle":"cosmo","name":"Cosmo","devices":["phone-c"],"access":"member"}],"claude_account":"","you":"",
            "device_access":[
              {"client_id":"phone-o","person":"","access":"owner","cap":"","check":false,"step_up":"trusted","step_up_code":"418207","step_up_at":5},
              {"client_id":"pad-o","person":"","access":"owner","cap":"","check":false,"step_up":"trusted","step_up_code":"","step_up_at":5,"step_up_vouched":true},
              {"client_id":"old-o","person":"","access":"owner","cap":"","check":false,"step_up":"trusted","step_up_code":"","step_up_at":5},
              {"client_id":"phone-c","person":"cosmo","access":"member","cap":"","check":false,"step_up":"trusted","step_up_code":"123456","step_up_at":5}
            ]}
            """#.utf8)
        let mac = try #require(PeopleAccess(members: members, isSocket: true))
        #expect(mac.toVouch.map(\.id) == ["phone-o"])
        #expect(mac.toVouch.first?.code == "418207")
        let pad = try #require(mac.people.first?.devices.first { $0.id == "pad-o" })
        #expect(pad.vouched && !pad.needsVouch)
        let phone = try #require(PeopleAccess(members: members, thisDevice: "phone-o"))
        #expect(phone.toVouch.isEmpty, "a phone lists none: only the Mac approves with Touch ID")
        #expect(phone.thisDevice?.needsVouch == true, "the phone shows its own code until then")
        #expect(PeopleAccess.vouchPrompt(for: try #require(mac.toVouch.first)).contains("418 207"))
    }

    /// ov-982: each device to approve says when and how it was paired,
    /// and one paired outside Far Cooler is flagged. The row shape is
    /// the one `members_json`'s test pins on the runner's side.
    @Test("Each device says how and when it was paired")
    func pairedFacts() throws {
        let members = Data(
            #"""
            {"members":[],"claude_account":"","you":"",
            "device_access":[
              {"client_id":"phone-o","person":"","access":"owner","cap":"","check":false,"step_up":"trusted","step_up_code":"","step_up_at":5,"step_up_vouched":true,"paired_at":1731000000000,"paired_via":"app","paired_by":""},
              {"client_id":"cli-o","person":"","access":"owner","cap":"","check":false,"step_up":"trusted","step_up_code":"418207","step_up_at":5,"paired_at":1731000000000,"paired_via":"terminal","paired_by":""},
              {"client_id":"pad-o","person":"","access":"owner","cap":"","check":false,"step_up":"trusted","step_up_code":"221133","step_up_at":5,"paired_at":1731000000000,"paired_via":"device","paired_by":"phone-o"},
              {"client_id":"old-o","person":"","access":"owner","cap":"","check":false,"step_up":"trusted","step_up_code":"990011","step_up_at":5,"paired_at":0,"paired_via":"","paired_by":""}
            ]}
            """#.utf8)
        let mac = try #require(PeopleAccess(members: members, isSocket: true))
        let by = Dictionary(uniqueKeysWithValues: mac.people.flatMap(\.devices).map { ($0.id, $0) })
        let cli = try #require(by["cli-o"]), pad = try #require(by["pad-o"]), old = try #require(by["old-o"])
        #expect(cli.pairedVia == .terminal && cli.pairedOutsideApp)
        #expect(cli.pairedAt == Date(timeIntervalSince1970: 1_731_000_000))
        #expect(pad.pairedVia == .device && !pad.pairedOutsideApp)
        #expect(pad.pairedBy != nil && pad.pairedBy != "", "the phone that added it is named")
        #expect(old.pairedVia == nil && old.pairedAt == nil && !old.pairedOutsideApp,
                "a device from before the record is not flagged: nothing says it came from outside")
        #expect(PeopleAccess.pairedLine(for: cli, runner: "Mac").contains("terminal"))
        #expect(PeopleAccess.pairedLine(for: pad, runner: "Mac").hasPrefix("Added from "))
        #expect(PeopleAccess.pairedLine(for: old, runner: "Mac").contains("before Far Cooler kept a record"))
        #expect(PeopleAccess.pairedOutsideConfirmTitle(for: cli).hasSuffix("Anyway?"))
    }

    /// ov-989: the model a device said at pairing is on its row, and leads the line under its name.
    @Test("The approve list shows the model a device said")
    func modelShown() throws {
        let members = Data(
            #"""
            {"members":[],"claude_account":"","you":"",
            "device_access":[
              {"client_id":"pad-o","person":"","access":"owner","cap":"","check":false,"step_up":"proposed","step_up_code":"221133","step_up_at":5,"paired_at":1731000000000,"paired_via":"app","paired_by":"","model":"iPhone 17 Pro"},
              {"client_id":"old-o","person":"","access":"owner","cap":"","check":false,"step_up":"proposed","step_up_code":"990011","step_up_at":5,"paired_at":1731000000000,"paired_via":"app","paired_by":""}
            ]}
            """#.utf8)
        let mac = try #require(PeopleAccess(members: members, isSocket: true))
        let by = Dictionary(uniqueKeysWithValues: mac.people.flatMap(\.devices).map { ($0.id, $0) })
        let said = try #require(by["pad-o"]), silent = try #require(by["old-o"])
        #expect(said.model == "iPhone 17 Pro")
        #expect(PeopleAccess.modelLine(for: said) == "Says it’s: iPhone 17 Pro", "its own labeled claim")
        #expect(PeopleAccess.pairedLine(for: said, runner: "Mac") == PeopleAccess.pairedLine(for: silent, runner: "Mac"),
                "the route line never carries what the device said")
        #expect(silent.model == "" && PeopleAccess.modelLine(for: silent) == nil)
    }

    /// The review's path (ov-982 B1): an agent pairs from a terminal, its key
    /// arrives as a proposal (ov-980), and the Mac lists it under Waiting for
    /// Approval, where one press of Allow trusted and vouched for it. Wherever
    /// its approval is offered, the first press asks again.
    @Test("A device paired from a terminal is asked about twice, in whichever list it waits")
    func askedTwiceWherever() throws {
        let members = Data(
            #"""
            {"members":[],"claude_account":"","you":"",
            "device_access":[
              {"client_id":"cli-o","person":"","access":"owner","cap":"","check":false,"step_up":"proposed","step_up_code":"418207","step_up_at":5,"step_up_vouched":false,"paired_at":1731000000000,"paired_via":"terminal","paired_by":""},
              {"client_id":"pad-o","person":"","access":"owner","cap":"","check":false,"step_up":"proposed","step_up_code":"221133","step_up_at":5,"step_up_vouched":false,"paired_at":1731000000000,"paired_via":"app","paired_by":""},
              {"client_id":"old-o","person":"","access":"owner","cap":"","check":false,"step_up":"trusted","step_up_code":"990011","step_up_at":5,"paired_at":1731000000000,"paired_via":"terminal","paired_by":""}
            ]}
            """#.utf8)
        let mac = try #require(PeopleAccess(members: members, isSocket: true))
        let waiting = try #require(mac.shownWaiting.first { $0.id == "cli-o" }, "the agent's device waits for approval")
        #expect(mac.mayAllow(waiting))
        #expect(PeopleAccess.firstStep(approving: waiting) == .askAgain, "Allow asks again first")
        let vouch = try #require(mac.toVouch.first { $0.id == "old-o" })
        #expect(PeopleAccess.firstStep(approving: vouch) == .askAgain, "and so does Approve on the cutover list")
        let inApp = try #require(mac.shownWaiting.first { $0.id == "pad-o" })
        #expect(PeopleAccess.firstStep(approving: inApp) == .approve, "one paired in Far Cooler is approved at once")
    }
}
