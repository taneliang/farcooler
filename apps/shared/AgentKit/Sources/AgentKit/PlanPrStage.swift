import Foundation

// Where a ticket's pull request stands (ov-316, team mode T5): the
// runner derives the stage on read (`pr_stage.rs`, ov-312) and sends its kind
// and facts; this file words it in the apps' own style, for the Mac now and the
// phones next (ov-317). Every field is optional on the wire: a runner before
// ov-312 sends no `stage`, and a kind this build does not know decodes as
// `.unspecified`, which says nothing.

/// The stage's kind, as `plan --json` spells it.
public enum PlanPrStageKind: String, Decodable, Sendable, CaseIterable {
    case building, agentReview = "agent_review", fixing
    case waitingOnReviewer = "waiting_on_reviewer"
    case changesRequested = "changes_requested"
    case approvedChecksRunning = "approved_checks_running"
    case approvedChecksFailing = "approved_checks_failing"
    case approved, approvedConflicts = "approved_conflicts"
    case queued, merged, closed
    /// The runner could not ask GitHub. Never the same as `.building`, which
    /// is a runner that asked and found no pull request.
    case unknown
    /// A kind a newer runner sent.
    case unspecified

    public init(from decoder: Decoder) throws {
        self = PlanPrStageKind(rawValue: try decoder.singleValueContainer().decode(String.self)) ?? .unspecified
    }
}

public struct PlanPrStage: Decodable, Equatable, Hashable, Sendable {
    public var kind: PlanPrStageKind
    /// The runner's own words, for a kind this build cannot word.
    public var label: String
    /// Whom a review waits on, for `.waitingOnReviewer`: GitHub logins and
    /// team slugs, in GitHub's order.
    public var reviewers: [String]
    /// From 1, for `.queued`.
    public var queuePosition: Int
    /// 0 for none.
    public var prNumber: Int
    public var prUrl: String
    public var unresolvedThreads: Int?
    /// `passing`, `failing`, `pending` or `unknown`.
    public var checks: String
    public var readAt: Int64

    public init(
        kind: PlanPrStageKind, label: String = "", reviewers: [String] = [], queuePosition: Int = 0, prNumber: Int = 0,
        prUrl: String = "", unresolvedThreads: Int? = nil, checks: String = "unknown", readAt: Int64 = 0
    ) {
        self.kind = kind
        self.label = label
        self.reviewers = reviewers
        self.queuePosition = queuePosition
        self.prNumber = prNumber
        self.prUrl = prUrl
        self.unresolvedThreads = unresolvedThreads
        self.checks = checks
        self.readAt = readAt
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        kind = try c.decodeIfPresent(PlanPrStageKind.self, forKey: .kind) ?? .unspecified
        label = try c.decodeIfPresent(String.self, forKey: .label) ?? ""
        reviewers = try c.decodeIfPresent([String].self, forKey: .reviewers) ?? []
        queuePosition = try c.decodeIfPresent(Int.self, forKey: .queuePosition) ?? 0
        prNumber = try c.decodeIfPresent(Int.self, forKey: .prNumber) ?? 0
        prUrl = try c.decodeIfPresent(String.self, forKey: .prUrl) ?? ""
        unresolvedThreads = try c.decodeIfPresent(Int.self, forKey: .unresolvedThreads)
        checks = try c.decodeIfPresent(String.self, forKey: .checks) ?? "unknown"
        readAt = try c.decodeIfPresent(Int64.self, forKey: .readAt) ?? 0
    }

    private enum CodingKeys: String, CodingKey {
        case kind, label, reviewers, queuePosition, prNumber, prUrl, unresolvedThreads, checks, readAt
    }

    /// Whether the pull request is a draft: the runner derives Agent Review and
    /// Fixing from a draft and from nothing else (`pr_stage::derive`).
    public var isDraft: Bool { kind == .agentReview || kind == .fixing }

    /// Whether a person may mark it ready for review from here: a draft the
    /// runner named (`prNumber`), not one it could not read.
    public var canMarkReady: Bool { isDraft && prNumber > 0 }

    /// "PR 31 · 2 threads open": the facts under the words, nil without a
    /// pull request.
    public var detail: String? {
        guard prNumber > 0 else { return nil }
        var parts = ["PR \(prNumber)"]
        if let n = unresolvedThreads, n > 0 { parts.append(n == 1 ? "1 thread open" : "\(n) threads open") }
        return parts.joined(separator: " \u{00B7} ")
    }

    /// The pull request's page, only when it is a web link.
    public var link: URL? {
        guard let url = URL(string: prUrl), url.scheme == "https" else { return nil }
        return url
    }
}
