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
        func present(_ input: [[String: Any]]) -> WireNeedsYouAggregateView? {
            guard let request = try? JSONSerialization.data(withJSONObject: ["schema": 1, "needs_you": input]),
                let answer = PlanViewPresenter.ask(String(decoding: request, as: UTF8.self)),
                let decoded = try? JSONDecoder().decode(Answer.self, from: Data(answer.utf8))
            else { return nil }
            return decoded.needs_you
        }
        if let view = present(input) { return view }
        NSLog("Needs You presenter rejected the list; retrying without duplicate item IDs")
        let unique = runners.mapValues { items in
            var seen = Set<String>()
            return items.filter { seen.insert($0.itemID).inserted }
        }
        if unique.values.reduce(0, { $0 + $1.count }) < runners.values.reduce(0, { $0 + $1.count }) {
            return aggregate(unique)
        }
        return WireNeedsYouAggregateView(schema: 1, rows: [], yours: [], others: [], split: false,
            counted_total: 0, runner_counts: [], workspace_counts: [], unclaimed_counts: [])
    }

    static func aggregate(_ items: [NeedsYouItem]) -> WireNeedsYouAggregateView {
        aggregate(Dictionary(grouping: items, by: \.runner))
    }
}
