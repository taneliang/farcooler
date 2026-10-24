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
    /// The same calls for subagent `agentId`'s own rows (ov-453), where the
    /// runner serves them (`subagent_rows`); nil where it doesn't.
    func subagent(_ agentId: String) -> (any AgentRowSource)?
}

extension AgentRowSource {
    public func subagent(_ agentId: String) -> (any AgentRowSource)? { nil }
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
    public internal(set) var row: AgentRow {
        didSet {
            isTurn = Self.isTurn(row)
            let now = AgentTray.Activity(row)
            if now != trayActivity { trayActivity = now }
        }
    }
    /// Whether the row is a turn, read without observing the row (ov-691):
    /// what finds the newest turn reads this of every row after it, and
    /// observing those rows redrew whatever asked, the whole transcript
    /// and the composer, on every streamed word.
    @ObservationIgnored public private(set) var isTurn: Bool
    /// What the agents tray reads of the row (ov-693): a call running,
    /// thinking not yet ended, or neither. Observed apart from `row`, and set
    /// only when it changes, so the tray hears a call start or end and not
    /// each word streamed into a reply.
    public private(set) var trayActivity: AgentTray.Activity

    init(_ row: AgentRow) {
        id = row.id
        self.row = row
        isTurn = Self.isTurn(row)
        trayActivity = AgentTray.Activity(row)
    }

