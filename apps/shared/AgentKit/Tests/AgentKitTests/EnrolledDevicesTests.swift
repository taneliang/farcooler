import Foundation
import Testing

@testable import AgentKit

// Who added each device, and what revoking one offers (ov-855).
//
// Run by `swift test --package-path apps/shared/AgentKit`. Android mirrors
// these in `apps/android/app/src/test/java/com/farcooler/model/EnrolledDevicesTest.kt`.

private let answer = Data(
    """
    {"clients":[
      {"clientId":"mac-1","label":"Studio Mac","scope":"host_admin","foreign":false,"shellAccess":false,"addedBy":""},
      {"clientId":"mac-1","label":"Studio Mac","scope":"host_admin","foreign":false,"shellAccess":true,"addedBy":""},
      {"clientId":"phone-1","label":"Alexandra’s iPhone","scope":"control","foreign":false,"addedBy":"mac-1"},
      {"clientId":"web-1","label":"Alexandra’s Browser","scope":"read","foreign":false,"addedBy":"phone-1"},
      {"clientId":"web-2","label":"","scope":"read","foreign":false,"addedBy":"phone-1"},
      {"clientId":"old-1","label":"Old Tablet","scope":"read","foreign":false,"addedBy":"gone-9"},
      {"clientId":"","label":"by hand","scope":"unspecified","foreign":true,"addedBy":""}
    ]}
    """.utf8)

@Suite struct EnrolledDevicesTests {
    private func list() throws -> EnrolledDevices { try #require(EnrolledDevices(json: answer)) }
    private func device(_ id: String, in list: EnrolledDevices) throws -> EnrolledDevices.Device {
        try #require(list.devices.first { $0.clientId == id })
    }

    @Test("A Mac's two lines are one row, and a hand-written line has none")
    func oneRowPerDevice() throws {
        let list = try list()
        #expect(list.devices.map(\.clientId) == ["mac-1", "phone-1", "web-1", "web-2", "old-1"])
    }

    @Test("A row names the device that added it, and none for one nobody added")
    func addedByLine() throws {
        let list = try list()
        #expect(list.addedByLine(for: try device("phone-1", in: list)) == "Added by Studio Mac")
        #expect(list.addedByLine(for: try device("web-1", in: list)) == "Added by Alexandra’s iPhone")
        #expect(list.addedByLine(for: try device("mac-1", in: list)) == nil)
    }

    @Test("An adder that is no longer enrolled is said so, not left blank")
    func removedAdder() throws {
        let list = try list()
        #expect(list.addedByLine(for: try device("old-1", in: list)) == "Added by a device that was removed")
    }

    @Test("A device with no name is titled by its id")
    func untitled() throws {
        let list = try list()
        #expect(try device("web-2", in: list).title == "web-2")
    }

    @Test("What a device added includes what those added in turn, and never itself")
    func chain() throws {
        let list = try list()
        let reached = list.added(by: try device("mac-1", in: list)).map(\.clientId)
        #expect(Set(reached) == ["phone-1", "web-1", "web-2"])
        #expect(!list.added(by: try device("phone-1", in: list)).contains { $0.clientId == "phone-1" })
    }

    @Test("A loop in who-added-whom ends")
    func loop() {
        let looped = EnrolledDevices(devices: [
            .init(clientId: "a", label: "A", addedBy: "b"), .init(clientId: "b", label: "B", addedBy: "a"),
        ])
        #expect(looped.added(by: looped.devices[0]).map(\.clientId) == ["b"])
    }

    @Test("A device that added others offers both choices, counted")
    func bothChoices() throws {
        let list = try list()
        let choices = list.revokeChoices(for: try device("phone-1", in: list))
        #expect(EnrolledDevices.RevokeChoices.justThisTitle == "Revoke This")
        #expect(choices.withAddedTitle == "Revoke This and the 2 It Added")
        #expect(choices.message.contains("Alexandra’s Browser, web-2"))
    }

    @Test("A device that added none offers only the one choice")
    func oneChoice() throws {
        let list = try list()
        #expect(list.revokeChoices(for: try device("web-1", in: list)).withAddedTitle == nil)
    }

    @Test("Only the second choice sends withAdded")
    func revokeArguments() throws {
        let list = try list()
        let phone = try device("phone-1", in: list)
        #expect(Set(EnrolledDevices.revokeArguments(for: phone, withAdded: false).keys) == ["clientId"])
        let with = EnrolledDevices.revokeArguments(for: phone, withAdded: true)
        #expect(Set(with.keys) == ["clientId", "withAdded"])
        #expect(with["withAdded"] as? Bool == true)
        #expect(with["clientId"] as? String == "phone-1")
    }

    @Test("Anything but a client list is not one")
    func notAList() {
        #expect(EnrolledDevices(json: Data("{}".utf8)) == nil)
        #expect(EnrolledDevices(json: Data("nope".utf8)) == nil)
    }

    @Test("The daily limit has its own sentence, only for add_limit")
    func addLimit() {
        let sentence = RunnerRefusal.addLimitSentence
        #expect(sentence.contains("five"))
        let limited = RunnerRefusal.trouble(
            forWord: "resource-conflict", what: "add_limit", message: "raw", otherwise: "generic")
        #expect(limited.sentence == sentence)
        #expect(limited.transcript == nil)
        let taken = RunnerRefusal.trouble(
            forWord: "resource-conflict", what: "device_taken", message: "raw", otherwise: "generic")
        #expect(taken.sentence == RunnerRefusal.resourceConflict.sentence)
        let other = RunnerRefusal.trouble(forWord: "not-found", what: "add_limit", message: "raw", otherwise: "g")
        #expect(other.sentence == RunnerRefusal.notFound.sentence)
    }

    private struct Refused: Error {}

    @Test("A revoke that throws says why and keeps no list")
    func revokeFailureIsShown() async {
        let result = await EnrolledDevices.revoke(
            { throw Refused() }, sentence: { _ in "That device is already gone." })
        #expect(result.failure == "That device is already gone.")
        #expect(result.list == nil)
    }

    @Test("A revoke that works hands back the list and no failure")
    func revokeWorks() async throws {
        let result = await EnrolledDevices.revoke({ answer }, sentence: { _ in "no" })
        #expect(result.failure == nil)
        #expect(result.list != nil)
    }

    @Test("A list read that throws says so, and an unreadable answer says it too")
    func listFailureIsShown() async {
        let thrown = await EnrolledDevices.read({ throw Refused() }, sentence: { _ in "Offline." })
        #expect(thrown.failure == "Offline.")
        let garbled = await EnrolledDevices.read({ Data("nope".utf8) }, sentence: { _ in "x" })
        #expect(garbled.failure == EnrolledDevices.listNotRead)
    }
}
