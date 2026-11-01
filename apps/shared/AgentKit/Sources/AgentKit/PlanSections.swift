import Foundation

// The Plan view as sections (ov-503, design 2): what moves main first, then
// what is waiting, then what it costs, then why. Every rule is here, as values:
// which ticket is in which section, which landing is furthest along, what a
// ticket inside a landing says about itself, and what each section says when
// it is empty. The views only lay it out, so the Mac, the phones and the
// watch's glance read one answer.
//
// Order (owner decision 7; ov-522): Needs You, Landing (the landings), In
// Review, Building, Waiting, Agents, Themes. The word is landing, never
// shipping.
//
// A ticket belongs to exactly one place:
// - on a live landing, it's in that landing's card (Landing), whatever its
//   stage;
// - naming a landing that landed while it's still waiting or landing, it's in
//   Waiting for the Next Landing (folded, and each row says which landing);
// - in review, needing the owner, or landing with no landing, it's In Review;
// - building (or fixing), it's Building;
// - a ticket building or in review is never hidden by a landed landing: it
//   routes by its stage and the row says it names that landing
//   (`landedLanding`);
// - waiting, it's Waiting on You when its card needs a decision, else Next Up
//   when the plan ranks it, else Not Ranked.

public struct PlanSections: Equatable, Sendable {
    public enum Kind: String, CaseIterable, Sendable {
        case needsYou, landing, inReview, building, waiting, onLandedLanding, agents, themes

        public var title: String {
            switch self {
            case .needsYou: "Needs You"
            case .landing: "Landing"
            case .inReview: "In Review"
            case .building: "Building"
            case .waiting: "Waiting"
            case .onLandedLanding: "Waiting for the Next Landing"
            case .agents: "Agents"
            case .themes: "Themes"
            }
        }

        /// What the section says with nothing in it, for the ones that say
        /// anything.
        public var emptyWords: String? {
            switch self {
            case .landing: "Nothing is landing right now."
            case .building: "Nothing is building."
            case .waiting: "Nothing is queued."
            case .agents: PlanWords.noAgents
            case .needsYou, .inReview, .onLandedLanding, .themes: nil
            }
        }

        /// Drawn folded until opened.
        public var startsFolded: Bool { self == .onLandedLanding }
    }

    /// A ticket row in In Review, Building or Waiting.
    public struct TicketRow: Equatable, Identifiable, Sendable {
        public var line: PlanTicketLine
        /// The agent that works it, for the line under its title.
        public var agent: PlanAgent?
        /// It waits on the owner: the runner says so, or its card needs a
        /// decision.
        public var waitsOnOwner: Bool
        /// The label of the landing that landed which the ticket still names,
        /// or nil.
        public var landedLanding: String?
        public var id: String { line.id }
    }

    /// How a ticket stands relative to the landing it's on.
    public enum Standing: Equatable, Sendable {
        case ready
        case fixing(round: Int)
        case landed(sha: String?)
        /// Not ready yet: waiting, building or in review.
        case notReady(TicketStage)

        public var word: String {
            switch self {
            case .ready: "Ready"
            case .fixing(let round): round > 0 ? "Fixing · round \(round)" : "Fixing"
            case .landed(let sha): sha.map { $0.isEmpty ? "Landed" : "Landed · \($0.prefix(8))" } ?? "Landed"
            case .notReady(let stage): stage.word
            }
        }
    }

    public struct LandingTicket: Equatable, Identifiable, Sendable {
        public var id: String { line.id }
        public var key: String
        public var line: PlanTicketLine
        public var standing: Standing
        /// It waits on the owner.
        public var waitsOnOwner: Bool

        /// What a ticket aboard says about itself, or nil when it is simply
        /// aboard (ov-522). "Ready" said nothing: the landing is being tested,
        /// ready for what? Only what matters is said: a ticket that waits on
        /// the owner, and a ticket that has landed.
        public var note: String? {
            if waitsOnOwner { return "Needs you" }
            if case .landed = standing { return standing.word }
            return nil
        }

        /// The note is drawn in amber: the ticket waits on the owner.
        public var noteNeedsAttention: Bool { waitsOnOwner }
    }

