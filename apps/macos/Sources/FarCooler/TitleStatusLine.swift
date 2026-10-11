import AppKit
import Foundation

// What the status area shows at a form, and in what order it gives things up
// (ov-469). The area's width is fixed per form (`TitleStatus.Form.width`), so
// the layout is worked out here by measure, and nothing is ever left to the
// toolbar's layout to squeeze: a counter is an icon and a number at their own
// width and is drawn whole or not at all, and the status line is the one
// piece that gives way, cut at its tail.
//
// Priority, last to go first: the need-you count, the status line, the other
// counters (failed, running, in review), then Activity. A form is a tier of
// that list; a form too narrow for its tier (a three-digit count, say) drops
// from the bottom until what's left fits.

extension TitleStatus {
    /// The space between the area's pieces.
    static let gap: CGFloat = 10
    /// The Activity icon's button.
    static let activityWidth: CGFloat = 24
    /// The least the status line is worth showing at: under it, it goes.
    static let minimumStatusWidth: CGFloat = 96
    /// What a borderless menu adds around its label, which the line's cut
    /// leaves room for.
    static let menuPadding: CGFloat = 10

    /// What the area draws at one form.
    struct Layout: Equatable {
        var status: Bool
        var statusWidth: CGFloat
        /// The failed, running and in-review counters, which go together.
        var counters: Bool
        var activity: Bool
    }

    /// An icon and a number, at their own width, with a little over for the
    /// toolbar button's padding: "99+" is the widest it gets.
    static func counterWidth(_ count: Int) -> CGFloat { 16 + 4 + textWidth(number(count)) + 4 }

    /// The status line: what the orchestrator is doing, or its state's word
    /// with nothing to say. No name and no icon (ov-469). Nil with no
    /// conversation column to have one.
    static func statusWords(_ model: Model) -> String? {
        guard let state = model.orchestrator else { return nil }
        if state == .none { return OrchestratorRow.word(.none) }
        return model.nowDoing ?? OrchestratorRow.word(state)
    }

    /// The tooltip over the status line, all of it: "Working: Delegating ov-468".
    static func statusHelp(_ model: Model) -> String {
        guard let state = model.orchestrator, state != .none else { return statusWords(model) ?? "" }
        let word = OrchestratorRow.word(state)
        return model.nowDoing.map { "\(word): \($0)" } ?? word
    }

    /// The counters showing, in order, with their counts.
    static func counterCounts(_ model: Model) -> [Int] {
        var counts: [Int] = []
        if !model.failed.isEmpty { counts.append(model.failed.count) }
        if !model.running.isEmpty || model.queued > 0 { counts.append(model.running.count) }
        if !model.inReview.isEmpty { counts.append(model.inReview.count) }
        return counts
    }

    /// What `form` shows of `model`, after dropping what its width can't hold.
    static func layout(_ model: Model, form: Form) -> Layout {
        var layout = Layout(
            status: form >= .short && statusWords(model) != nil, statusWidth: 0, counters: form >= .medium,
            activity: form >= .wide)
        let needYou = model.needYou > 0 ? [counterWidth(model.needYou)] : []
        let others = counterCounts(model).map(counterWidth)
        while true {
            var pieces = needYou
            if layout.counters { pieces += others }
            if layout.activity { pieces.append(activityWidth) }
            let used = pieces.reduce(0, +) + gap * CGFloat(pieces.count)
            let room = form.width - used - (layout.status ? gap : 0)
            if !layout.status || room >= minimumStatusWidth {
                layout.statusWidth = layout.status ? room : 0
                return layout
            }
            if layout.activity {
                layout.activity = false
            } else if layout.counters, !others.isEmpty {
                layout.counters = false
            } else {
                layout.status = false
            }
        }
    }

    /// `text` cut at its tail with an ellipsis until it's `room` wide, never
    /// shorter than `keeping`.
    static func fit(_ text: String, room: CGFloat, keeping base: String = "") -> String {
        guard textWidth(text) > room else { return text }
        var characters = Array(text)
        while characters.count > base.count + 1, textWidth(String(characters) + "…") > room {
            characters.removeLast()
        }
        let cut = String(characters).trimmingCharacters(in: .whitespaces)
        return cut.count <= base.count ? base : cut + "…"
    }
}
