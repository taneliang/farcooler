import Foundation
import Testing

@testable import AgentKit

/// Whom a device talks to in a workspace on a shared runner (ov-526): the
/// orchestrator for the owner and the lead, a member's own rep for everyone
/// else, and never a composer the runner would refuse `not_lead`.
struct RunnerPeopleTalkTests {
    static func board(lead: String?, id: String = "w-billing") -> WorkspaceSummary {
        WorkspaceSummary(id: id, name: "Billing", taskPrefix: "bil", isMain: false, ordinal: 1, lead: lead)
    }

    static func people(you: String) -> RunnerPeople {
        RunnerPeople(
            members: [
                .init(handle: "alice", name: "Alice Chen", reps: [.init(terminal: "t1", workspace: "w-billing")]),
                .init(handle: "bob", name: "Bob"),
            ], claudeAccount: "team@example.com", you: you)
    }

    @Test("The owner's device talks to the orchestrator, whoever leads")
    func ownerTalksToTheOrchestrator() {
        let owner = Self.people(you: "")
        #expect(owner.talk(in: Self.board(lead: "")) == .orchestrator)
        #expect(owner.talk(in: Self.board(lead: "alice")) == .orchestrator)
        #expect(owner.talk(in: Self.board(lead: nil)) == .orchestrator)
    }

