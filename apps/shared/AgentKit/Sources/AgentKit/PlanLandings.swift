import Foundation

// Landings (ov-309, ov-872) and the CI the runner reads (ov-306), as the plan
// carries them: a landing is tickets going into the product together, with a
// base, the SHA it pushed and where it stands, and the runner reads that SHA's
// CI through `gh` and moves a pushed landing to green or red. The same reads
// draw a page's CI references.
//
// The Mac reads `farcooler plan --json` and the phones the client's
// `plan_json`, both held to `test/fixtures/plan.json`.

/// Where a landing stands.
public enum LandingState: String, Decodable, Sendable, CaseIterable {
    case open, gating, pushed, green, red, landed, dropped, unknown

    public init(from decoder: Decoder) throws {
        self = LandingState(rawValue: try decoder.singleValueContainer().decode(String.self)) ?? .unknown
    }

    /// Landed or dropped: out of Now, and its CI no longer read.
    public var isSettled: Bool { self == .landed || self == .dropped }
}

/// How a landing gets into the product.
public enum LandingRoute: String, Decodable, Sendable, CaseIterable {
    case push, pullRequest = "pull_request", ask, unknown

    public init(from decoder: Decoder) throws {
        self = LandingRoute(rawValue: try decoder.singleValueContainer().decode(String.self)) ?? .unknown
    }
}

public struct PlanLanding: Decodable, Equatable, Identifiable, Sendable {
    public var id: String
    public var short: String
    /// What a person calls it: "Landing 9".
    public var number: Int
    /// What it was cut from: `origin/main`, or a SHA.
    public var base: String
    public var pushedSha: String?
    public var route: LandingRoute
    public var state: LandingState
    public var stateSince: Int64
    /// The phase the runner derived (ov-503). Nil for phase 0: `state` is all
    /// there is, and `PlanLanding.phase(in:)` reads it.
    public var phase: LandingPhase?
    /// When the landing entered `phase`, in ms; nil with it.
    public var phaseSince: Int64?
    /// Gathering tickets that are ready while the landing ahead lands.
    public var collecting: Bool
    public var actor: String
    public var createdAt: Int64
    public var landedAt: Int64?
    /// Its tickets, by card.
    public var tickets: [PlanCardRef]
    /// The CI subject its pushed SHA is read under, or empty before it has one.
    public var ciSubject: String
    /// What it is for: the title the orchestrator wrote, else "Landing 72".
    public var title: String
    /// One line of what it carries: its tickets' titles (ov-462).
    public var summary: String
    /// The agent landing it (ov-872): `PlanAgent.id`. Nil when none is.
    public var lander: String?
    /// What the lander spent on it; nil before it has one.
    public var spend: PlanSpend?

    private enum CodingKeys: String, CodingKey {
        case id, short, number, base, pushedSha, route, state, stateSince, phase, phaseSince, collecting, actor, createdAt
        case landedAt, tickets, ciSubject, title, summary, lander, spend
    }

    /// The runner always sends a title and a summary, but a client's canned
    /// plan may leave either null: that reads as empty, never as an unreadable plan.
    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        id = try c.decode(String.self, forKey: .id)
        short = try c.decode(String.self, forKey: .short)
        number = try c.decode(Int.self, forKey: .number)
        base = try c.decode(String.self, forKey: .base)
        pushedSha = try c.decodeIfPresent(String.self, forKey: .pushedSha)
        route = try c.decode(LandingRoute.self, forKey: .route)
        state = try c.decode(LandingState.self, forKey: .state)
        stateSince = try c.decode(Int64.self, forKey: .stateSince)
        phase = try c.decodeIfPresent(LandingPhase.self, forKey: .phase)
        phaseSince = try c.decodeIfPresent(Int64.self, forKey: .phaseSince)
        collecting = try c.decode(Bool.self, forKey: .collecting)
        actor = try c.decode(String.self, forKey: .actor)
        createdAt = try c.decode(Int64.self, forKey: .createdAt)
        landedAt = try c.decodeIfPresent(Int64.self, forKey: .landedAt)
        tickets = try c.decode([PlanCardRef].self, forKey: .tickets)
        ciSubject = try c.decode(String.self, forKey: .ciSubject)
        title = try c.decodeIfPresent(String.self, forKey: .title) ?? ""
        summary = try c.decodeIfPresent(String.self, forKey: .summary) ?? ""
        lander = try c.decodeIfPresent(String.self, forKey: .lander)
        spend = try c.decodeIfPresent(PlanSpend.self, forKey: .spend)
    }

    /// "Landing 72": what every surface calls the landing, as its heading and
    /// in a sentence (ov-514, ov-522). The title is its subtitle, never its
    /// name: a title can run to a line of its own.
    public var label: String { "Landing \(number)" }

    /// What it is for, under the label: the title without a "Landing 72 ·" the
    /// runner put in front, or nil when it has none or says no more than the
    /// label.
    public var subtitle: String? {
        var text = title
        if text.hasPrefix(label + " \u{00B7} ") { text = String(text.dropFirst(label.count + 3)) }
        return text.isEmpty || text == label ? nil : text
    }

    /// Its tickets' titles on one line, or nil when it carries none.
    public var carries: String? { summary.isEmpty ? nil : summary }
}

