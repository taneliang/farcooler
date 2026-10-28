import Foundation
import Testing

@testable import AgentKit

/// Add Teammate from a phone (ov-854): what the phone sends, what it reads
/// back, and what it says about each runner. The Mac shares these types.
struct TeammateAddTests {
    private func choice(
        scope: TeammateScope = .control, handle: String? = nil, name: String? = nil
    ) -> TeammateChoice {
        var choice = TeammateChoice(
            who: CeremonyTeammate(account: "user_bob", email: "bob.smith@example.com", team: "Platform"),
            directory: "{}", scanner: "user_alice", grantedBy: "alice@example.com")
        choice.scope = scope
        if let handle { choice.handle = handle }
        if let name { choice.name = name }
        return choice
    }

    // MARK: What the phone sends

    @Test("The phone sends one teammate.enroll with the chosen scope, never a shell, and the directory's account and email")
    func theCallCarriesTheChoice() throws {
        #expect(TeammateAddRequest.method == "teammate.enroll", "a plain client.enroll would leave the key the owner's")
        var teammate = choice(scope: .read)
        teammate.name = "  Bob Smith  "
        let arguments = TeammateAddRequest.arguments(
            publicKey: "ssh-ed25519 AAAA bob", label: "Bob’s iPhone", clientId: "phone-bob", nodeKey: "NODE",
            teammate: teammate)
        #expect(arguments["scope"] as? String == "read")
        #expect(arguments["handle"] as? String == "bob-smith")
        #expect(arguments["name"] as? String == "Bob Smith")
        #expect(arguments["account"] as? String == "user_bob")
        #expect(arguments["email"] as? String == "bob.smith@example.com")
        #expect(arguments["clientId"] as? String == "phone-bob")
        #expect(arguments["nodeKey"] as? String == "NODE")
        #expect(arguments["shellAccess"] == nil, "a teammate never gets a shell line")
    }

    @Test("A teammate's access is Control or Read, and nothing wider")
    func accessIsControlOrRead() {
        #expect(TeammateScope.allCases.map(\.rawValue) == ["control", "read"])
        #expect(TeammateScope.allCases.map(\.title) == ["Control", "Read"])
        #expect(choice().scope == .control, "Control is the default")
    }

    // MARK: Who they are

    @Test("A suggested handle and name come from the directory's email, and an invalid handle isn't complete")
    func suggestionsAndCompleteness() {
        let suggested = choice()
        #expect(suggested.handle == "bob-smith")
        #expect(suggested.name == "Bob Smith")
        #expect(suggested.isComplete)
        #expect(!choice(handle: "Bob").isComplete)
        #expect(!choice(handle: "owner").isComplete, "a handle the runner keeps for itself")
        #expect(!choice(name: "   ").isComplete)
    }

    @Test("The core's accepted answer decodes to the offer, the directory's person and no hold")
    func acceptedAnswerDecodes() throws {
        let json = #"{"offer":"{\"v\":2}","teammate":{"account":"user_bob","email":"bob@example.com","team":"Platform"}}"#
        let accepted = try JSONDecoder().decode(TeammateAccepted.self, from: Data(json.utf8))
        #expect(accepted.offer == #"{"v":2}"#)
        #expect(accepted.teammate == CeremonyTeammate(account: "user_bob", email: "bob@example.com", team: "Platform"))
        #expect(accepted.held == nil)
    }

    // MARK: What the phone reads back

    @Test("Each outcome word the client core answers decodes, and a stillThere survives")
    func outcomesDecode() {
        func decode(_ json: String) -> TeammateAddOutcome { TeammateAddOutcome.decode(Data(json.utf8)) }
        #expect(decode(#"{"outcome":"written","tunnel":"tok"}"#) == .written(tunnel: "tok"))
        #expect(decode(#"{"outcome":"written"}"#) == .written(tunnel: ""))
        #expect(decode(#"{"outcome":"needs_admin"}"#) == .needsAdmin)
        #expect(decode(#"{"outcome":"too_old"}"#) == .tooOld)
        #expect(decode(#"{"outcome":"could_not_read"}"#) == .couldNotRead)
        #expect(decode(#"{"outcome":"someone_elses"}"#) == .someoneElses)
        #expect(decode(#"{"outcome":"not_added","stillThere":true}"#) == .notAdded(stillThere: true))
        #expect(decode(#"{"outcome":"not_added","stillThere":false}"#) == .notAdded(stillThere: false))
        #expect(decode(#"{"outcome":"not_recorded","stillThere":true}"#) == .notRecorded(stillThere: true))
        #expect(decode(#"{"outcome":"not_updated"}"#) == .notUpdated)
    }

