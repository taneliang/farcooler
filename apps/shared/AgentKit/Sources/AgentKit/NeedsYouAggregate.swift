import Foundation

// Aggregation is Rust's needs_you_view, carried by the existing view-model bridge.
// Keep the native payloads: their dates, derived marker and action targets stay native.
extension NeedsYou {
    static func aggregate(_ runners: [String: [NeedsYouItem]]) -> WireNeedsYouAggregateView {
        let input: [[String: Any]] = runners.map { runner, items in
            ["runner_id": runner, "items": items.map { item -> [String: Any] in
                var value: [String: Any] = ["id": item.itemID, "kind": item.kind == .unknown ? item.rawKind : item.kind.rawValue, "rank": item.rank]
                value["workspace_id"] = item.workspaceID
                value["repository_id"] = item.repositoryID
                if let concern = item.concern { value["concern"] = ["yours": concern.yours] }
                return value
            }]
        }
        struct Answer: Decodable { var needs_you: WireNeedsYouAggregateView }
        let request = try! JSONSerialization.data(withJSONObject: ["schema": 1, "needs_you": input])
        guard let answer = PlanViewPresenter.ask(String(decoding: request, as: UTF8.self)),
            let decoded = try? JSONDecoder().decode(Answer.self, from: Data(answer.utf8))
        else { preconditionFailure("needs_you_bridge") }
        return decoded.needs_you
    }

    static func aggregate(_ items: [NeedsYouItem]) -> WireNeedsYouAggregateView {
        aggregate(Dictionary(grouping: items, by: \.runner))
    }
}