/// Where a CI subject stands, over all its runs.
public enum PlanCIStatus: String, Decodable, Sendable, CaseIterable {
    /// `superseded`: nothing failed and a run was canceled, as CI does when a
    /// newer push supersedes it. Neutral, never red.
    case passed, failed, running, queued, superseded, none, unknown

    public init(from decoder: Decoder) throws {
        self = PlanCIStatus(rawValue: try decoder.singleValueContainer().decode(String.self)) ?? .unknown
    }
}

/// One job: "CI / Rust (ubuntu-latest)", and `passed`, `failed`, `running`,
/// `queued`, `skipped` or `canceled`.
public struct PlanCIJob: Decodable, Equatable, Sendable {
    public var name: String
    public var state: String
    public var url: String

    public init(name: String, state: String, url: String = "") {
        self.name = name
        self.state = state
        self.url = url
    }
}

/// What the runner last read of one subject: `main`, `sha:<sha>` or
/// `run:<id>`.
public struct PlanCIRead: Decodable, Equatable, Sendable {
    public var subject: String
    /// The commit its runs are for, in full; empty until a run names it.
    public var sha: String
    public var status: PlanCIStatus
    /// The run's page, or the first failing one's.
    public var url: String
    public var jobs: [PlanCIJob]
    /// When a read last worked: what's here is as of then. 0 if none has.
    public var fetchedAt: Int64
    public var changedAt: Int64
    /// When the runner last asked, answered or not.
    public var askedAt: Int64?

    public init(
        subject: String, sha: String = "", status: PlanCIStatus, url: String = "", jobs: [PlanCIJob] = [],
        fetchedAt: Int64 = 0, changedAt: Int64 = 0, askedAt: Int64? = nil
    ) {
        self.subject = subject
        self.sha = sha
        self.status = status
        self.url = url
        self.jobs = jobs
        self.fetchedAt = fetchedAt
        self.changedAt = changedAt
        self.askedAt = askedAt
    }

    /// A failed run needs the owner: the one CI state drawn in color.
    public var needsAttention: Bool { status == .failed }
}

/// One group of Now: a landing not yet landed and its tickets working now, or
/// the tickets on no such landing.
public struct PlanNowGroup: Equatable, Identifiable, Sendable {
    public var landing: PlanLanding?
    public var lines: [PlanTicketLine]
    public var id: String { landing?.id ?? "tickets" }
}

extension PlanModel {
    /// The read a subject names: `sha:<sha>` matches a read of the same commit
    /// however short either was written; `main` and `run:<id>` match exactly.
    public func ci(_ subject: String) -> PlanCIRead? {
        let subject = subject.lowercased()
        if let exact = ci.first(where: { $0.subject == subject }) { return exact }
        guard subject.hasPrefix("sha:") else { return nil }
        let sha = String(subject.dropFirst(4))
        return ci.first { read in
            let theirs = read.subject.hasPrefix("sha:") ? String(read.subject.dropFirst(4)) : ""
            return (!theirs.isEmpty && (theirs.hasPrefix(sha) || sha.hasPrefix(theirs))) || (!read.sha.isEmpty && read.sha.hasPrefix(sha))
        }
    }

    /// A landing's CI, once the runner has read it.
    public func ci(of landing: PlanLanding) -> PlanCIRead? {
        landing.ciSubject.isEmpty ? nil : ci(landing.ciSubject)
    }

    /// Landings not yet landed or dropped, oldest first.
    public var liveLandings: [PlanLanding] { landings.filter { !$0.state.isSettled } }

    /// The tickets on a landing: those it lists, plus any that name its
    /// number, in the line's order.
    public func lines(on landing: PlanLanding) -> [PlanTicketLine] {
        let tasks = Set(landing.tickets.map(\.task))
        return lines.filter { tasks.contains($0.task) || $0.landing == landing.number }
    }

    /// The landing with this number.
    public func landing(number: Int) -> PlanLanding? { landings.first { $0.number == number } }