    @Test("An answer that doesn't read, or names a word this build lacks, is never a success")
    func unreadableIsNotReached() {
        for json in ["", "nope", "{}", #"{"outcome":"written_somehow"}"#, #"{"outcome":3}"#] {
            let outcome = TeammateAddOutcome.decode(Data(json.utf8))
            #expect(outcome == .notReached, "\(json)")
            #expect(!outcome.isWritten)
        }
        #expect(TeammateAddOutcome.written(tunnel: "").isWritten)
    }

    // MARK: What the phone says

    @Test("A bound runner is not mentioned; each other one is, by name, with what to do")
    func notesNameTheRunnersThatFailed() throws {
        #expect(
            TeammateAddCopy.note(about: [("Cosmo", .written(tunnel: "")), ("Atlas", .written(tunnel: "t"))], device: "iPhone")
                == nil)
        let note = try #require(
            TeammateAddCopy.note(
                about: [("Cosmo", .written(tunnel: "")), ("Atlas", .needsAdmin), ("Borealis", .tooOld)], device: "iPhone"))
        #expect(!note.contains("Cosmo"))
        #expect(note.contains("This iPhone can only control Atlas, so it can’t add a teammate there. Add your teammate from your Mac."))
        #expect(note.contains("Borealis’s Far Cooler is too old to record whose device this is"))
    }

    @Test("A key that couldn't be taken off again is said to be still there; one that was says it's gone")
    func stillThereIsSaid() {
        let gone = TeammateAddCopy.sentence(.notRecorded(stillThere: false), runner: "Cosmo", device: "iPhone") ?? ""
        let there = TeammateAddCopy.sentence(.notRecorded(stillThere: true), runner: "Cosmo", device: "iPhone") ?? ""
        #expect(gone.contains("took the device’s key off again, so it has no access there"))
        #expect(there.contains("couldn’t make sure the device’s key isn’t on that runner"))
        #expect(!there.contains("so it has no access there"))
        #expect(gone.hasPrefix("Far Cooler couldn’t record this device as your teammate’s on Cosmo."))
    }

    @Test("Every sentence is a sentence: no Rust word, no raw error, and none says nothing")
    func everySentenceReadsAsCopy() throws {
        let outcomes: [TeammateAddOutcome] = [
            .needsAdmin, .tooOld, .couldNotRead, .someoneElses, .notAdded(stillThere: true),
            .notAdded(stillThere: false), .notRecorded(stillThere: true), .notRecorded(stillThere: false), .notUpdated,
            .notReached,
        ]
        for outcome in outcomes {
            let sentence = try #require(TeammateAddCopy.sentence(outcome, runner: "Cosmo", device: "iPhone"), "\(outcome)")
            #expect(sentence.contains("Cosmo"), "\(outcome)")
            #expect(sentence.hasSuffix("."), "\(outcome)")
            for word in ["needs_admin", "too_old", "Err(", "error:", "host_admin", "member.set"] {
                #expect(!sentence.contains(word), "\(outcome) leaks \(word)")
            }
        }
    }

    @Test("The refusals a teammate's code can meet name what to do next")
    func refusalsSayWhatToDo() {
        #expect(TeammateRefusalCopy.notATeammate(email: "alice@example.com").contains("sign in to alice@example.com"))
        #expect(TeammateRefusalCopy.heldKeyOnAPhone.contains("Add it from your Mac"))
        #expect(TeammateRefusalCopy.directoryUnreachable.contains("Check your connection"))
        #expect(TeammateRefusalCopy.unregisteredKey.contains("sign in to Far Cooler on it"))
    }
}
