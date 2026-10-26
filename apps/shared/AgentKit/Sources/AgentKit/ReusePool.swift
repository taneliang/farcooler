import Foundation

/// Objects no one is using, kept for the next to take (ov-788): a
/// conversation's text views, costly to make, given back by the rows a
/// fling takes away and taken by the rows it brings in.
///
/// `reset` runs as one is kept, so a kept object holds nothing of what it
/// served: no text, no selection, no delegate. The pool keeps at most
/// `limit`, and `drain` empties it, on a memory warning.
@MainActor
public final class ReusePool<Object: AnyObject> {
    private var kept: [Object] = []
    private let limit: Int
    private let reset: (Object) -> Void

    public init(limit: Int, reset: @escaping (Object) -> Void) {
        self.limit = max(0, limit)
        self.reset = reset
    }

    /// How many it holds now.
    public var count: Int { kept.count }

    /// Keep `object` for the next to take, unless the pool is full.
    public func keep(_ object: Object) {
        guard kept.count < limit, !kept.contains(where: { $0 === object }) else { return }
        reset(object)
        kept.append(object)
    }

    /// One kept, or nil when there's none.
    public func take() -> Object? { kept.popLast() }

    /// Let every kept one go.
    public func drain() { kept.removeAll() }
}
