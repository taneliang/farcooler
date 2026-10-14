import Foundation
import Testing

@testable import AgentKit

/// The files the phone's UI tests launch the canned runner with, read as the
/// phone reads them: `{"plan": ..., "records": ...}` from `test/fixtures`
/// (ov-538). A rule that a UI test used to pin on a launched app is pinned
/// here, on the same bytes.
enum PlanPhoneFixtures {
    static var root: URL {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<6 { root.deleteLastPathComponent() }
        return root
    }

    private static func object(_ name: String) throws -> [String: Any] {
        let data = try Data(contentsOf: root.appendingPathComponent("test/fixtures/\(name)"))
        return try #require(try JSONSerialization.jsonObject(with: data) as? [String: Any])
    }

    static func plan(_ name: String) throws -> PlanModel {
        try PlanModel.decode(JSONSerialization.data(withJSONObject: try #require(try object(name)["plan"])))
    }

    static func record(_ name: String, _ id: String) throws -> PlanRecord {
        let records = try #require(try object(name)["records"] as? [String: Any])
        return try PlanRecord.decode(JSONSerialization.data(withJSONObject: try #require(records[id], "no record for \(id)")))
    }

    /// `plan-phones-ov503.json` as the Billing board shows it: the card
    /// `web-site` works is the board's one that needs a decision, and that one
    /// thing needs the owner.
    static let decision = "0198f2c0-0000-7000-8000-00000000e007"

    static func sections(agents: Bool = true) throws -> PlanSections {
        PlanSections(
            plan: try plan("plan-phones-ov503.json"), needsYou: 1, statuses: [decision: .needsDecision],
            agentsSupported: agents)
    }

    /// The row for the lane called `name`, wherever Waiting, In Review or Building holds it.
    static func row(_ sections: PlanSections, _ name: String) -> PlanSections.LaneRow? {
        let all = sections.inReview + sections.building + sections.onLandedTrain + sections.waiting.next
            + sections.waiting.notRanked + sections.waiting.onYou
        return all.first { $0.lane.name == name }
    }

    static let en = Locale(identifier: "en_US")
}
