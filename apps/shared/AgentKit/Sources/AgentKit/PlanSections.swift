import Foundation

// The Plan view as sections (ov-503, design 2): what moves main first, then
// what is queued, then what it costs, then why. Every rule is here, as values:
// which lane is in which section, which train is furthest along, what a lane
// inside a train says about itself, and what each section says when it is
// empty. The views only lay it out, so the Mac, the phones and the watch's
// glance read one answer.
//
// Order (owner decision 7): Needs You, Shipping, In Review, Building,
// Waiting, Agents, Themes.
//
// A lane belongs to exactly one place:
// - on a live train, it's in that train's card (Shipping), whatever its state;
// - naming a train that landed while it's still queued or landing, it's in On
//   a Landed Train (folded, and each row says which train);
// - in review, fixing, or landing with no train, it's In Review;
// - building, it's Building;
// - a lane building, in review or fixing is never hidden by a landed train: it
//   routes by its state and the row says it names that train (`landedTrain`);
// - queued, it's Waiting on You when a card needs a decision, else Next Up
//   when the plan ranks it, else Not Ranked.
// A lane named like a live train is that train's agent and is left out (ov-461).

public struct PlanSections: Equatable, Sendable {
    public enum Kind: String, CaseIterable, Sendable {
        case needsYou, shipping, inReview, building, waiting, onLandedTrain, agents, themes

        public var title: String {
            switch self {
            case .needsYou: "Needs You"
            case .shipping: "Shipping"
            case .inReview: "In Review"
            case .building: "Building"
            case .waiting: "Waiting"
            case .onLandedTrain: "On a Landed Train"
            case .agents: "Agents"
            case .themes: "Themes"
            }
        }

        /// What the section says with nothing in it, for the ones that say
        /// anything.
        public var emptyWords: String? {
            switch self {
            case .shipping: "No train is shipping."
            case .building: "Nothing is building."
            case .waiting: "Nothing is queued."
            case .agents: PlanWords.noAgents
            case .needsYou, .inReview, .onLandedTrain, .themes: nil
            }
        }

        /// Drawn folded until opened.
        public var startsFolded: Bool { self == .onLandedTrain }
    }

    /// A lane row in In Review, Building or Waiting.
    public struct LaneRow: Equatable, Identifiable, Sendable {
        public var lane: PlanLane
        /// The agent that works it, for the line under its title.
        public var agent: PlanFleetAgent?
        /// One of its cards needs the owner's decision.
        public var waitsOnOwner: Bool
        /// The heading of the landed train the lane still names, or nil.
        public var landedTrain: String?
        public var id: String { lane.id }
    }

    /// How a lane stands relative to the train it's on.
    public enum Standing: Equatable, Sendable {
        case ready
        case fixing(round: Int)
        case landed(sha: String?)
        /// Not ready yet: queued, building or in review.
        case notReady(LaneState)

        public var word: String {
            switch self {
            case .ready: "Ready"
            case .fixing(let round): round > 0 ? "Fixing · round \(round)" : "Fixing"
            case .landed(let sha): sha.map { $0.isEmpty ? "Landed" : "Landed · \($0.prefix(8))" } ?? "Landed"
            case .notReady(let state): state.word
            }
        }
    }

    public struct TrainLane: Equatable, Identifiable, Sendable {
        public var id: String
        public var name: String
        public var lane: PlanLane
        public var standing: Standing
        /// The train is held by this lane.
        public var holding: Bool
        /// One of its cards needs the owner's decision.
        public var waitsOnOwner: Bool
    }

    /// A live train and its lanes.
    public struct TrainCard: Equatable, Identifiable, Sendable {
        public var train: PlanTrain
        public var phase: TrainPhase
        public var since: Int64
        public var lanes: [TrainLane]
        /// "Holding for ios-overwrite · 2 h", or nil.
        public var held: String?
        public var id: String { train.id }
    }

    public struct Waiting: Equatable, Sendable {
        /// Ranked, in the plan's order.
        public var next: [LaneRow]
        public var notRanked: [LaneRow]
        public var onYou: [LaneRow]
        public var isEmpty: Bool { next.isEmpty && notRanked.isEmpty && onYou.isEmpty }
        public var count: Int { next.count + notRanked.count + onYou.count }
    }

    /// The Needs You count the title bar says; the section's rows are the
    /// runner's own list, which the plan doesn't carry.
    public var needsYou: Int
    /// Live trains, furthest along first.
    public var shipping: [TrainCard]
    public var inReview: [LaneRow]
    public var building: [LaneRow]
    public var waiting: Waiting
    public var fleet: PlanFleet
    public var themes: PlanThemeGrid
    /// Queued or landing lanes that name a train that has landed.
    public var onLandedTrain: [LaneRow]
    /// Whether the runner lists agents (`board_agents`); without it the
    /// Agents section isn't drawn, since "No agents running" would be false.
    public var agentsSupported: Bool
    /// The runner's clock.
    public var now: Int64