    private static func isTurn(_ row: AgentRow) -> Bool {
        if case .turn = row.kind { return true }
        return false
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
    /// What a transcript draws (ov-452): `shownIds` with each run of tool
    /// calls folded into one item. Built off the main thread with `ids`
    /// (`AgentRowDelta.items`), from each row's kind, which never changes,
    /// so no row is observed for it.
    public private(set) var items: [ConversationItem] = []
    /// Every `Subagent` row held, oldest first: what the agent tray reads
    /// (ov-453). Changes only when one arrives or goes.
    public private(set) var subagentIds: [String] = []
    public private(set) var phase: Phase = .loading
    /// Whether older rows exist than the oldest held.
    public private(set) var moreBefore = false
    public private(set) var loadingOlder = false

    /// How many updates `apply` has taken, and the main-thread time they
    /// cost: the budget the tests hold this to.
    @ObservationIgnored public private(set) var applied = 0
    /// The newest updates' main-thread times, up to `timesKept` of them.
    @ObservationIgnored public private(set) var applyTimes: [Duration] = []
    /// The ledger version the view matches (ov-612): every change up to it
    /// applied, in order, or a whole window taken at it.
    @ObservationIgnored public private(set) var shownVersion: UInt64 = 0
    /// How many whole windows `apply` has walked: each costs the main thread
    /// a pass over every held row, so they're taken only on a gap.
    @ObservationIgnored public private(set) var reconciled = 0
    nonisolated static let timesKept = 512

    @ObservationIgnored public let key: String
    @ObservationIgnored private var boxes: [String: AgentRowBox] = [:]
    @ObservationIgnored public let ledger = AgentRowLedger()
    @ObservationIgnored private let cache: AgentRowCache?
    @ObservationIgnored private var seeded: AgentRowSnapshot?
    @ObservationIgnored private var feed: Task<Void, Never>?
    /// The first wait after a failed call, doubled on each failure in a row.
    /// Tests set it near zero before `start`.
    @ObservationIgnored var retryDelay: Duration = .milliseconds(500)

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

    /// The rows held may be out of date: the last call failed, or the runner
    /// stopped serving them. A view says so over them rather than letting
    /// them look live.
    public var isStale: Bool {
        switch phase {
        case .trouble, .unavailable: true
        case .loading, .cached, .live: false
        }
    }

    public var isFollowing: Bool { feed != nil }

    /// Put `delta` on screen. The only main-thread work an update costs.
    public func apply(_ delta: AgentRowDelta) {
        let began = ContinuousClock.now
        if delta.complete {
            reconcile(delta)
            reconciled += 1
            shownVersion = delta.version
        } else {
            change(delta)
            if delta.base == shownVersion { shownVersion = delta.version }
        }
        applied += 1
        if applyTimes.count == Self.timesKept { applyTimes.removeFirst() }
        applyTimes.append(ContinuousClock.now - began)
    }

    private func change(_ delta: AgentRowDelta) {
        var agents: [String] = []
        for row in delta.rows {
            if let box = boxes[row.id] {
                box.row = row
            } else {
                boxes[row.id] = AgentRowBox(row)
                if case .subagent = row.kind { agents.append(row.id) }
            }
        }
        for id in delta.removed { boxes[id] = nil }
        if let order = delta.order { ids = order }
        if !agents.isEmpty || delta.removed.contains(where: subagentIds.contains) {
            // In the order they were launched, a page of older ones too.
            let held = Set(subagentIds + agents).filter { boxes[$0] != nil }
            subagentIds = ids.filter(held.contains)
        }
        if let items = delta.items { self.items = items }
        if moreBefore != delta.moreBefore { moreBefore = delta.moreBefore }
    }

    /// Put `delta` on screen if it follows on from what the view shows, and
    /// say whether the view needs the ledger's whole window instead (ov-612).
    /// A change already covered (a window taken after it) is dropped: applied
    /// late, a window would take back rows an older page added since, and an
    /// order would lose them.
    func take(_ delta: AgentRowDelta) -> Bool {
        if delta.version <= shownVersion { return true }
        // While a scroll is in progress an older page waits, and so does
        // every change after it, which goes on top of it: rows put above a
        // reader who is dragging or coasting would move what they are
        // looking at (ov-629). A whole window goes straight in.
        if scrollInProgress, !delta.complete, !held.isEmpty || delta.prepends {
            guard delta.base == (held.last?.version ?? shownVersion) else { return false }
            held.append(delta)
            #if DEBUG
            heldCount += 1
            #endif
            return true
        }
        guard delta.complete || delta.base == shownVersion else { return false }
        // A whole window has everything the changes that waited would add:
        // left waiting, each later change would queue behind them, or be
        // refused for not following on, and take the window again.
        if delta.complete { held = [] }
        apply(delta)
        return true
    }

    /// Whether a finger is scrolling the list, or it is still coasting (set
    /// by the view, ov-629). When it comes to rest, whatever waited goes in.
    @ObservationIgnored public var scrollInProgress = false {
        didSet {
            guard oldValue, !scrollInProgress else { return }
            watchdog?.cancel()
            watchdog = nil
            releaseHeld()
        }
    }

    /// How long a scroll may go without a sign of life before it is taken as
    /// over: a gesture cancelled by a call or a system sheet, or an app sent
    /// to the background mid-drag, never reaches rest, and the pane would wait
    /// on it for good.
    @ObservationIgnored public var scrollTimeout: Duration = .seconds(3)
    @ObservationIgnored private var watchdog: Task<Void, Never>?

    /// The list is scrolling (a phase change, or it moved): in progress, and
    /// for `scrollTimeout` more.
    public func scrolling() {
        scrollInProgress = true
        watchdog?.cancel()
        let timeout = scrollTimeout
        watchdog = Task { @MainActor [weak self] in
            try? await Task.sleep(for: timeout)
            guard !Task.isCancelled else { return }
            self?.scrollInProgress = false
        }
    }

    /// The scroll is over, or can't be known to be: the app left the front.
    public func scrollEnded() { scrollInProgress = false }
    #if DEBUG
    /// How many changes have waited for a scroll to rest, for the UI tests.
    @ObservationIgnored public private(set) var heldCount = 0
    #endif
    /// Changes that waited for the scroll to rest, in order.
    @ObservationIgnored private var held: [AgentRowDelta] = []

    private func releaseHeld() {
        let waiting = held
        held = []
        for delta in waiting where delta.version > shownVersion {
            // Each follows on from the last; a gap is for the next follow to
            // find and take the whole window.
            guard delta.base == shownVersion else { break }
            apply(delta)
        }
    }

    /// Make the view the ledger's whole window (ov-612): every row set,
    /// every box it no longer holds dropped. A row that didn't change keeps
    /// its box untouched, so only what drifted redraws.
    private func reconcile(_ delta: AgentRowDelta) {
        let order = delta.order ?? delta.rows.map(\.id)
        var agents: [String] = []
        var kept: [String: AgentRowBox] = [:]
        kept.reserveCapacity(delta.rows.count)
        for row in delta.rows {
            if let box = boxes[row.id] {
                if box.row != row { box.row = row }
                kept[row.id] = box
            } else {
                kept[row.id] = AgentRowBox(row)
            }
            if case .subagent = row.kind { agents.append(row.id) }
        }
        boxes = kept
        if ids != order { ids = order }
        if subagentIds != agents { subagentIds = agents }
        if let items = delta.items, self.items != items { self.items = items }
        if moreBefore != delta.moreBefore { moreBefore = delta.moreBefore }
    }

    /// Start following `source`; a running follow is replaced.
    /// The old loop's ledger writes are refused from here on (ov-612).
    public func start(_ source: any AgentRowSource) {
        feed?.cancel()
        let generation = ledger.advance()
        let seed = seeded
        seeded = nil
        let weakly = Weakly(self)
        let retryDelay = retryDelay
        feed = Task.detached(priority: .userInitiated) { [ledger, cache, key] in
            await Self.run(
                store: weakly, ledger: ledger, source: source, cache: cache, key: key, seed: seed, generation: generation,
                retryDelay: retryDelay)
        }
    }

    /// Stop following. A call already made can't be taken back, so its
    /// answer is refused by the ledger rather than applied (ov-612).
    public func stop() {
        feed?.cancel()
        feed = nil
        ledger.advance()
    }

    /// Stop, and wait for the loop to finish. A stopped loop still saves the
    /// rows to the cache as it ends, on its own task, so a caller that is about
    /// to remove the cache's folder waits here first, then flushes the cache
    /// (ov-735).
    public func stopAndSettle() async {
        let ending = feed
        stop()
        await ending?.value
    }

    /// Ask for the page above the oldest held row, once at a time.
    public func loadOlder(_ source: any AgentRowSource) {
        guard moreBefore, !loadingOlder else { return }
        loadingOlder = true
        let weakly = Weakly(self)
        Task.detached(priority: .userInitiated) { [ledger] in
            let delta: AgentRowDelta?
            if let oldest = await ledger.oldestOrd, let data = try? await source.page(before: oldest, limit: AgentRowStore.pageSize) {
                delta = try? await ledger.older(data)
            } else {
                delta = nil
            }
            await Self.deliver(delta, to: weakly, from: ledger)
            await MainActor.run { weakly.store?.loadingOlder = false }
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

    /// The ledger took up the seed this store drew when it was made: the
    /// view shows `restored`'s version without walking it again.
    func adopt(_ restored: AgentRowDelta) {
        if shownVersion == restored.base { shownVersion = restored.version }
    }

    /// Put `delta` on screen, or the ledger's whole window where the view has
    /// missed a change (ov-612). Nothing when `generation` has been replaced.
    /// `phase` is set either way.
    nonisolated static func deliver(
        _ delta: AgentRowDelta?, to weakly: Weakly, from ledger: AgentRowLedger, generation: UInt64? = nil, phase: Phase? = nil
    ) async {
        var next = delta
        while true {
            let current = next
            let gap = await MainActor.run { () -> Bool in
                guard generation.map(ledger.isCurrent) ?? true, let store = weakly.store else { return false }
                defer { if let phase { store.set(phase) } }
                guard let delta = current else { return false }
                return !store.take(delta)
            }
            guard gap else { return }
            next = await ledger.window()
        }
    }

    /// Draw a cached snapshot, before the ledger has it.
    private func show(_ snapshot: AgentRowSnapshot) {
        for row in snapshot.rows { boxes[row.id] = AgentRowBox(row) }
        ids = snapshot.rows.map(\.id)
        subagentIds = snapshot.rows.filter { if case .subagent = $0.kind { true } else { false } }.map(\.id)
        let kinds = Dictionary(snapshot.rows.map { ($0.id, $0) }, uniquingKeysWith: { $1 })
        items = AgentRowLedger.items(ids) { kinds[$0] }
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
    ///
    /// The view can't drift from the ledger (ov-612). Every change carries
    /// the ledger versions it goes from and to; a view that missed one takes
    /// the ledger's whole window instead (`deliver`), and a start checks the
    /// view against the ledger before anything else. Everything the loop
    /// writes is under `generation`: once a newer start or a stop has
    /// replaced it, the ledger refuses its writes and the view isn't touched.
    nonisolated static func run(
        store weakly: Weakly, ledger: AgentRowLedger, source: any AgentRowSource, cache: AgentRowCache?,
        key: String, seed: AgentRowSnapshot?, generation: UInt64, retryDelay: Duration = .milliseconds(500)
    ) async {
        // Weak, so a pane that goes away ends its loop rather than being
        // kept alive by it.
        /// Put `delta` on screen while this loop is still the newest.
        func show(_ delta: AgentRowDelta?, _ phase: Phase) async {
            await deliver(delta, to: weakly, from: ledger, generation: generation, phase: phase)
        }
        do {
            if let seed {
                // The view drew the seed when it was made: it shows what
                // the ledger now holds.
                let held = try await ledger.restore(seed, generation: generation)
                await MainActor.run { weakly.store?.adopt(held) }
            } else if await ledger.cursor == nil, let disk = cache?.onDisk(key), !disk.rows.isEmpty {
                await show(try await ledger.restore(disk, generation: generation), .cached)
            }
        } catch {
            return
        }
        // Whatever the view missed while no loop ran (a late answer the
        // ledger took just before the stop), it's given now.
        let version = await ledger.version
        let behind = await MainActor.run { ledger.isCurrent(generation) && weakly.store.map { $0.shownVersion != version } == true }
        if behind { await deliver(await ledger.window(), to: weakly, from: ledger, generation: generation) }
        var needsPage = await ledger.cursor == nil
        var waitMs = 0
        var backoff = retryDelay
        var saved = ContinuousClock.now
        while !Task.isCancelled {
            do {
                var delta: AgentRowDelta?
                if needsPage {
                    delta = try await ledger.page(source.page(before: nil, limit: pageSize), generation: generation)
                    needsPage = false
                } else if let cursor = await ledger.cursor {
                    let answer = try await source.follow(epoch: cursor.epoch, afterRev: cursor.rev, waitMs: waitMs)
                    switch try await ledger.follow(answer, generation: generation) {
                    case .reset:
                        needsPage = true
                        continue
                    case .delta(let changed):
                        delta = changed.isEmpty ? nil : changed
                    }
                }
                waitMs = followWaitMs
                backoff = retryDelay
                guard !Task.isCancelled else { break }
                await show(delta, .live)
                if let cache, ContinuousClock.now - saved > .seconds(1), delta != nil {
                    cache.keep(await ledger.snapshot(newest: AgentRowCache.rowsKept), for: key)
                    saved = .now
                }
            } catch is AgentRowsUnavailable {
                await show(nil, .unavailable)
                return
            } catch is AgentRowLedger.Stale {
                // Replaced: the newer loop keeps the cache.
                return
            } catch {
                guard !Task.isCancelled else { break }
                let said = (error as? LocalizedError)?.errorDescription ?? "\(error)"
                await show(nil, .trouble(said))
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
