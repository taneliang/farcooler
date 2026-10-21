import Foundation

/// Keeps the reader's place while older rows load above the conversation
/// (ov-629).
///
/// A lazy stack has no say in where the rows it already shows end up when
/// others are put above them: the reader's row went down by the whole height
/// of what came in (the first row, 164 pt from the top, went to 1017). The
/// content's size changes in several steps under a lazy stack (rows are
/// estimated, then measured as they come into view), so the height added
/// can't be known when the rows arrive; and the scroll position following a
/// row at the foot left the row off by 18 to 26 pt (ov-626).
///
/// This watches the rows instead. Each row on the screen reports where it is
/// in the scroll view, and the last place of each is kept. When rows come in
/// above the oldest one the list had, the first row to report a new place
/// becomes the reference: how far it moved is how far the reader's view
/// moved, and the list is scrolled by that. The row reports again as the
/// scroll lands and as rows above it are measured, each time put right the
/// same way, until it has been still for a while or the reader's finger takes
/// over.
@MainActor
final class PlaceHold {
    /// The scroll view's content offset, from the top of the content (not of
    /// its inset, which `scrollTo(y:)` counts from), kept by the view as it
    /// scrolls.
    var offset: CGFloat = 0
    /// The height of the scroll view's viewport, kept by the view: a row
    /// outside it isn't held (it may leave the list before it reports again).
    var viewport: CGFloat = 0
    /// Where each row on the screen last was.
    private var places: [String: CGFloat] = [:]
    /// The oldest row the list had when the rows last stood still.
    private var oldest: String?
    /// The row held, and where it was before the older rows came.
    private var reference: (id: String, y: CGFloat)?
    /// Where the last correction asked the scroll view to be, until it
    /// gets there.
    private var asked: CGFloat?
    /// Where a report asked the scroll view to be while the last correction
    /// was still on its way, and so was not acted on: done once that lands.
    private var deferred: CGFloat?
    private var quiet: Task<Void, Never>?

    var isHolding: Bool { reference != nil }

    /// What to do to the scroll view.
    enum Move: Equatable {
        /// Go to this offset (`scrollTo(y:)`).
        case offset(CGFloat)
        /// Bring this row into the viewport (`scrollTo(id:)`), which makes the
        /// list build it: a page of rows pushes the row held so far down that
        /// the list's estimates of what is between are too rough to scroll by.
        case row(String)
    }
    /// The row has been brought into view once already.
    private var broughtIn = false

    #if DEBUG
    /// For the UI tests: the held row's place before the older rows came and
    /// at its latest report, and its id, `164>164@prose:live0`; "none" before
    /// any row was held.
    private(set) var probe = "none"
    /// The scroll view's top inset: what a row's place in the scroll view
    /// lacks of its place in the window.
    var inset: CGFloat = 0
    /// The steps taken, newest last, for finding out why a place was lost.
    private(set) var trace: [String] = []
    private func note(_ step: String) { trace.append(step); if trace.count > 14 { trace.removeFirst() } }
    #endif

    /// `id` is at `y` in the scroll view. `oldest` is the list's oldest *row*
    /// now (not its oldest item: a run of tool calls is one item whose id is
    /// its first row's and changes as older calls join it) and `kept`
    /// whether the row that was the oldest is still in the list;
    /// `following` is a list on its tail, which follows its own end, and
    /// `driving` a reader's finger on it. Returns the offset to scroll to, or
    /// nil where the place is right.
    func report(
        _ id: String, y: CGFloat, oldest now: String?, kept: (String) -> Bool, following: Bool, driving: Bool
    ) -> Move? {
        let before = places[id]
        places[id] = y
        if driving || following {
            release(oldest: now)
            return nil
        }
        if let reference {
            guard id == reference.id else { return nil }
            #if DEBUG
            probe = "\(Int(reference.y.rounded()))>\(Int(y.rounded()))@\(id)"
            #endif
            return correct(y - reference.y, row: id)
        }
        if let known = oldest, known != now, kept(known) {
            // Rows came in above: a row that was already on the screen is
            // the one to hold. Rows that are new report first, and wait.
            // One in view: rows far above or below are let out of a lazy list
            // as it grows, and are gone before they report again.
            guard let before, viewport == 0 || (before >= -viewport * 0.1 && before <= viewport) else { return nil }
            reference = (id, before)
            #if DEBUG
            probe = "\(Int(before.rounded()))>\(Int(y.rounded()))@\(id)"
            #endif
            return correct(y - before, row: id)
        }
        oldest = now
        return nil
    }

    /// The row left the screen: its place is no longer known. The row held
    /// is kept: pushed down by a whole page, it is let go by the list at once,
    /// and comes back, to report again, once the scroll has taken the reader
    /// to where it will be.
    func gone(_ id: String) {
        places[id] = nil
        #if DEBUG
        if reference?.id == id { note("G") }
        #endif
    }

    private func correct(_ error: CGFloat, row id: String) -> Move? {
        if viewport > 0, abs(error) > viewport, !broughtIn, asked == nil {
            broughtIn = true
            deferred = nil
            #if DEBUG
            note("R\(Int(error))")
            #endif
            return .row(id)
        }
        let target = max(offset + error, 0)
        if let asked {
            // The last correction hasn't landed: this place is of a list in
            // motion. Kept, for `landed` to act on if nothing newer comes.
            guard abs(offset - asked) <= 1 else {
                deferred = abs(error) > 0.5 ? target : nil
                return nil
            }
            self.asked = nil
        }
        deferred = nil
        guard abs(error) > 0.5 else { return nil }
        asked = target
        #if DEBUG
        note("K\(Int(error))")
        #endif
        return .offset(target)
    }

    /// The scroll view's offset is now `now`. Where the last correction has
    /// landed and a place was waiting behind it, the offset to scroll to.
    func landed(at now: CGFloat) -> CGFloat? {
        offset = now
        guard let asked, abs(now - asked) <= 1 else { return nil }
        #if DEBUG
        note("L")
        #endif
        self.asked = nil
        guard let target = deferred else { return nil }
        deferred = nil
        guard abs(target - now) > 0.5 else { return nil }
        self.asked = target
        return target
    }

    /// Calls `done` once nothing has moved for `seconds`; each call starts
    /// the wait again.
    func settle(after seconds: Double, _ done: @escaping @MainActor () -> Void) {
        quiet?.cancel()
        quiet = Task { @MainActor in
            try? await Task.sleep(for: .seconds(seconds))
            guard !Task.isCancelled else { return }
            done()
        }
    }

    /// The held row is let go, and `now` is the list's oldest row.
    func release(oldest now: String? = nil) {
        quiet?.cancel()
        quiet = nil
        reference = nil
        broughtIn = false
        asked = nil
        deferred = nil
        oldest = now
    }
}
