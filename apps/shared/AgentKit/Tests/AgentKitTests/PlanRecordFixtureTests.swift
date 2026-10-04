import Foundation
import Testing

@testable import AgentKit

/// A theme's and a lane's record as the CLI prints it (ov-273, review 1004i
/// P4): `plan theme show --json` and `plan lane show --json`, which
/// `plan_show_json_is_the_record_the_mac_reads` in `crates/cli/src/plan_tests.rs`
/// writes byte for byte. The Mac's timeline and What Changed come from these;
/// a renamed or reshaped key fails here, not silently on screen.
struct PlanRecordFixtureTests {
    static func fixture(_ name: String) throws -> PlanRecord {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<6 { root.deleteLastPathComponent() }
        return try PlanRecord.decode(Data(contentsOf: root.appendingPathComponent("test/fixtures/\(name)")))
    }

    @Test("A theme's record from the CLI gives its timeline and What Changed")
    func theme() throws {
        let record = try Self.fixture("plan-theme-show.json")
        #expect(record.events.count == 6)
        #expect(record.events.map(\.kind) == ["state", "story", "cards", "cards", "story", "state"])
        #expect(record.events[1].extra?.to == "Tokens are in review.")
        #expect(record.events[5].extra?.from == "active")
        #expect(record.timeline.map(\.text) == [
            "Created.", "Rewrote where it stands.", "Added ov-1 and ov-2.", "Rewrote where it stands.", "Paused.",
        ])
        #expect(record.previousStory?.story == "Tokens are in review.")
        #expect(record.previousStory?.at == Int64(1_799_996_400_000), "the second rewrite, an hour before now")
    }

    @Test("A lane's record from the CLI gives its timeline")
    func lane() throws {
        let record = try Self.fixture("plan-lane-show.json")
        #expect(record.events.map(\.kind) == ["state", "plan", "state", "state"])
        #expect(record.events[2].extra?.from == "queued" && record.events[2].extra?.to == "building")
        #expect(record.timeline.map(\.text) == ["Queued.", "Ranked 1.", "Started building.", "Moved to review."])
        #expect(record.previousStory == nil)
    }
}
