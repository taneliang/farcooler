import Foundation
import Observation

/// Where a terminal's agent rows come from: `agent.rows` and
/// `agent.rows_follow` on some client, answering the client core's JSON
/// (`crates/client/src/ffi/rows_args.rs`) undecoded, so decoding happens in
/// `AgentRowLedger` and never on the caller's thread.
public protocol AgentRowSource: Sendable {
    /// The newest rows, or up to `limit` before `before`.
    func page(before: UInt64?, limit: Int) async throws -> Data
    /// What changed after `afterRev`, the runner holding the call up to
    /// `waitMs` for something to.
    func follow(epoch: UInt64, afterRev: UInt64, waitMs: Int) async throws -> Data
}

/// A source's failure that retrying won't fix: the runner doesn't serve rows
/// (no `agent_rows`, the projector flag off), or the pane is gone.
public struct AgentRowsUnavailable: Error, Equatable {
    public init() {}
}

/// One row on screen. Its own observable object, so a change to one row
/// re-renders that row's view and nothing else: the list observes `ids`, and
/// a row view observes only its box (ov-371).
@MainActor
@Observable
public final class AgentRowBox: Identifiable {
    public let id: String
    public internal(set) var row: AgentRow

    init(_ row: AgentRow) {
        id = row.id
        self.row = row
    }
}

/// A terminal's rows for a view, on the main actor, fed from off it (ov-371).
///
/// The main thread's whole share of an update is `apply`: set each changed
/// row into its box, and swap in an order the ledger already built when the
/// set of rows changed. Decoding, merging by id and ordering all happen in
/// `AgentRowLedger`; the loop that pages, follows and re-pages runs on the
/// cooperative pool (`run`).
///
/// A store made for a pane this launch already showed draws the cached rows in
/// its initializer, so the first frame doesn't wait on anything; one opened
/// after a relaunch reads them from disk off the main thread.
@MainActor
@Observable
public final class AgentRowStore {
    public enum Phase: Equatable, Sendable {
        /// Nothing to draw yet.
        case loading
        /// Drawn from the cache; the runner hasn't answered yet.
        case cached
        /// Following the runner.
        case live
        /// The runner doesn't serve rows for this pane.
        case unavailable
        /// The last call failed; it's being retried. The words are the
        /// source's.
        case trouble(String)
    }

    /// Every held row's id, oldest first. Changes only when a row arrives or
    /// goes, never when one's contents change.
    public private(set) var ids: [String] = []
    public private(set) var phase: Phase = .loading
    /// Whether older rows exist than the oldest held.
    public private(set) var moreBefore = false
    public private(set) var loadingOlder = false

    /// How many updates `apply` has taken, and the main-thread time they
    /// cost: the budget the tests hold this to.
    @ObservationIgnored public private(set) var applied = 0
    /// The newest updates' main-thread times, up to `timesKept` of them.
    @ObservationIgnored public private(set) var applyTimes: [Duration] = []
    nonisolated static let timesKept = 512

    @ObservationIgnored public let key: String
    @ObservationIgnored private var boxes: [String: AgentRowBox] = [:]
    @ObservationIgnored let ledger = AgentRowLedger()
    @ObservationIgnored private let cache: AgentRowCache?
    @ObservationIgnored private var seeded: AgentRowSnapshot?
    @ObservationIgnored private var feed: Task<Void, Never>?

    /// `key` names the pane (a runner and a terminal) in `cache`.
    public init(key: String, cache: AgentRowCache? = .shared) {
        self.key = key
        self.cache = cache
        if let snapshot = cache?.inMemory(key) {
            seeded = snapshot
            show(snapshot)
        }
    }

    public func box(_ id: String) -> AgentRowBox? { boxes[id] }

    public var isFollowing: Bool { feed != nil }

    /// Put `delta` on screen. The only main-thread work an update costs.
    public func apply(_ delta: AgentRowDelta) {
        let began = ContinuousClock.now
        for row in delta.rows {
            if let box = boxes[row.id] {
                box.row = row
            } else {
                boxes[row.id] = AgentRowBox(row)
            }
        }
        for id in delta.removed { boxes[id] = nil }
        if let order = delta.order { ids = order }
        if moreBefore != delta.moreBefore { moreBefore = delta.moreBefore }
        applied += 1
        if applyTimes.count == Self.timesKept { applyTimes.removeFirst() }
        applyTimes.append(ContinuousClock.now - began)
    }

