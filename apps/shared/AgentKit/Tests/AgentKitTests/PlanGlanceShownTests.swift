import Foundation
import Testing

@testable import AgentKit

/// What a plan surface shows when the relay answers, doesn't, or names a
/// quiet runner's plan (ov-310 review H2). "No board has a plan yet" only
/// when the relay said so.
struct PlanGlanceShownTests {
    let now = Date(timeIntervalSince1970: 1_791_019_800)
    let board = PlanGlanceTests.main

    @Test("A plan the relay just gave is current, and is remembered as of when its runner was heard")
    func aCurrentPlan() {
        var said = board
        said.heardAgo = 90_000
        let (shown, kept) = PlanGlanceMemory.shown(reading: .answered([], plan: said), remembered: nil, at: now)
        #expect(shown == .plan(said, caveat: nil))
        #expect(kept == PlanGlanceSeen(glance: said, asOf: now.addingTimeInterval(-90)))
    }

    @Test("A quiet runner's last plan says it can't reach the runner, and how long ago")
    func aQuietRunnersPlan() {
        var said = board
        said.runner = "Studio"
        said.heardAgo = 3 * 3_600_000
        said.quiet = true
        let (shown, _) = PlanGlanceMemory.shown(reading: .answered([], plan: said), remembered: nil, at: now)
        #expect(shown == .plan(said, caveat: PlanCaveat(age: 3 * 3600, cantReach: "Studio")))
        #expect(PlanCaveat(age: 3 * 3600, cantReach: "Studio").line == "Can’t reach Studio · 3h ago")
        #expect(said.cardCaveat == PlanCaveat(age: 3 * 3600, cantReach: "Studio"))
    }

    @Test("No answer draws the last plan with its age, never \"no plan\"")
    func noAnswer() {
        let seen = PlanGlanceSeen(glance: board, asOf: now.addingTimeInterval(-3 * 3600))
        let (shown, kept) = PlanGlanceMemory.shown(reading: .failed, remembered: seen, at: now)
        #expect(shown == .plan(board, caveat: PlanCaveat(age: 3 * 3600)))
        #expect(PlanCaveat(age: 3 * 3600).line == "As of 3h ago")
        #expect(kept == seen, "kept until a newer one or none")
        // Nothing remembered: it can't say, so it says that.
        let (unknown, _) = PlanGlanceMemory.shown(reading: .failed, remembered: nil, at: now)
        #expect(unknown == .unknown)
        #expect(unknown.message == "Can’t check your plan right now.")
    }

    @Test("Only an answer naming no board says no board has a plan, and forgets the last one")
    func noPlan() {
        let seen = PlanGlanceSeen(glance: board, asOf: now)
        let (shown, kept) = PlanGlanceMemory.shown(reading: .answered([]), remembered: seen, at: now)
        #expect(shown == .noPlan)
        #expect(shown.message == "No board has a plan yet.")
        #expect(kept == nil)
        for reading in [RunnerPulse.Reading.noCredential, .refused] {
            let (signedOut, forgot) = PlanGlanceMemory.shown(reading: reading, remembered: seen, at: now)
            #expect(signedOut == .signedOut)
            #expect(forgot == nil, "a signed-out phone keeps no lane names")
        }
    }

    @Test("The memory round-trips through the container's file, and nil removes it")
    func memoryFile() throws {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: dir) }
        var said = board
        said.runner = "Studio"
        said.quiet = true
        let seen = PlanGlanceSeen(glance: said, asOf: now)
        PlanGlanceMemory.write(seen, to: dir)
        #expect(PlanGlanceMemory.read(from: dir) == seen)
        PlanGlanceMemory.write(nil, to: dir)
        #expect(PlanGlanceMemory.read(from: dir) == nil)
    }
}
