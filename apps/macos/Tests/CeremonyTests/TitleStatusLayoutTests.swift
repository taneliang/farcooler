import AgentKit
import Foundation
import Testing

@testable import Far_Cooler

/// What the title bar's center shows as the window narrows (ov-469): at the
/// minimum width, at 900 and at 1400, no counter is ever too wide for its
/// room, and what's given up goes in order, Activity first and the need-you
/// count last, never a piece cut short.
@MainActor
struct TitleStatusLayoutTests {
    private static let row = TaskRow(id: "t", key: "ov-1", title: "T", status: .inProgress, statusSince: Date(timeIntervalSince1970: 0))
    private static let line = ActivityLine(
        id: "f", title: "ov-9 Failing", detail: "Failed", status: .failed, kind: .queued(task: "ov-9"), taskKey: "ov-9")

    private static func model(needYou: Int, doing: String?) -> TitleStatus.Model {
        TitleStatus.Model(
            orchestrator: .working, status: .working, nowDoing: doing, needYou: needYou, running: [row], inReview: [row],
            queued: 2, failed: [line])
    }

    private static let room = TitleStatusRoom(
        switcherTitle: "Main", switcherRepository: "overnight", editor: true, changes: true, trouble: nil, needsYou: 11)

    /// The pieces at `window`, most to least important: need you, the
    /// status line, the other counters, Activity.
    private static func shown(_ model: TitleStatus.Model, window: CGFloat) -> [Bool] {
        let form = room.form(window: window)
        let layout = TitleStatus.layout(model, form: form)
        return [model.needYou > 0, layout.status, layout.counters, layout.activity]
    }

    @Test("At the minimum, 900 and 1400, every counter fits its room, whatever its number",
          arguments: [MainWindowChrome.minimumWidth, 900, 1400], [0, 7, 99, 1234])
    func countersFit(window: CGFloat, needYou: Int) {
        let model = Self.model(needYou: needYou, doing: "Delegating ov-468 and then reading the whole of the integration report")
        let form = Self.room.form(window: window)
        let layout = TitleStatus.layout(model, form: form)
        var pieces: [CGFloat] = []
        if needYou > 0 { pieces.append(TitleStatus.counterWidth(needYou)) }
        if layout.counters { pieces += TitleStatus.counterCounts(model).map(TitleStatus.counterWidth) }
        if layout.activity { pieces.append(TitleStatus.activityWidth) }
        let used = pieces.reduce(0, +) + TitleStatus.gap * CGFloat(pieces.count) + (layout.status ? TitleStatus.gap : 0)
        #expect(used + layout.statusWidth <= form.width, "at \(window): \(used) and a \(layout.statusWidth) status line in \(form.width)")
        if layout.status { #expect(layout.statusWidth >= TitleStatus.minimumStatusWidth, "at \(window)") }
        // The widest a counter draws is the real text at the real font.
        for count in [needYou, 1, 99] where count > 0 {
            #expect(TitleStatus.counterWidth(count) >= 16 + 4 + TitleStatus.textWidth(TitleStatus.number(count)))
        }
    }

    @Test("Narrowing drops Activity, then the other counters, then the status line; need you stays")
    func priorityOrder() {
        let model = Self.model(needYou: 4, doing: "Delegating ov-468")
        var previous = Self.shown(model, window: 2400)
        #expect(previous == [true, true, true, true], "everything fits at the widest")
        var window: CGFloat = 2400
        var sawEach = Set<[Bool]>([previous])
        while window > MainWindowChrome.minimumWidth {
            window -= 20
            let now = Self.shown(model, window: window)
            // Each piece present now was present at every wider width, and
            // the order of loss is the priority's: a lower piece is never
            // kept once a higher one is gone.
            for (a, b) in zip(previous, now) { #expect(!(b && !a), "at \(window): \(now) after \(previous)") }
            for i in 1..<now.count { #expect(!(now[i] && !now[i - 1]), "at \(window): \(now)") }
            #expect(now[0], "the need-you count is always there (\(window))")
            sawEach.insert(now)
            previous = now
        }
        #expect(sawEach.count == 4, "every step of the order shows somewhere: \(sawEach)")
        #expect(Self.shown(model, window: MainWindowChrome.minimumWidth) == [true, false, false, false])
        #expect(Self.shown(model, window: 900) == [true, true, true, false])
        #expect(Self.shown(model, window: 1400) == [true, true, true, true])
    }

    @Test("A crowded form drops from the bottom before it cuts anything")
    func crowdedFormDrops() {
        // A form wide enough for its tier only with short numbers: three-digit
        // counts push the status line out before they push a counter.
        var model = Self.model(needYou: 999, doing: "Delegating ov-468")
        model.orchestrator = .working
        let wide = TitleStatus.layout(model, form: .wide)
        let narrow = TitleStatus.layout(model, form: .medium)
        #expect(wide.statusWidth >= narrow.statusWidth || !wide.status)
        let none = TitleStatus.Model(orchestrator: nil, status: nil, nowDoing: nil, needYou: 3, running: [], inReview: [])
        #expect(!TitleStatus.layout(none, form: .wide).status, "no orchestrator, no status line")
    }
}
