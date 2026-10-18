import Foundation
import Testing
import os

@testable import AgentKit

/// A clock that moves only when a test moves it. Sleepers wake when `advance`
/// passes their deadline.
final class SteppedClock: Clock, @unchecked Sendable {
    struct Instant: InstantProtocol {
        var offset: Duration
        func advanced(by duration: Duration) -> Instant { Instant(offset: offset + duration) }
        func duration(to other: Instant) -> Duration { other.offset - offset }
        static func < (a: Instant, b: Instant) -> Bool { a.offset < b.offset }
    }

    private struct Sleeper {
        let id: UUID
        let deadline: Instant
        let wake: CheckedContinuation<Void, Error>
    }

    private let state = OSAllocatedUnfairLock<(now: Instant, sleepers: [Sleeper])>(
        initialState: (Instant(offset: .zero), []))

    var now: Instant { state.withLock { $0.now } }
    var minimumResolution: Duration { .zero }
    var sleeping: Int { state.withLock { $0.sleepers.count } }

    func sleep(until deadline: Instant, tolerance: Duration?) async throws {
        let id = UUID()
        try await withTaskCancellationHandler {
            try await withCheckedThrowingContinuation { (wake: CheckedContinuation<Void, Error>) in
                let due = state.withLock { s -> Bool in
                    if deadline <= s.now { return true }
                    s.sleepers.append(Sleeper(id: id, deadline: deadline, wake: wake))
                    return false
                }
                if due { wake.resume() }
            }
        } onCancel: {
            let gone = state.withLock { s -> Sleeper? in
                guard let at = s.sleepers.firstIndex(where: { $0.id == id }) else { return nil }
                return s.sleepers.remove(at: at)
            }
            gone?.wake.resume(throwing: CancellationError())
        }
    }

    func advance(by duration: Duration) {
        let due = state.withLock { s -> [Sleeper] in
            s.now = s.now.advanced(by: duration)
            let now = s.now
            let due = s.sleepers.filter { $0.deadline <= now }
            s.sleepers.removeAll { $0.deadline <= now }
            return due
        }
        for sleeper in due { sleeper.wake.resume() }
    }
}

/// A core that records what was asked, and answers only when told to.
final class HeldCore: @unchecked Sendable {
    private let state = OSAllocatedUnfairLock<(asked: [String], held: [String: [CheckedContinuation<Void, Never>]])>(
        initialState: ([], [:]))

    var asked: [String] { state.withLock { $0.asked } }

    /// Ask `method`, and wait until it is released.
    func ask(_ method: String) async {
        await withCheckedContinuation { (answer: CheckedContinuation<Void, Never>) in
            state.withLock {
                $0.asked.append(method)
                $0.held[method, default: []].append(answer)
            }
        }
    }

    func release(_ method: String) {
        let answers = state.withLock { $0.held.removeValue(forKey: method) ?? [] }
        for answer in answers { answer.resume() }
    }
}

/// What the connection under test did, in order, on the main actor.
@MainActor
final class Moves {
    var dials: [LinkRevival.Outcome] = []
    var refreshes = 0
}

/// Wait, in real time, until `condition` holds. A bound for a hang, not a
/// measurement: a check that never comes true fails at the bound.
@MainActor
func eventuallyRevived(_ what: String, _ condition: () -> Bool) async {
    let deadline = Date().addingTimeInterval(30)
    while !condition() {
        if Date() > deadline {
            Issue.record("never: \(what)")
            return
        }
        try? await Task.sleep(for: .milliseconds(2))
    }
}

@MainActor
private func returning(
    _ standing: LinkRevival.Standing, clock: SteppedClock, core: HeldCore, moves: Moves,
    probe: @escaping @Sendable () async -> LinkRevival.Probe
) -> Task<LinkRevival.Outcome, Never> {
    Task { @MainActor in
        await LinkRevival.onReturn(
            standing, clock: clock, probe: probe,
            dialNow: { moves.dials.append($0) },
            refresh: {
                moves.refreshes += 1
                await core.ask("fleet")
            })
    }
}

/// **A probe nobody answers redials at the probe's deadline**, 2.5 s, not
/// at the fleet read's 30 s and not at the keepalive's two minutes. Before
/// the deadline its sleeper is still parked on the clock, which no number of
/// hops can make true early; a deadline set too long fails on the time limit
/// rather than hanging the run.
@MainActor
@Test(.timeLimit(.minutes(1))) func aProbeTimingOutReconnectsAtTheProbeDeadline() async {
    let clock = SteppedClock()
    let core = HeldCore()
    let moves = Moves()
    let back = returning(.connected, clock: clock, core: core, moves: moves) {
        await core.ask("probe")
        return .answered
    }
    await eventuallyRevived("the probe's deadline is armed") { clock.sleeping == 1 }
    clock.advance(by: .milliseconds(2400))
    #expect(clock.sleeping == 1, "the deadline fired before 2.5 s")
    #expect(moves.dials.isEmpty, "redialed before the deadline")
    clock.advance(by: .milliseconds(100))
    #expect(clock.sleeping == 0, "the deadline did not fire at 2.5 s")
    // Bounded: a deadline past 2.5 s fails here instead of hanging on `back`.
    await eventuallyRevived("redialed at the deadline") { !moves.dials.isEmpty }
    #expect(moves.dials == [.redialed(.silent)])
    await eventuallyRevived("the fleet read went out") { core.asked.contains("fleet") }
    core.release("probe")
    core.release("fleet")
    // Released either way, so a wrong deadline still lets the return end.
    clock.advance(by: .seconds(60))
    #expect(await back.value == .redialed(.silent))
}