    /// A live landing and its tickets.
    public struct LandingCard: Equatable, Identifiable, Sendable {
        public var landing: PlanLanding
        public var phase: LandingPhase
        public var since: Int64
        public var tickets: [LandingTicket]
        /// The agent landing it, when the runner lists one.
        public var lander: PlanAgent?
        public var id: String { landing.id }
    }

    public struct Waiting: Equatable, Sendable {
        /// Ranked, in the plan's order.
        public var next: [TicketRow]
        public var notRanked: [TicketRow]
        public var onYou: [TicketRow]
        public var isEmpty: Bool { next.isEmpty && notRanked.isEmpty && onYou.isEmpty }
        public var count: Int { next.count + notRanked.count + onYou.count }
    }

    /// The Needs You count the title bar says; the section's rows are the
    /// runner's own list, which the plan doesn't carry.
    public var needsYou: Int
    /// Live landings, furthest along first.
    public var landings: [LandingCard]
    public var inReview: [TicketRow]
    public var building: [TicketRow]
    public var waiting: Waiting
    public var fleet: PlanFleet
    public var themes: PlanThemeGrid
    /// Waiting or landing tickets that name a landing that has landed.
    public var onLandedLanding: [TicketRow]
    /// Whether the runner lists agents (`board_line`); without it the
    /// Agents section isn't drawn, since "No agents running" would be false.
    public var agentsSupported: Bool
    /// The runner's clock.
    public var now: Int64

    /// - Parameters:
    ///   - needsYou: the workspace's Needs You count.
    ///   - statuses: the board's card statuses by task id, which say whether
    ///     a card waits on a decision and which glyph a theme's card gets.
    ///   - activity: when each card last moved (ms), by task id.
    ///   - agentsSupported: whether the runner advertises `board_line`.
    public init(
        plan: PlanModel, needsYou: Int = 0, statuses: [String: TaskStatus] = [:], activity: [String: Int64] = [:],
        agentsSupported: Bool = true
    ) {
        self.needsYou = max(0, needsYou)
        self.agentsSupported = agentsSupported
        now = plan.nowMs
        let candidates = plan.lines

        func agent(of line: PlanTicketLine) -> PlanAgent? {
            let mine = plan.agents.filter { agent in agent.role != .orchestrator && agent.tickets.contains { $0.task == line.task } }
            return mine.sorted {
                if $0.working != $1.working { return $0.working }
                return $0.spend.totalTokens > $1.spend.totalTokens
            }.first
        }
        // A live ticket on a landed landing names it still.
        let landedLandings = plan.landings.filter { $0.state == .landed }
        func landed(_ line: PlanTicketLine) -> PlanLanding? {
            landedLandings.first { landing in
                landing.tickets.contains { $0.task == line.task } || line.landing == landing.number
            }
        }
        func row(_ line: PlanTicketLine) -> TicketRow {
            TicketRow(
                line: line, agent: agent(of: line), waitsOnOwner: plan.waitsOnOwner(line, statuses: statuses),
                landedLanding: landed(line)?.label)
        }

        // Landings, furthest along first; the plan's order breaks a tie.
        let live = plan.liveLandings.enumerated().sorted {
            let (a, b) = ($0.element.phase(in: plan).progress, $1.element.phase(in: plan).progress)
            return a != b ? a > b : $0.offset < $1.offset
        }.map(\.element)
        var onLive = Set<String>()
        landings = live.map { landing in
            let phase = landing.phase(in: plan)
            let since = landing.phaseSince(in: plan)
            let mine = plan.lines(on: landing)
            onLive.formUnion(mine.map(\.id))
            return LandingCard(
                landing: landing, phase: phase, since: since,
                tickets: mine.map { line in
                    LandingTicket(
                        key: line.key, line: line, standing: Self.standing(line),
                        waitsOnOwner: plan.waitsOnOwner(line, statuses: statuses))
                }, lander: plan.lander(of: landing))
        }

        let free = candidates.filter { $0.stage.isLive && !onLive.contains($0.id) }
        // Only waiting and landing tickets go to the folded section; the rest
        // route by their stage and are flagged.
        let parked = free.filter { ($0.stage == .waiting || $0.stage == .landing) && landed($0) != nil }
        onLandedLanding = parked.map(row)
        let parkedIDs = Set(parked.map(\.id))
        let loose = free.filter { !parkedIDs.contains($0.id) }

        inReview = loose.filter { [.reviewing, .needsYou, .landing].contains($0.stage) }.map(row)
        building = loose.filter { $0.stage == .building }.map(row)
        let waitingLines = loose.filter { $0.stage == .waiting }
        let blocked = waitingLines.filter { plan.waitsOnOwner($0, statuses: statuses) }
        let blockedIDs = Set(blocked.map(\.id))
        let ranked = waitingLines.filter { $0.planRank != nil && !blockedIDs.contains($0.id) }
            .sorted { ($0.planRank ?? .max) < ($1.planRank ?? .max) }
        let rankedIDs = Set(ranked.map(\.id))
        waiting = Waiting(
            next: ranked.map(row),
            notRanked: waitingLines.filter { !rankedIDs.contains($0.id) && !blockedIDs.contains($0.id) }.map(row),
            onYou: blocked.map(row))

        fleet = PlanFleet(plan.agents, spend: plan.agentsSpend)
        let moving = Set(landings.flatMap { $0.tickets.map(\.id) } + (inReview + building).map(\.id))
        themes = PlanThemeGrid(plan: plan, moving: moving, statuses: statuses, activity: activity)
    }

