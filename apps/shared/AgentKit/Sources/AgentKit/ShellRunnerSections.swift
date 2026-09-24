import Foundation

// The overview, one section per runner, and a drag inside one of them.
//
// The overview used to be one merged section for every runner the phone was
// connected to, sorted by what needed you, with a heading only when a SECOND,
// cached runner was drawn under it. Two asks retired that: the grid is
// sectioned by runner, and a runner's worktrees can be dragged into an order
// that runner keeps — the same order the Mac's sidebar draws, because it is the
// same table (`crates/store/src/migrate.rs`, migration 0009).
//
// Those two asks are one change and not two. An order you can drag has to be
// the order that is DRAWN, so precedence no longer sorts anything here: a card
// put third stays third when its agent starts asking for you. And a drag has to
// go to exactly one runner, carrying exactly that runner's worktrees, because
// each runner keeps its own table and a phone holds several — so the grid has
// to know which runner each card is on, which is what a section is.
//
// Everything a drop decides lives in this file rather than in the grid, for
// `ShellNavigation.swift`'s reason: the iOS target has no unit tests, and the
// three ways a drop goes wrong — the wrong runner, a shorter list than the
// screen shows, a card that springs back for a round trip — all look fine in a
// screenshot. See `ShellRunnerSectionsTests`.

/// A runner as the overview names it.
struct ShellRunnerLabel: Identifiable, Hashable, Sendable {
    /// The runner's id, which is what `ShellWorkspace.runner` carries. Not the
    /// label: two runners on one box can share that.
    var id: String
    /// What the section's header says.
    var name: String
    /// Whether the runner can take a write right now.
    ///
    /// A runner that is reconnecting still contributes its last good rows to
    /// the merge — that is what keeps the grid from blanking while a laptop
    /// sleeps — but a drop on them would be a request with nowhere to go.
    var isAnswering: Bool = true
}

/// One card in a section: which worktree, by the id that survives a poll, and
/// where it sits in the fleet right now.
///
/// **The id is the identity and the index is only a lookup.** The grid used to
/// be a `ForEach` over indices, and an index names a different worktree the
/// moment a poll inserts one above it — which a drop in flight would then have
/// moved. The index stays because the card's frame is published under it and
/// the flight reads it back that way; see `ShellTileFrame`.
struct ShellCard: Identifiable, Hashable, Sendable {
    var id: String
    var index: Int
}

/// One runner's worktrees, in the order that runner keeps them.
struct ShellRunnerSection: Identifiable, Hashable {
    var runner: ShellRunnerLabel
    var cards: [ShellCard]
    /// Whether a card in this section can be dragged to a new place in it.
    ///
    /// Only when all of these hold, and each one is a way a drag could appear
    /// to work and not have:
    ///
    /// - **Nothing is being searched for.** A search shows a subset, and a
    ///   drop among a subset is a drop whose meaning depends on cards nobody
    ///   can see.
    /// - **The runner is answering.** Otherwise the write has nowhere to go.
    /// - **The runner keeps an order.** A runner too old to store one would
    ///   accept nothing and put the card back on the next poll with no error.
    /// - **There are two cards.** One card has nowhere else to be.
    var canReorder: Bool

    var id: String { runner.id }

    /// What dropping `sources` just before `target` — or at the end, for a nil
    /// target — asks this section's runner for, or nil when it asks nothing.
    ///
    /// Nil for a drop that changes nothing, because a reorder makes every other
    /// connected client re-read the fleet. Nil for a section that cannot be
    /// reordered at all. And nil for a card that is not in this section: a
    /// worktree carried into another runner's section is not a move either
    /// runner can make, because a worktree lives on the machine its directory
    /// is on.
    ///
    /// Several sources keep the order they already had between them, which is
    /// what a drag of several cards is expected to do and what the platform's
    /// own lists do.
    func reorder(moving sources: [String], before target: String?) -> ShellReorderRequest? {
        guard canReorder, !sources.isEmpty else { return nil }
        let ids = cards.map(\.id)
        let moving = Set(sources)
        guard moving.isSubset(of: Set(ids)) else { return nil }
        if let target {
            // Dropped onto one of the cards being dragged: nowhere new.
            guard ids.contains(target), !moving.contains(target) else { return nil }
        }

        let carried = ids.filter { moving.contains($0) }
        var rest = ids.filter { !moving.contains($0) }
        let landing = target.flatMap { rest.firstIndex(of: $0) } ?? rest.endIndex
        rest.insert(contentsOf: carried, at: landing)
        guard rest != ids else { return nil }
        return ShellReorderRequest(runner: runner.id, order: rest)
    }
}

/// A drop, as the runner it is for will be told it.
///
/// The WHOLE section, first on screen first, and not "move this one to N". The
/// runner permutes exactly the rows it is named among the ranks those rows
/// already hold and leaves everything else where it was
/// (`Store::reorder_workspaces`), so a section that leaves hidden worktrees out
/// reorders cleanly around them — but only within one runner's table, which is
/// why this carries a runner and nothing from any other.
struct ShellReorderRequest: Hashable, Sendable {
    /// The runner, by id.
    var runner: String
    /// The shell's own workspace ids, in the order asked for.
    var order: [String]