    /// Start following `source`; a running follow is replaced.
    public func start(_ source: any AgentRowSource) {
        feed?.cancel()
        let seed = seeded
        seeded = nil
        let weakly = Weakly(self)
        feed = Task.detached(priority: .userInitiated) { [ledger, cache, key] in
            await Self.run(store: weakly, ledger: ledger, source: source, cache: cache, key: key, seed: seed)
        }
    }

    public func stop() {
        feed?.cancel()
        feed = nil
    }

    /// Ask for the page above the oldest held row, once at a time.
    public func loadOlder(_ source: any AgentRowSource) {
        guard moreBefore, !loadingOlder else { return }
        loadingOlder = true
        Task.detached(priority: .userInitiated) { [weak self, ledger] in
            let delta: AgentRowDelta?
            if let oldest = await ledger.oldestOrd, let data = try? await source.page(before: oldest, limit: AgentRowStore.pageSize) {
                delta = try? await ledger.older(data)
            } else {
                delta = nil
            }
            await MainActor.run {
                if let delta { self?.apply(delta) }
                self?.loadingOlder = false
            }
        }
    }

    /// The store, not kept alive by the loop that feeds it.
    final class Weakly: @unchecked Sendable {
        weak var store: AgentRowStore?
        init(_ store: AgentRowStore) { self.store = store }
    }

    func set(_ phase: Phase) {
        if self.phase != phase { self.phase = phase }
    }

    /// Draw a cached snapshot, before the ledger has it.
    private func show(_ snapshot: AgentRowSnapshot) {
        for row in snapshot.rows { boxes[row.id] = AgentRowBox(row) }
        ids = snapshot.rows.map(\.id)
        moreBefore = snapshot.moreBefore
        phase = snapshot.rows.isEmpty ? .loading : .cached
    }

    /// Rows a page asks for.
    nonisolated public static let pageSize = 100
    /// How long a follow is held when nothing changes.
    nonisolated public static let followWaitMs = 20_000

    /// The loop: take up the cache, then follow; page whenever the runner
    /// says it can't diff (a reset, a new epoch) and after any failure, since
    /// a call that failed may have lost changes nobody can name.
    nonisolated static func run(
        store weakly: Weakly, ledger: AgentRowLedger, source: any AgentRowSource, cache: AgentRowCache?,
        key: String, seed: AgentRowSnapshot?
    ) async {
        // Weak, so a pane that goes away ends its loop rather than being
        // kept alive by it.
        var store: AgentRowStore? { weakly.store }
        if let seed {
            _ = await ledger.restore(seed)
        } else if let disk = cache?.onDisk(key), !disk.rows.isEmpty {
            let delta = await ledger.restore(disk)
            await MainActor.run {
                store?.apply(delta)
                store?.set(.cached)
            }
        }
        var needsPage = await ledger.cursor == nil
        var waitMs = 0
        var backoff: Duration = .milliseconds(500)
        var saved = ContinuousClock.now
        while !Task.isCancelled {
            do {
                var delta: AgentRowDelta?
                if needsPage {
                    delta = try await ledger.page(source.page(before: nil, limit: pageSize))
                    needsPage = false
                } else if let cursor = await ledger.cursor {
                    switch try await ledger.follow(source.follow(epoch: cursor.epoch, afterRev: cursor.rev, waitMs: waitMs)) {
                    case .reset:
                        needsPage = true
                        continue
                    case .delta(let changed):
                        delta = changed.isEmpty ? nil : changed
                    }
                }
                waitMs = followWaitMs
                backoff = .milliseconds(500)
                guard !Task.isCancelled else { break }
                await MainActor.run {
                    if let delta { store?.apply(delta) }
                    store?.set(.live)
                }
                if let cache, ContinuousClock.now - saved > .seconds(1), delta != nil {
                    cache.keep(await ledger.snapshot(newest: AgentRowCache.rowsKept), for: key)
                    saved = .now
                }
            } catch is AgentRowsUnavailable {
                await MainActor.run { store?.set(.unavailable) }
                return
            } catch {
                guard !Task.isCancelled else { break }
                let said = (error as? LocalizedError)?.errorDescription ?? "\(error)"
                await MainActor.run { store?.set(.trouble(said)) }
                needsPage = true
                try? await Task.sleep(for: backoff)
                backoff = min(backoff * 2, .seconds(10))
            }
        }
        if let cache, await ledger.cursor != nil {
            cache.keep(await ledger.snapshot(newest: AgentRowCache.rowsKept), for: key)
        }
    }
}
