import Foundation

/// A bounded memo for work a SwiftUI `body` would otherwise redo every time it
/// runs.
///
/// A transcript row's body is not evaluated once. It is evaluated when the row
/// is realized while scrolling, again when it leaves and re-enters the lazy
/// container, and — for the row at the tail of a streaming turn — on every
/// poll, because chunks COALESCE into the last row rather than appending a new
/// one (`Transcript.appendMessage`). So the last row of a live turn re-runs its
/// body several times a second, and every row a reader scrolls past re-runs
/// its own.
///
/// What those bodies were doing is not cheap. `MarkdownText` re-parsed the
/// whole message — a line scan plus an `AttributedString(markdown:)` per
/// paragraph — and `DiffView` recomputed an LCS of up to 160,000 cells, which
/// it does even while COLLAPSED because the body needs the line count for its
/// header and its "Show N lines" label. Both ran on the main thread, inside
/// the scroll.
///
/// The Android port already fixed this and the two Apple surfaces never did:
/// `ui/Markdown.kt` wraps the same parse in `remember(text)` and
/// `ui/AgentRows.kt` wraps the same diff in `remember(diff)`. Compose has
/// `remember` per composition; SwiftUI's equivalents do not fit here — `@State`
/// cannot be seeded from the value that determines it without an extra render,
/// and `body` is not allowed to write view state at all. A memo outside the
/// view is the version that works for both.
///
/// Bounded because a transcript is not: `Transcript.rows` has no cap, so a memo
/// keyed on message text would otherwise hold every message of a long session
/// plus every intermediate prefix the tail row streamed through. Eviction is
/// least-recently-used, which is the right shape for a scroll — what a reader
/// is looking at now is what they are about to look at again.
///
/// `@MainActor` rather than locked: every caller is a SwiftUI `body`, which is
/// main-actor isolated, and an actor-isolated dictionary needs no lock at all.
/// That also keeps `misses` honest enough to assert on, which is what
/// `RenderMemoTests` and `MarkdownCacheTests` do.
@MainActor
public final class RenderMemo<Key: Hashable, Value> {
    private var entries: [Key: Value] = [:]
    /// Least-recently-used first. A plain array because `limit` is in the
    /// dozens: a linear remove of a 60-element array is cheaper than the
    /// bookkeeping a linked list would need, and this runs once per hit.
    private var recency: [Key] = []
    private let limit: Int

    /// How many times a value had to be computed rather than found.
    ///
    /// Public so a test can prove the cache is actually reached from the view
    /// that is supposed to reach it. A cache is exactly the kind of change that
    /// keeps working after it stops being used, so the guard has to assert on
    /// the miss and not merely on the answer.
    public private(set) var misses = 0

    public init(limit: Int) {
        self.limit = max(1, limit)
    }

    /// The stored value for `key`, computing and storing it on a miss.
    public func value(for key: Key, compute: (Key) -> Value) -> Value {
        if let hit = entries[key] {
            touch(key)
            return hit
        }
        misses += 1
        let made = compute(key)
        entries[key] = made
        recency.append(key)
        while recency.count > limit, let oldest = recency.first {
            recency.removeFirst()
            entries.removeValue(forKey: oldest)
        }
        return made
    }

    private func touch(_ key: Key) {
        guard let at = recency.firstIndex(of: key) else { return }
        recency.remove(at: at)
        recency.append(key)
    }

    /// Empties it. For tests; nothing in the app needs to forget.
    public func removeAll() {
        entries.removeAll()
        recency.removeAll()
        misses = 0
    }
}
