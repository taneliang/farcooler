import Foundation

// The Plan's glance band (ov-503, design 2): one line for Now and one for
// Next. `PlanStrip` carries it, so the folded strip, the phones and the
// watch's glance read the same sentence. It is not `PlanGlance`, which is the
// board as the relay hands it to the watch.
//
//   NOW   Train 86 waiting for CI · 2 building · 3 in review
//   NEXT  reps — Each person's asks reach the orchestrator, then skill-init
//
// The Now line names the furthest-along train first, because shipping is what
// moves main, then counts what's building and in review. The Next line names
// the first ranked lane and the second.

public struct PlanGlanceSentence: Equatable, Sendable {
    /// "Train 86 waiting for CI · 2 building · 3 in review"; nil when nothing
    /// is shipping, building or in review.
    public var now: String?
    /// "reps — Each person's asks…, then skill-init"; nil when nothing is
    /// ranked.
    public var next: String?

    public init(now: String? = nil, next: String? = nil) {
        self.now = now
        self.next = next
    }

    public init(sections: PlanSections) {
        var parts: [String] = []
        if let lead = sections.shipping.first {
            parts.append("\(lead.train.heading) \(lead.phase.sentence)")
            let more = sections.shipping.count - 1
            if more > 0 { parts.append(more == 1 ? "1 more train" : "\(more) more trains") }
        }
        if !sections.building.isEmpty { parts.append("\(sections.building.count) building") }
        if !sections.inReview.isEmpty { parts.append("\(sections.inReview.count) in review") }
        now = parts.isEmpty ? nil : parts.joined(separator: " · ")

        let ranked = sections.waiting.next.map(\.lane)
        if let first = ranked.first {
            // The lane's title after its name, when it has one.
            var said = first.slug == nil ? first.name : "\(first.name) \u{2014} \(first.heading)"
            if ranked.count > 1 { said += ", then \(ranked[1].name)" }
            next = said
        } else {
            next = nil
        }
    }

    public static let nothingRunning = "Nothing running"
    public static let nothingQueued = "Nothing queued"

    /// The Now line as the band says it.
    public var nowLine: String { now ?? Self.nothingRunning }

    /// The Next line as the band says it.
    public var nextLine: String { next ?? Self.nothingQueued }

    /// Both as one sentence: "Nothing running. Next: nothing queued."
    public var text: String {
        let next = next ?? Self.nothingQueued.lowercased()
        return "\(nowLine). Next: \(next)."
    }
}
