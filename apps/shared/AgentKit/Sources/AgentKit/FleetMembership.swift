import Foundation

// Which runners a phone should be talking to, and what changing that list
// costs.
//
// The arithmetic behind `FleetStore`, with no connection in it and no phone.
// Here rather than beside that store for the reason `ShellNavigation`,
// `PaneDrafts` and `KeyedCallbacks` are here: the iOS target has no unit tests,
// only UI tests, and CI compiles that suite without ever running it. What is
// left in the store is the part that genuinely needs a `Connection` — starting
// one, tearing one down, subscribing to what it publishes. Every DECISION about
// which runners those are is this file, and this file runs under
// `swift test --package-path apps/shared/AgentKit`.
//
// Two rules in here are the ones that were expensive to get wrong on the other
// two platforms, and both are pinned by `FleetMembershipTests`:
//
// - **A runner whose details are unchanged is kept, never replaced.** The Mac's
//   `FleetStore.rebuild()` says it directly — "existing clients are kept rather
//   than replaced, because replacing one would drop a live connection and its
//   fleet with it" — and a reconcile that rebuilt everything on every runner
//   list change would look correct while silently costing a reconnect per
//   keystroke in the runner editor.
// - **A runner whose details were EDITED is torn down and rebuilt.** Android's
//   `FleetRepository.reconcile()` owns this sentence: correcting a mistyped
//   address is as much a change of runner as picking a different one, and
//   reusing the connection would leave the old session running under new
//   details.
//
// Generic over the runner type rather than importing one, because there isn't
// one to import: `Runner` is declared in the iOS target's `Store.swift` and
// this package cannot see it. `Identifiable & Equatable` is the whole surface
// the rules above need — an id to key a connection by, and equality to answer
// "are these the same details I dialed with".
enum FleetMembership {
    /// The runners a reconcile should receive.
    ///
    /// The battery gate, and the one place the `allRunnersAtOnce` setting
    /// means anything. Android's `FleetRepository.init` combines exactly these
    /// three values and this is that expression, moved somewhere a test can
    /// read it back.
    ///
    /// The Mac has no equivalent and needs none: it connects to everything
    /// unconditionally, because a Mac on mains does not pay what a phone pays
    /// in radio wake-ups per extra SSH session. Both phones do, so both phones
    /// get the choice.
    ///
    /// With the gate off and nothing selected the answer is no runners at all,
    /// which is deliberate rather than a hole: "talk only to the runner I
    /// picked" is not satisfiable by picking one for somebody. The same is true
    /// of a selection naming a runner that has since been removed — the filter
    /// simply matches nothing, and the next selection is what fixes it.
    static func wanted<Runner: Identifiable>(
        all: [Runner], selected: Runner.ID?, everyRunnerAtOnce: Bool
    ) -> [Runner] {
        guard !everyRunnerAtOnce else { return all }
        return all.filter { $0.id == selected }
    }

    /// What a reconcile has to do, with nothing done yet.
    ///
    /// Four disjoint lists rather than the two loops that produce them, so that
    /// "kept" is a thing a test can assert on. It is the field that carries the
    /// rule with no visible symptom: a connection that is torn down and
    /// immediately built again looks identical on screen a second later and
    /// costs a full SSH bring-up every time the runner list is touched.
    struct Plan<ID: Hashable>: Equatable {
        /// Runners with no connection at all. Bring one up.
        var started: [ID] = []
        /// Runners already connected under exactly these details. Leave them
        /// alone — this is the list that must not shrink into `rebuilt`.
        var kept: [ID] = []
        /// Runners whose details were edited. Retire the connection, then start
        /// a fresh one, because the live session belongs to the old details.
        var rebuilt: [ID] = []
        /// Connections whose runner is no longer wanted — removed, or filtered
        /// out by the gate above. Retire, and do not start anything.
        ///
        /// A set, and not an array, because the order teardown happens in
        /// carries no meaning: each of these is an independent session being
        /// dropped. The other three are ordered, because they are the order the
        /// runners themselves are in.
        var retired: Set<ID> = []
    }

    /// Bring `existing` into line with `wanted`.
    ///
    /// `existing` maps a runner's id to the details its connection was DIALED
    /// with, which is what makes an edit detectable at all. Keying it on the
    /// current details instead would compare a value with itself and never
    /// find one.
    static func plan<Runner: Identifiable & Equatable>(
        wanted: [Runner], existing: [Runner.ID: Runner]
    ) -> Plan<Runner.ID> {
        var plan = Plan<Runner.ID>()
        var seen: Set<Runner.ID> = []

        for runner in wanted {
            // A runner listed twice is one runner. Nothing in the app produces
            // that today, and a reconcile that answered it by starting a second
            // connection under the same key would leak the first one — the
            // dictionary in the store holds one value per id, so the loser
            // would be running with nothing left pointing at it.
            guard seen.insert(runner.id).inserted else { continue }
            guard let dialed = existing[runner.id] else {
                plan.started.append(runner.id)
                continue
            }
            if dialed == runner {
                plan.kept.append(runner.id)
            } else {
                plan.rebuilt.append(runner.id)
            }
        }

        for id in existing.keys where !seen.contains(id) {
            plan.retired.insert(id)
        }

        return plan
    }

    /// One list from N, in the order the runners are listed.
    ///
    /// The merged publish, minus the merging: what a screen reads has to be
    /// stable under a runner going quiet, so the order comes from the runner
    /// list a person arranged and never from a dictionary's own.
    ///
    /// A runner with no connection contributes nothing rather than a gap. Two
    /// different things arrive here that way and both are ordinary: one the
    /// battery gate filtered out, and one whose bring-up has not happened yet.
    ///
    /// Driven by `order` rather than by `live` deliberately, so a connection
    /// left under an id that is no longer in the runner list cannot reach the
    /// screen. That is the shape of the merge bug worth naming: a runner nobody
    /// is polling any more, still contributing its last rows, indefinitely.
    static func published<ID: Hashable, Value>(order: [ID], live: [ID: Value]) -> [Value] {
        order.compactMap { live[$0] }
    }
}