    /// A ticket's standing on its landing.
    static func standing(_ line: PlanTicketLine) -> Standing {
        if line.isFixing { return .fixing(round: line.fixRounds) }
        switch line.stage {
        case .landing: return .ready
        case .landed: return .landed(sha: line.landedSha)
        default: return .notReady(line.stage)
        }
    }

    // MARK: Which sections are drawn

    /// The sections to draw, in order. Needs You, In Review and Themes are
    /// drawn only with rows; Landing, Waiting and Agents always are, and say
    /// so when empty; Building also when something is queued, since the gap
    /// between Next and Landing is then worth saying. Agents is left out for
    /// a runner that lists none.
    public var shown: [Kind] {
        Kind.allCases.filter { kind in
            switch kind {
            case .needsYou: needsYou > 0
            case .landing, .waiting: true
            case .onLandedLanding: !onLandedLanding.isEmpty
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
        case .landing: landings.count
        case .inReview: inReview.count
        case .building: building.count
        case .waiting: waiting.count
        case .onLandedLanding: onLandedLanding.count
        case .agents: fleet.agents.count
        case .themes: themes.tiles.count + themes.finished.count
        }
    }

    /// What the section says now: its empty words with nothing in it, else nil.
    public func emptyWords(_ kind: Kind) -> String? {
        guard count(kind) == 0 else { return nil }
        // Agents says what the day spent when it spent something.
        return kind == .agents ? fleet.header() : kind.emptyWords
    }
}

extension PlanWords {
    /// "Building · ov-481 · Builder · Sonnet · 380K · 3 h": a ticket row's second
    /// line. The stage with its round and landing, its key, the main agent and its tokens,
    /// and how long the ticket has been in this stage (`aged`, left off when a
    /// warning already says so). `state` replaces the ticket's own words for
    /// its stage: a ticket on a landing says how it stands relative to the
    /// landing ("Ready", "Fixing · round 2").
    public static func ticketSecond(
        _ line: PlanTicketLine, agent: PlanAgent?, landedLanding: String?, now: Int64, aged: Bool = true, state: String? = nil
    ) -> String {
        var parts = [state ?? status(line)]
        if let landedLanding, !parts[0].contains(landedLanding) { parts.append("\(landedLanding) landed") }
        parts.append(line.key)
        if let agent { parts.append([agent.who, agent.tokens()].compactMap { $0 }.joined(separator: " \u{00B7} ")) }
        if aged, line.stageSince > 0, now > line.stageSince { parts.append(span(now - line.stageSince)) }
        return parts.filter { !$0.isEmpty }.joined(separator: " \u{00B7} ")
    }
}