    /// The daemon's own workspace ids, resolved from the shell's, or nil.
    ///
    /// **All or nothing.** A worktree removed between the lift and the drop
    /// resolves to nothing, and dropping it from the list would send a SHORTER
    /// list than the screen showed — the runner would faithfully reorder the
    /// rest around a card the phone still believes it moved. One that resolves
    /// to a different runner is the wrong table entirely. Either way the drop
    /// is not sent, and the card goes back to where the runner has it.
    func workspaceIDs(
        resolving resolve: (String) -> (runner: String, workspace: String)?
    ) -> [String]? {
        var resolved: [String] = []
        for id in order {
            guard let found = resolve(id), found.runner == runner else { return nil }
            resolved.append(found.workspace)
        }
        return resolved
    }
}

/// Drops sent and not yet answered, drawn in the meantime.
///
/// Without this a card springs back to where it was the moment it is let go
/// and jumps forward a round trip later, which reads as a drop that failed and
/// then changed its mind.
///
/// It is NOT a guess about the outcome, and that is what `settle` is for. An
/// order stands only until the call that sent it returns — and by then the
/// connection has already re-read the fleet — so what is drawn afterwards is
/// always the runner's own answer. A refusal puts the card back.
struct ShellPendingOrders: Hashable {
    private var orders: [String: [String]] = [:]

    init() {}

    /// A drop has been sent: draw it.
    mutating func begin(_ request: ShellReorderRequest) {
        orders[request.runner] = request.order
    }

    /// That drop's call has returned: stop drawing it — unless a newer drop on
    /// the same runner has replaced it, whose answer is still to come.
    mutating func settle(_ request: ShellReorderRequest) {
        guard orders[request.runner] == request.order else { return }
        orders[request.runner] = nil
    }

    /// The order still waiting on this runner's answer, if there is one.
    func order(for runner: String) -> [String]? {
        orders[runner]
    }
}

/// `ids` with the ones `order` names dealt back into the places they hold, in
/// `order`'s order, and every id it does not name left exactly where it was.
///
/// The runner's own rule, stated once for the two callers that have to draw
/// what a runner would do before it has done it. An id `order` names that is
/// no longer in `ids` is skipped; one `ids` has that `order` does not name —
/// a worktree created while a drop was in flight — keeps its place.
private func permuting(_ ids: [String], into order: [String]) -> [String] {
    let present = Set(ids)
    let dealt = order.filter { present.contains($0) }
    let named = Set(dealt)
    let slots = ids.indices.filter { named.contains(ids[$0]) }
    var result = ids
    for (slot, id) in zip(slots, dealt) { result[slot] = id }
    return result
}

extension ShellFleet {
    /// One section per runner, in the order `runners` gives them.
    ///
    /// **Every runner has a section while nothing is being searched for**,
    /// even one with no worktrees on it: the header is where that runner's
    /// own actions live, and a runner with nothing on it is exactly the one
    /// somebody opens to correct. A search is different — a header over no
    /// cards reads as a runner that has gone empty, when what happened is that
    /// nothing on it matched — so a search drops a section it emptied.
    ///
    /// Cards are the runner's shown worktrees in FLEET order, which is the
    /// runner's own order: the merge appends each runner's list as it arrived,
    /// and a runner lists its worktrees by the rank it keeps. Hidden ones are
    /// the Hidden section's — see `hiddenOrder`.
    func runnerSections(
        _ runners: [ShellRunnerLabel], matching query: String = "",
        pending: ShellPendingOrders = ShellPendingOrders()
    ) -> [ShellRunnerSection] {
        let searching = !query.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        return runners.compactMap { runner in
            let mine = workspaces.indices.filter {
                workspaces[$0].runner == runner.id && !workspaces[$0].isHidden
            }
            var shown = ShellFleet.matching(query, in: mine, of: workspaces)
            if searching && shown.isEmpty { return nil }
            if let order = pending.order(for: runner.id) {
                let byID = Dictionary(
                    shown.map { (workspaces[$0].id, $0) }, uniquingKeysWith: { first, _ in first })
                shown = permuting(shown.map { workspaces[$0].id }, into: order)
                    .compactMap { byID[$0] }
            }
            let canReorder =
                !searching && runner.isAnswering && shown.count > 1
                && shown.allSatisfy { workspaces[$0].keepsOrder }
            return ShellRunnerSection(
                runner: runner,
                cards: shown.map { ShellCard(id: workspaces[$0].id, index: $0) },
                canReorder: canReorder)
        }
    }

    /// This fleet as the runner will have it once `request` is applied: that
    /// runner's worktrees permuted among the slots they hold, and every other
    /// runner's exactly where it was.
    ///
    /// What `ShellHarness` does in place of a runner, so the drag can be driven
    /// against a canned fleet and the grid seen to answer it.
    func reordered(_ request: ShellReorderRequest) -> ShellFleet {
        let ids = workspaces.map(\.id)
        let mine = Set(
            workspaces.filter { $0.runner == request.runner }.map(\.id))
        let order = request.order.filter { mine.contains($0) }
        let byID = Dictionary(
            workspaces.map { ($0.id, $0) }, uniquingKeysWith: { first, _ in first })
        return ShellFleet(
            workspaces: permuting(ids, into: order).compactMap { byID[$0] })
    }
}