    /// The agent landing it, if the runner lists one.
    public func lander(of landing: PlanLanding) -> PlanAgent? { landing.lander.flatMap(agent) }

    /// Now as row groups (ov-309): each live landing heading the tickets on
    /// it, then the tickets on none. A landing with no ticket working now
    /// still heads its group, so a pushed landing waiting on CI is never out
    /// of sight.
    public var nowGroups: [PlanNowGroup] {
        let lines = working
        var grouped = Set<String>()
        var groups: [PlanNowGroup] = []
        for landing in liveLandings {
            let mine = self.lines(on: landing).filter { $0.stage.isPastWaiting }
            grouped.formUnion(mine.map(\.task))
            groups.append(PlanNowGroup(landing: landing, lines: mine))
        }
        let rest = (lines + unranked).filter { !grouped.contains($0.task) }
        if !rest.isEmpty { groups.append(PlanNowGroup(landing: nil, lines: rest)) }
        return groups
    }

    /// Whether Now has anything to draw.
    public var hasNow: Bool { !nowGroups.isEmpty }

    /// How many of the board's cards are in `status` (a board status's word,
    /// or `open`: every card not done or canceled); nil without the counts.
    public func cardCount(_ status: String) -> Int? {
        guard let c = boardCounts else { return nil }
        switch status {
        case "backlog": return c.backlog
        case "todo": return c.todo
        case "needs_decision": return c.needsDecision
        case "in_progress": return c.inProgress
        case "in_review": return c.inReview
        case "done": return c.done
        case "cancelled": return c.cancelled
        case "open": return c.backlog + c.todo + c.needsDecision + c.inProgress + c.inReview
        default: return nil
        }
    }
}

extension PlanWords {
    /// A CI read's status: "Passed", "Failed", "Running", "Queued", "No Runs
    /// Yet", or "CI Unknown" when `gh` couldn't say.
    public static func ciStatus(_ status: PlanCIStatus) -> String {
        switch status {
        case .passed: "Passed"
        case .failed: "Failed"
        case .running: "Running"
        case .queued: "Queued"
        case .superseded: "Superseded"
        case .none: "No Runs Yet"
        case .unknown: "CI Unknown"
        }
    }

    /// How its jobs stand: "1 of 3 jobs failed", "2 of 3 jobs done", "4
    /// jobs"; nil with none read.
    public static func ciJobs(_ read: PlanCIRead) -> String? {
        let total = read.jobs.count
        guard total > 0 else { return nil }
        let n = { (state: String) in read.jobs.filter { $0.state == state }.count }
        let jobs = total == 1 ? "job" : "jobs"
        switch read.status {
        // A canceled job is superseded, not failed (review train-1005c H1).
        case .failed: return "\(n("failed")) of \(total) \(jobs) failed"
        case .running, .queued: return "\(total - n("running") - n("queued")) of \(total) \(jobs) done"
        default: return "\(total) \(jobs)"
        }
    }

    /// Older than this, a read is stale: the runner reads a finished subject
    /// every ten minutes, so this is GitHub not answering (review train-1005c
    /// M1). The CLI and Android hold the same number.
    public static let ciStaleAfterMs: Int64 = 25 * 60_000

    /// "as of 3 h ago" for a read that worked once and not lately; nil while
    /// it's current or never worked.
    public static func ciStale(_ read: PlanCIRead, now: Int64) -> String? {
        guard read.fetchedAt > 0, now - read.fetchedAt > ciStaleAfterMs else { return nil }
        return "as of \(ago(read.fetchedAt, now: now))"
    }

    /// "Failed · 1 of 3 jobs failed": the status and how its jobs stand.
    public static func ciSummary(_ read: PlanCIRead) -> String {
        [ciStatus(read.status), ciJobs(read)].compactMap { $0 }.joined(separator: " · ")
    }

    /// `text` cut at a word to `max` characters, with an ellipsis: a title in
    /// the strip's one line.
    public static func short(_ text: String, max: Int = 24) -> String {
        guard text.count > max else { return text }
        let cut = String(text.prefix(max - 1))
        let head = cut.lastIndex(of: " ").map { String(cut[..<$0]) }.flatMap { $0.count > max / 2 ? $0 : nil } ?? cut
        return head.trimmingCharacters(in: .whitespaces) + "\u{2026}"
    }

    /// Whether a landing needs the owner: red, or its CI failed.
    public static func landingNeedsAttention(_ landing: PlanLanding, ci: PlanCIRead?) -> Bool {
        landing.state == .red || (ci?.needsAttention ?? false)
    }
}