/// **A drop the probe finds dials at once**, before anything sleeps.
@MainActor
@Test(.timeLimit(.minutes(1))) func aFoundDisconnectDialsAtOnce() async {
    let clock = SteppedClock()
    let core = HeldCore()
    let moves = Moves()
    let back = returning(.connected, clock: clock, core: core, moves: moves) { .dropped }
    #expect(await back.value == .redialed(.dropped))
    #expect(moves.dials == [.redialed(.dropped)])
    await eventuallyRevived("the fleet read went out") { core.asked.contains("fleet") }
    core.release("fleet")
}

/// **A drop the fleet read finds while the app is coming back dials at
/// once**; any other waits out the backoff's first rung. This is the decision
/// both `linkDropped`s make, where the 1.6 to 2.4 s wait used to be.
@Test func aDropFoundOnReturnSkipsTheBackoff() {
    #expect(LinkRevival.afterDrop(returning: true) == .dialNow)
    #expect(LinkRevival.afterDrop(returning: false) == .backoff(attempt: 1))
}

/// **A healthy link costs one probe**, alongside the read it would have made
/// anyway, and no reconnect.
@MainActor
@Test(.timeLimit(.minutes(1))) func aHealthyLinkCostsOneProbe() async {
    let clock = SteppedClock()
    let core = HeldCore()
    let moves = Moves()
    let back = returning(.connected, clock: clock, core: core, moves: moves) {
        await core.ask("probe")
        return .answered
    }
    // Both out before either is answered: the fleet does not wait on the probe.
    await eventuallyRevived("the probe and the fleet read both go out") { core.asked.count == 2 }
    core.release("probe")
    #expect(await back.value == .alive)
    core.release("fleet")
    #expect(core.asked.sorted() == ["fleet", "probe"])
    #expect(moves.dials.isEmpty)
    #expect(moves.refreshes == 1)
}

/// A dial from before the app went away, past the deadline, is started
/// again. One begun since the app came back is left however long it has
/// taken, so a slow first connect is never churned by someone switching apps;
/// so is a young one. Retrying dials at once; a question for a person is left.
@MainActor
@Test(.timeLimit(.minutes(1))) func aStuckDialIsRestartedAndAFreshOneLeft() async {
    let clock = SteppedClock()
    let core = HeldCore()
    for (standing, outcome) in [
        (LinkRevival.Standing.dialing(for: .seconds(40), beganBeforeLeaving: true), LinkRevival.Outcome.restartedDial),
        (.dialing(for: .seconds(6), beganBeforeLeaving: false), .left),
        (.dialing(for: .milliseconds(300), beganBeforeLeaving: true), .left),
        (.retrying, .dialed),
        (.waitingOnPerson, .left),
    ] {
        let moves = Moves()
        let back = returning(standing, clock: clock, core: core, moves: moves) { .answered }
        #expect(await back.value == outcome, "\(standing)")
        #expect(moves.dials == (outcome == .left ? [] : [outcome]), "\(standing)")
        #expect(moves.refreshes == 0, "\(standing)")
    }
    #expect(core.asked.isEmpty)
}

/// **A new link's reads go out together**: every one has started before the
/// first is answered.
@MainActor
@Test(.timeLimit(.minutes(1))) func theReadsStartConcurrently() async {
    let core = HeldCore()
    let reads = ["fleet", "host", "repositories", "themes"]
    let bringUp = Task { @MainActor in
        await LinkRevival.together(reads.map { name in { await core.ask(name) } })
    }
    await eventuallyRevived("all four reads are out") { core.asked.count == reads.count }
    #expect(Set(core.asked) == Set(reads))
    for name in reads {
        await eventuallyRevived("\(name) went out") { core.asked.contains(name) }
        core.release(name)
    }
    await bringUp.value
}

/// **The boards are swept only once every read is in.** A sweep builds its
/// list from the build, the fleet and the repositories; one that ran beside
/// them, before they landed, swept an empty list on a cold start and read no
/// board at all.
@MainActor
@Test(.timeLimit(.minutes(1))) func theBoardsWaitForTheRepositories() async {
    let core = HeldCore()
    let moves = Moves()
    let bringUp = Task { @MainActor in
        await LinkRevival.bringUp(
            ["fleet", "repositories"].map { name in { await core.ask(name) } },
            afterwards: { moves.refreshes += 1 })
    }
    await eventuallyRevived("both reads are out") { core.asked.count == 2 }
    core.release("fleet")
    for _ in 0..<50 { await Task.yield() }
    #expect(moves.refreshes == 0, "the boards were swept before the repositories were in")
    core.release("repositories")
    await bringUp.value
    #expect(moves.refreshes == 1)
}
