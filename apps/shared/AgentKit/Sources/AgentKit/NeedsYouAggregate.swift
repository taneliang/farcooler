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
        NSLog("Needs You presenter rejected the list; retrying without duplicate item IDs") // platform copy: developer diagnostic, not UI copy
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

extension NeedsYouItem {
    /// Whether it adds to a count or breaks the all-clear. An FYI (a member
    /// kept a ruling, ov-756) is read and never acted on, so it is a row in
    /// the list and nothing more.
    public var isCounted: Bool { NeedsYou.aggregate([runner: [self]]).rows.first?.counted ?? false }
}

extension NeedsYou {
    /// Every runner's items in one list: by rank, then by runner.
    ///
    /// Rank, not `since`, because a rank is a duration measured on its own
    /// runner and a `since` is a reading of that runner's clock: a build box
    /// whose clock runs two minutes slow would otherwise put its newest ask
    /// ahead of an older one on the studio. The runner name breaks ties so the
    /// order doesn't shuffle between reads. A kind this build doesn't know
    /// sorts after every kind it does, whatever its rank.
    ///
    /// Each item comes back with its `runner` set to its key here.
    ///
    /// A pull request that wants a review is the GitHub login's, not a
    /// runner's: two runners signed in as one login list the same one under
    /// the same id (`pr-review:<hash of its URL>`), and it is one thing to do.
    /// So it appears once, from the runner that has waited longest (the lowest
    /// rank), and the other runner's copy is dropped (ov-315).
    public static func merge(_ runners: [String: [NeedsYouItem]]) -> [NeedsYouItem] {
        aggregate(runners).rows.compactMap { row in
            guard var item = runners[row.key.runner_id]?.first(where: { $0.itemID == row.key.item_id }) else { return nil }
            item.runner = row.key.runner_id
            return item
        }
    }

    /// `merged` split into the reader's own (and nobody's) and everyone
    /// else's, in the order it came.
    public static func grouped(_ merged: [NeedsYouItem]) -> Groups {
        let view = aggregate(merged)
        let yours = view.yours.compactMap { key in merged.first { $0.runner == key.runner_id && $0.itemID == key.item_id } }
        let others = view.others.compactMap { key in merged.first { $0.runner == key.runner_id && $0.itemID == key.item_id } }
        return Groups(yours: yours, others: others, isSplit: view.split)
    }
}

extension Collection where Element == NeedsYouItem {
    /// How many items are counted under `workspaceID`.
    ///
    /// Items, not signals: an ask with a decision in its `also` is one thing
    /// to act on and counts once. The badge on a workspace's row is this
    /// number, and the Needs You total is `count`.
    ///
    /// A workspace id is a runner's uuid, so this needs no runner to tell two
    /// workspaces apart.
    public func count(in workspaceID: String) -> Int {
        Int(NeedsYou.aggregate(Array(self)).workspace_counts.filter { $0.id == workspaceID }.reduce(0) { $0 + $1.count })
    }

    /// How many items with no workspace belong to `repositoryID`: its
    /// Unclaimed group's count.
    public func unclaimedCount(inRepository repositoryID: String) -> Int {
        Int(NeedsYou.aggregate(Array(self)).unclaimed_counts.filter { $0.id == repositoryID }.reduce(0) { $0 + $1.count })
    }

    /// How many items count toward a total, a badge or the all-clear: every
    /// one but an FYI, which is read and never acted on (ov-756).
    public var countedTotal: Int { Int(NeedsYou.aggregate(Array(self)).counted_total) }
}
