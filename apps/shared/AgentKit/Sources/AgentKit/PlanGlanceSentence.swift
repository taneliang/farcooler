import Foundation

// The Plan's glance band (ov-503, design 2): one line for Now and one for
// Next. `PlanStrip` carries it, so the folded strip, the phones and the
// watch's glance read the same sentence. It is not `PlanGlance`, which is the
// board as the relay hands it to the watch.
//
//   NOW   Landing 86 checking on CI · 2 building · 3 in review
//   NEXT  Reps: each person's asks reach the orchestrator, then Onboarding: skill-init…
//
// The Now line names the furthest-along landing first, because landing is what
// moves main, then counts what's building and in review. The
// Next line names the first ranked ticket and the second, or, with none ranked,
// counts the tickets that wait: "4 waiting, none ranked" (ov-522).
//
// The landing is "Landing 86", never its title: a title can run to a line, and
// the phase after it is what the band is for (ov-514). The landing card shows
// the title.

public struct PlanGlanceSentence: Equatable, Sendable {
    /// "Landing 86 checking on CI · 2 building · 3 in review"; nil when nothing
    /// is landing, building or in review.
    public var now: String?
    /// "Reps: each person's asks…, then Onboarding: skill-init…", or "4
    /// waiting, none ranked"; nil when nothing is queued.
    public var next: String?

    public init(now: String? = nil, next: String? = nil) {
        self.now = now
        self.next = next
    }

    /// The band for the Plan's view as the Rust core built it: the sentence is
    /// still put together here (ov-1015 phase 2), from the view's sections.
    public init(view: WirePlanView) {
        var parts: [String] = []
        if let lead = view.landings.first {
            parts.append("\(lead.label) \(lead.phaseSentence)")
            let more = view.landings.count - 1
            if more > 0 { parts.append(more == 1 ? "1 more landing" : "\(more) more landings") }
        }
        if !view.building.isEmpty { parts.append("\(view.building.count) building") }
        if !view.inReview.isEmpty { parts.append("\(view.inReview.count) in review") }
        now = parts.isEmpty ? nil : parts.joined(separator: " · ")

        let ranked = view.waiting(.next)
        if let first = ranked.first {
            // Titles only, each said once and cut at a word: the ticket's key
            // is in its title often enough, and never twice.
            var said = Self.title(first.heading)
            if let second = ranked.dropFirst().first(where: { $0.id != first.id && $0.heading != first.heading }) {
                said += ", then \(Self.title(second.heading))"
            }
            next = said
        } else if view.waitingCount > 0 {
            // Tickets wait and none is ranked: "Nothing queued" would be false.
            next = "\(view.waitingCount) waiting, none ranked"
        } else {
            next = nil
        }
    }

    /// How long each ticket's title is in the Next line, before it is cut at a word.
    public static let titleMax = 72

    /// A ticket's title cut at a word, with no comma or colon left hanging
    /// before the ellipsis.
    static func title(_ heading: String) -> String {
        let cut = PlanWords.short(heading, max: titleMax)
        guard cut.hasSuffix("\u{2026}") else { return cut }
        var head = cut.dropLast()
        while let last = head.last, ",;:\u{2014}- ".contains(last) { head = head.dropLast() }
        return head + "\u{2026}"
    }

    /// "1 needs you", "2 need you": the band's pill.
    public static func needsYouWords(_ count: Int) -> String { "\(count) \(count == 1 ? "needs" : "need") you" }

    /// About tickets, as the band is: the Agents card beside it counts agents,
    /// and "Nothing running" there contradicted it (ov-616 review).
    public static let nothingRunning = "No tickets moving"
    public static let nothingQueued = "Nothing queued"

    /// The Now line as the band says it.
    public var nowLine: String { now ?? Self.nothingRunning }

    /// The Now line with each "·"-separated span held together, for a view that
    /// wraps: "2 building" is not cut after "2" (ov-515). The words are `nowLine`'s.
    public var nowLineUnbroken: String { PlanWords.unbroken(nowLine) }

    /// The Next line as the band says it.
    public var nextLine: String { next ?? Self.nothingQueued }

    /// Both as one sentence: "No tickets moving. Next: nothing queued."
    public var text: String {
        let next = next ?? Self.nothingQueued.lowercased()
        return "\(nowLine). Next: \(next)."
    }
}