    @Test("A member who leads the workspace keeps the orchestrator; any other member gets their own rep")
    func memberTalksToTheirRep() {
        let alice = Self.people(you: "alice")
        #expect(alice.talk(in: Self.board(lead: "alice")) == .orchestrator)
        let running = RunnerPeople.Rep(terminal: "t1", workspace: "w-billing")
        #expect(alice.talk(in: Self.board(lead: "")) == .rep(handle: "alice", name: "Alice Chen", existing: running))
        // The same person in a workspace where they have no rep yet.
        #expect(
            alice.talk(in: Self.board(lead: "bob", id: "w-main"))
                == .rep(handle: "alice", name: "Alice Chen", existing: nil))
        #expect(Self.people(you: "bob").talk(in: Self.board(lead: "alice"))
            == .rep(handle: "bob", name: "Bob", existing: nil))
    }

    @Test("A runner that doesn't name the lead never gives a member an orchestrator composer")
    func unknownLeadIsTheSafeWay() {
        #expect(Self.people(you: "alice").talk(in: Self.board(lead: nil))
            == .rep(handle: "alice", name: "Alice Chen", existing: .init(terminal: "t1", workspace: "w-billing")))
    }

    @Test("A released device is let into neither")
    func releasedTalksToNobody() {
        #expect(Self.people(you: "(released)").talk(in: Self.board(lead: "")) == .neither)
    }

    @Test("The rep's first screen says what a rep is for, then whose account the work runs on")
    func firstScreen() {
        let intro = Self.people(you: "bob").repIntro
        #expect(intro == [RunnerPeople.repPurpose, "Work here runs on team@example.com’s Claude account."])
        // Below Control the runner doesn't say whose, and the line still doesn't go blank.
        let looking = RunnerPeople(members: [], claudeAccount: "", you: "bob")
        #expect(looking.repIntro.last == "Work here runs on the Claude account signed in on this runner.")
    }

    @Test("A workspace decodes its lead, and keeps nil for a runner that doesn't say")
    func decodesLead() throws {
        func summary(_ extra: String) throws -> WorkspaceSummary {
            try JSONDecoder().decode(WorkspaceSummary.self, from: Data(#"{"id":"w","name":"Billing"\#(extra)}"#.utf8))
        }
        #expect(try summary(#","lead":"alice""#).lead == "alice")
        #expect(try summary(#","lead":"""#).lead == "")
        #expect(try summary("").lead == nil)
    }

    @Test("The seat waits for the people, says so when they can't be read, and is the orchestrator's where there are none")
    func seatBeforeAndBesideTheAnswer() {
        let board = Self.board(lead: "alexandra")
        func seat(_ people: RunnerPeople?, failed: Bool = false, hasPeople: Bool = true, in ws: WorkspaceSummary? = nil) -> TalkSeat {
            TalkSeat.resolve(workspace: ws ?? board, people: people, readFailed: failed, hasPeople: hasPeople)
        }
        // Nothing to type into until the runner has said who this is.
        #expect(seat(nil) == .reading)
        #expect(seat(nil, failed: true) == .unknown)
        // A runner without people refuses nobody: the orchestrator, as before.
        #expect(seat(nil, hasPeople: false) == .orchestrator)
        #expect(seat(Self.people(you: "bob"), hasPeople: false) == .orchestrator)
        // A runner's implicit workspace has no orchestrator to refuse.
        #expect(seat(Self.people(you: "bob"), in: .implicit(repository: "r")) == .orchestrator)
        #expect(seat(Self.people(you: "bob")) == .rep(handle: "bob", name: "Bob", existing: nil))
        #expect(seat(Self.people(you: "")) == .orchestrator)
        #expect(seat(Self.people(you: "(released)")) == .neither)
    }

    @Test("A member's place is called Your Rep; everyone else's is the Orchestrator")
    func titles() {
        #expect(TalkSeat.rep(handle: "bob", name: "Bob", existing: nil).title == "Your Rep")
        for seat in [TalkSeat.orchestrator, .reading, .unknown, .neither] { #expect(seat.title == "Orchestrator") }
    }

    @Test("The owner never waits on the people: a local owner, or a device last told it's the owner's, keeps the orchestrator while reading and when the read fails")
    func ownerDoesNotWait() {
        let board = Self.board(lead: "alexandra")
        func seat(_ failed: Bool, you: String?, local: Bool = false) -> TalkSeat {
            TalkSeat.resolve(workspace: board, people: nil, readFailed: failed, hasPeople: true, knownYou: you, isLocalOwner: local)
        }
        #expect(seat(false, you: nil, local: true) == .orchestrator)
        #expect(seat(true, you: nil, local: true) == .orchestrator)
        #expect(seat(false, you: "") == .orchestrator)
        #expect(seat(true, you: "") == .orchestrator)
        // A device last told it was a member's, or never told, stays shut out.
        #expect(seat(false, you: "bob") == .reading)
        #expect(seat(true, you: "bob") == .unknown)
        #expect(seat(false, you: nil) == .reading)
    }

    @Test("Every orchestrator surface is the orchestrator's seat alone, and the place is named for the seat")
    func surfacesBySeat() {
        let all: [TalkSeat] = [
            .orchestrator, .reading, .unknown, .neither,
            .rep(handle: "bob", name: "Bob", existing: nil),
        ]
        for seat in all {
            let on = seat == .orchestrator
            let s = seat.surfaces
            #expect(s.columnIsOrchestrator == on, "\(seat) column")
            #expect(s.orchestratorMenu == on, "\(seat) menu")
            #expect(s.asksOrchestrator == on, "\(seat) ask")
            #expect(s.discussesRulings == on, "\(seat) rulings")
            #expect(s.adoptsOrchestrator == on, "\(seat) use as orchestrator")
        }
        // Reading and unknown are never called Your Rep: an owner's row doesn't flash it.
        #expect(TalkSeat.reading.surfaces.title == "Orchestrator")
        #expect(TalkSeat.unknown.surfaces.title == "Orchestrator")
        #expect(TalkSeat.rep(handle: "bob", name: "Bob", existing: nil).surfaces.title == "Your Rep")
    }
}

struct RepBoardCopyTests {
    @Test("A member's blank board says to ask the rep, and never to tell an orchestrator they can't reach")
    func blankBoard() {
        let copy = BoardForm.blankCopy(ledByOrchestrator: true, orchestratorRunning: true, asksRep: true)
        #expect(copy.rows.map(\.text).contains("Ask your rep what you want done"))
        #expect(!copy.rows.map(\.text).contains("Tell it what you want built"))
        #expect(BoardForm.blankCopy(ledByOrchestrator: true, orchestratorRunning: true) != copy)
    }
}
