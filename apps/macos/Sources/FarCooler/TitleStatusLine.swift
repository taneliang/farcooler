import AppKit
import Foundation

// The orchestrator's label and the room it has (ov-329). The status area's
// width is fixed per form (`TitleStatus.Form.width`), and a borderless menu
// won't shrink its label inside it, so the line is cut to fit by measure,
// here, rather than left to the layout: its tail gives way, with an ellipsis,
// and the other pieces never move.

extension TitleStatus {
    /// What the area's other pieces take in `model` at `form`, each with the
    /// gap before it: the status mark and the orchestrator's symbol and caret
    /// before the words, then the activity button where it's drawn, and each
    /// count that's showing. Estimates, a little over, like `leading`'s.
    static func reserved(_ model: Model, form: Form) -> CGFloat {
        let gap: CGFloat = form >= .medium ? 14 : 8
        // The mark, the symbol, their gaps, and the caret.
        var width: CGFloat = 24 + 6 + 16 + 6 + 16
        if showsActivityButton(model, form: form) { width += gap + textWidth("Activity") + 6 + textWidth("⌘K") }
        func count(_ words: String?, _ n: Int) -> CGFloat {
            gap + 16 + 4 + textWidth(form == .wide ? (words ?? "") : number(n))
        }
        if model.needYou > 0 { width += count(needYouWords(model.needYou), model.needYou) }
        if !model.failed.isEmpty { width += count(failedWords(model.failed.count), model.failed.count) }
        if !model.running.isEmpty || model.queued > 0 { width += count(runningWords(model.running.count), model.running.count) }
        if !model.inReview.isEmpty { width += count(inReviewWords(model.inReview.count), model.inReview.count) }
        return width
    }

    /// Whether the "Activity ⌘K" button is drawn beside the orchestrator: at
    /// the wide form, and at medium only while the orchestrator has no line
    /// of its own to give the room to.
    static func showsActivityButton(_ model: Model, form: Form) -> Bool {
        form >= .wide || (form == .medium && model.nowDoing == nil)
    }

    /// The orchestrator's words at `form`, cut to the room it has.
    static func fittedWords(_ model: Model, form: Form) -> String {
        guard let state = model.orchestrator else { return "" }
        guard form >= .medium else { return OrchestratorRow.word(state) }
        let room = form.width - reserved(model, form: form)
        let full = orchestratorWords(state, doing: model.nowDoing)
        let base = orchestratorWords(state)
        // Never less than the state, which is the status itself.
        return fit(full, room: max(room, textWidth(base)), keeping: base)
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
