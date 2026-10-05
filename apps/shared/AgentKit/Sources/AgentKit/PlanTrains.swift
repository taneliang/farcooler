import Foundation

// Trains (ov-309) and the CI the runner reads (ov-306), as the plan carries
// them: a train is lanes landing together, with a base, the SHA it pushed and
// where it stands, and the runner reads that SHA's CI through `gh` and moves
// a pushed train to green or red. The same reads draw a page's CI
// references. EXPERIMENTAL with the plan layer, behind `board_trains`.
//
// The Mac reads `farcooler plan --json` and the phones the client's
// `plan_json`, both held to `test/fixtures/plan.json`.

/// Where a train stands.
public enum TrainState: String, Decodable, Sendable, CaseIterable {
    case integrating, gating, pushed, green, red, landed, dropped, unknown

    public init(from decoder: Decoder) throws {
        self = TrainState(rawValue: try decoder.singleValueContainer().decode(String.self)) ?? .unknown
    }

    /// Landed or dropped: out of Now, and its CI no longer read.
    public var isSettled: Bool { self == .landed || self == .dropped }
}

/// A lane on a train, by id and name.
public struct PlanTrainLane: Decodable, Equatable, Sendable {
    public var lane: String
    public var name: String
}

public struct PlanTrain: Decodable, Equatable, Identifiable, Sendable {
    public var id: String
    public var short: String
    public var name: String
    /// What it was cut from: `origin/main`, or a SHA.
    public var base: String
    public var pushedSha: String?
    public var state: TrainState
    public var stateSince: Int64
    public var actor: String
    public var createdAt: Int64
    public var landedAt: Int64?
    /// Its lanes, oldest first, finished ones included.
    public var lanes: [PlanTrainLane]
    /// The CI subject its pushed SHA is read under, or empty before it has one.
    public var ciSubject: String
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

/// One group of Now: a train not yet landed and its lanes working now, or the
/// lanes on no such train.
public struct PlanNowGroup: Equatable, Identifiable, Sendable {
    public var train: PlanTrain?
    public var lanes: [PlanLane]
    public var id: String { train?.id ?? "lanes" }
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

    /// A train's CI, once the runner has read it.
    public func ci(of train: PlanTrain) -> PlanCIRead? {
        train.ciSubject.isEmpty ? nil : ci(train.ciSubject)
    }

    /// Trains not yet landed or dropped, oldest first.
    public var liveTrains: [PlanTrain] { trains.filter { !$0.state.isSettled } }

    /// Now as row groups (ov-309): each live train heading the lanes on it,
    /// then the lanes on none. A train with no lane working now still heads
    /// its group, so a pushed train waiting on CI is never out of sight.
    public var nowGroups: [PlanNowGroup] {
        let lanes = working + unranked
        var grouped = Set<String>()
        var groups: [PlanNowGroup] = []
        for train in liveTrains {
            let ids = Set(train.lanes.map(\.lane))
            let mine = lanes.filter { ids.contains($0.id) }
            grouped.formUnion(mine.map(\.id))
            groups.append(PlanNowGroup(train: train, lanes: mine))
        }
        let rest = lanes.filter { !grouped.contains($0.id) }
        if !rest.isEmpty { groups.append(PlanNowGroup(train: nil, lanes: rest)) }
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
    /// A train's state: "Integrating", "Pushed", "Red".
    public static func trainState(_ state: TrainState) -> String {
        switch state {
        case .integrating: "Integrating"
        case .gating: "Gating"
        case .pushed: "Pushed"
        case .green: "Green"
        case .red: "Red"
        case .landed: "Landed"
        case .dropped: "Dropped"
        case .unknown: "Unknown"
        }
    }

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

    /// A train's line under its name: "Red · c85bf83d · CI Failed · 1 of 3
    /// jobs failed", or "CI not read yet" once pushed and before the runner
    /// has read it.
    public static func train(_ train: PlanTrain, ci: PlanCIRead?, now: Int64 = 0) -> String {
        var parts = [trainState(train.state)]
        if let sha = train.pushedSha, !sha.isEmpty { parts.append(String(sha.prefix(8))) }
        if let ci {
            parts.append("CI \(ciSummary(ci))")
            if let stale = ciStale(ci, now: now) { parts.append(stale) }
        } else if train.pushedSha != nil && !train.state.isSettled {
            parts.append("CI not read yet")
        }
        return parts.joined(separator: " · ")
    }

    /// Whether a train needs the owner: red, or its CI failed.
    public static func trainNeedsAttention(_ train: PlanTrain, ci: PlanCIRead?) -> Bool {
        train.state == .red || (ci?.needsAttention ?? false)
    }
}