    /// - Parameters:
    ///   - needsYou: the workspace's Needs You count.
    ///   - statuses: the board's card statuses by task id, which say whether
    ///     a card waits on a decision and which glyph a theme's card gets.
    ///   - activity: when each card last moved (ms), by task id.
    ///   - agentsSupported: whether the runner advertises `board_agents`.
    public init(
        plan: PlanModel, needsYou: Int = 0, statuses: [String: TaskStatus] = [:], activity: [String: Int64] = [:],
        agentsSupported: Bool = true
    ) {
        self.needsYou = max(0, needsYou)
        self.agentsSupported = agentsSupported
        now = plan.nowMs
        let isTrainAgent = Set(plan.laneIsTrain.map(\.lane))
        let candidates = plan.lanes.filter { !isTrainAgent.contains($0.id) }

        func agent(of lane: PlanLane) -> PlanFleetAgent? {
            let mine = plan.agents.filter { $0.lane?.id == lane.id && $0.role != .orchestrator }
            return mine.sorted {
                if $0.working != $1.working { return $0.working }
                return $0.spend.totalTokens > $1.spend.totalTokens
            }.first
        }
        // A live lane on a landed train names it still.
        let landedTrains = plan.trains.filter { $0.state == .landed }
        func landed(_ lane: PlanLane) -> PlanTrain? {
            landedTrains.first { train in
                train.lanes.contains { $0.lane == lane.id } || lane.train?.lowercased() == train.name.lowercased()
            }
        }
        func row(_ lane: PlanLane) -> LaneRow {
            LaneRow(
                lane: lane, agent: agent(of: lane), waitsOnOwner: plan.waitsOnOwner(lane, statuses: statuses),
                landedTrain: landed(lane)?.heading)
        }

        // Trains, furthest along first; the plan's order breaks a tie.
        let live = plan.liveTrains.enumerated().sorted {
            let (a, b) = ($0.element.phase(in: plan).progress, $1.element.phase(in: plan).progress)
            return a != b ? a > b : $0.offset < $1.offset
        }.map(\.element)
        var onLive = Set<String>()
        shipping = live.map { train in
            let phase = train.phase(in: plan)
            let since = train.phaseSince(in: plan)
            let holders = Set(train.holders.map(\.lane))
            let mine = plan.lanes(on: train).filter { !isTrainAgent.contains($0.id) }
            onLive.formUnion(mine.map(\.id))
            return TrainCard(
                train: train, phase: phase, since: since,
                lanes: mine.map { lane in
                    TrainLane(
                        id: lane.id, name: lane.name, lane: lane, standing: Self.standing(lane), holding: holders.contains(lane.id),
                        waitsOnOwner: plan.waitsOnOwner(lane, statuses: statuses))
                },
                held: PlanWords.held(train, since: since, now: plan.nowMs))
        }

        let free = candidates.filter { $0.state.isLive && !onLive.contains($0.id) }
        // Only queued and landing lanes go to the folded section; the rest
        // route by their state and are flagged.
        let parked = free.filter { ($0.state == .queued || $0.state == .landing) && landed($0) != nil }
        onLandedTrain = parked.map(row)
        let parkedIDs = Set(parked.map(\.id))
        let loose = free.filter { !parkedIDs.contains($0.id) }

        inReview = loose.filter { [.review, .fixing, .landing].contains($0.state) }.map(row)
        building = loose.filter { $0.state == .building }.map(row)
        let queued = loose.filter { $0.state == .queued }
        let blocked = queued.filter { plan.waitsOnOwner($0, statuses: statuses) }
        let blockedIDs = Set(blocked.map(\.id))
        let ranked = plan.order.compactMap { id in queued.first { $0.id == id && !blockedIDs.contains(id) } }
        let rankedIDs = Set(ranked.map(\.id))
        waiting = Waiting(
            next: ranked.map(row), notRanked: queued.filter { !rankedIDs.contains($0.id) && !blockedIDs.contains($0.id) }.map(row),
            onYou: blocked.map(row))

        fleet = PlanFleet(plan.agents)
        let moving = Set(shipping.flatMap { $0.lanes.map(\.id) } + (inReview + building).map(\.id))
        themes = PlanThemeGrid(plan: plan, moving: moving, statuses: statuses, activity: activity)
    }

    /// A lane's standing on its train.
    static func standing(_ lane: PlanLane) -> Standing {
        switch lane.state {
        case .landing: .ready
        case .fixing: .fixing(round: lane.fixRounds)
        case .landed: .landed(sha: lane.landedSha)
        default: .notReady(lane.state)
        }
    }

    // MARK: Which sections are drawn

    /// The sections to draw, in order. Needs You, In Review and Themes are
    /// drawn only with rows; Shipping, Waiting and Agents always are, and say
    /// so when empty; Building also when something is queued, since the gap
    /// between Next and Shipping is then worth saying. Agents is left out for
    /// a runner that lists none.
    public var shown: [Kind] {
        Kind.allCases.filter { kind in
            switch kind {
            case .needsYou: needsYou > 0
            case .shipping, .waiting: true
            case .onLandedTrain: !onLandedTrain.isEmpty
            case .inReview: !inReview.isEmpty
            case .building: !building.isEmpty || !waiting.next.isEmpty
            case .agents: agentsSupported
            case .themes: !themes.isEmpty
            }
        }
    }

    /// Rows in a section, for deciding whether it's empty. Needs You counts
    /// its own.
    public func count(_ kind: Kind) -> Int {
        switch kind {
        case .needsYou: needsYou
        case .shipping: shipping.count
        case .inReview: inReview.count
        case .building: building.count
        case .waiting: waiting.count
        case .onLandedTrain: onLandedTrain.count
        case .agents: fleet.working.count + fleet.finished.count
        case .themes: themes.tiles.count + themes.finished.count
        }
    }

    /// What the section says now: its empty words with nothing in it, else nil.
    public func emptyWords(_ kind: Kind) -> String? {
        count(kind) == 0 ? kind.emptyWords : nil
    }
}
