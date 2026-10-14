import Foundation

// The Plan's glance band (ov-503, design 2): one line for Now and one for
// Next. `PlanStrip` carries it, so the folded strip, the phones and the
// watch's glance read the same sentence. It is not `PlanGlance`, which is the
// board as the relay hands it to the watch.
//
//   NOW   Train 86 waiting for CI · 2 building · 3 in review
//   NEXT  Reps: each person's asks reach the orchestrator, then Onboarding: skill-init…
//
// The Now line names the furthest-along train first, because shipping is what
// moves main, then counts what's building and in review. The Next line names
// the first ranked lane and the second.
//
// The train is "Train 86", never its title: a title can run to a line, and
// the phase after it is what the band is for (ov-514). The train card shows
// the title.

public struct PlanGlanceSentence: Equatable, Sendable {
    /// "Train 86 waiting for CI · 2 building · 3 in review"; nil when nothing
    /// is shipping, building or in review.
    public var now: String?
    /// "Reps: each person's asks…, then Onboarding: skill-init…"; nil when
    /// nothing is ranked.
    public var next: String?

    public init(now: String? = nil, next: String? = nil) {
        self.now = now
        self.next = next
    }

    public init(sections: PlanSections) {
        var parts: [String] = []
        if let lead = sections.shipping.first {
            parts.append("\(lead.train.label) \(lead.phase.sentence)")
            let more = sections.shipping.count - 1
            if more > 0 { parts.append(more == 1 ? "1 more train" : "\(more) more trains") }
        }
        if !sections.building.isEmpty { parts.append("\(sections.building.count) building") }
        if !sections.inReview.isEmpty { parts.append("\(sections.inReview.count) in review") }
        now = parts.isEmpty ? nil : parts.joined(separator: " · ")

        let ranked = sections.waiting.next.map(\.lane)
        if let first = ranked.first {
            // Titles only, each said once and cut at a word: the lane's name
            // is in its title often enough, and never twice.
            var said = Self.title(first)
            if let second = ranked.dropFirst().first(where: { $0.id != first.id && $0.heading != first.heading }) {
                said += ", then \(Self.title(second))"
            }
            next = said
        } else {
            next = nil
        }
    }

    /// How long each lane's title is in the Next line, before it is cut at a word.
    public static let titleMax = 72

    /// A lane's title cut at a word, with no comma or colon left hanging
    /// before the ellipsis.
    static func title(_ lane: PlanLane) -> String {
        let cut = PlanWords.short(lane.heading, max: titleMax)
        guard cut.hasSuffix("\u{2026}") else { return cut }
        var head = cut.dropLast()
        while let last = head.last, ",;:\u{2014}- ".contains(last) { head = head.dropLast() }
        return head + "\u{2026}"
    }

    /// "1 needs you", "2 need you": the band's pill.
    public static func needsYouWords(_ count: Int) -> String { "\(count) \(count == 1 ? "needs" : "need") you" }

    public static let nothingRunning = "Nothing running"
    public static let nothingQueued = "Nothing queued"

    /// The Now line as the band says it.
    public var nowLine: String { now ?? Self.nothingRunning }

    /// The Now line with each "·"-separated span held together, for a view that
    /// wraps: "2 building" is not cut after "2" (ov-515). The words are `nowLine`'s.
    public var nowLineUnbroken: String { PlanSections.TrainCard.unbroken(nowLine) }

    /// The Next line as the band says it.
    public var nextLine: String { next ?? Self.nothingQueued }

    /// Both as one sentence: "Nothing running. Next: nothing queued."
    public var text: String {
        let next = next ?? Self.nothingQueued.lowercased()
        return "\(nowLine). Next: \(next)."
    }
}
