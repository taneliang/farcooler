import Foundation
import os

/// What a phone does about its link to a runner when the app comes back to the
/// foreground, and how a new link is brought up (ov-613).
///
/// The person's complaint was over ten seconds to reconnect after switching
/// apps. Three things made it:
///
/// - A link that died silently while the app was away (a network change, a
///   carrier NAT that forgot the mapping, a runner that slept) looked alive. The
///   foreground check was an ordinary `fleet` read with a 30 s deadline whose
///   timeout was ignored, so only the ssh keepalive noticed, 90 to 120 s later.
///   Now a probe with a 2.5 s deadline decides, alongside the fleet read.
/// - A drop that check found waited out the first backoff rung, about 2 s,
///   before dialing. Now it dials at once; the backoff starts only when that
///   dial fails.
/// - A new link read the fleet, the build, the repositories and the themes one
///   after another. Now they go out together, and the boards are swept once
///   they are all in (`bringUp`).
///
/// Here rather than in `Connection.swift` because CI compiles that file and never
/// runs it: the decisions need somewhere a fake clock and a fake core reach.
/// Android's `LinkRevival.kt` keeps the same rules.
public enum LinkRevival {
    /// How long the foreground probe waits. `deadlines::PROBE` in
    /// `crates/client/src/deadlines.rs` is the same number, and empties the
    /// session slot when it passes; this one is the app's own backstop, so a
    /// core that never answers still ends in a dial.
    public static let probeDeadline: Duration = .milliseconds(2500)

    /// Where the link stood when the app came back.
    public enum Standing: Equatable, Sendable {
        case connected
        /// A dial in flight: how long it has been, and whether it began before
        /// the app went away. A first connect and a reconnect alike.
        case dialing(for: Duration, beganBeforeLeaving: Bool)
        /// Waiting out a backoff, or stopped on a failure, with no dial out.
        case retrying
        /// Waiting on a person: a host key to approve.
        case waitingOnPerson
    }

    /// What the probe found.
    public enum Probe: Equatable, Sendable {
        /// The runner spoke, even to refuse: the link is up.
        case answered
        /// Nothing by the deadline.
        case silent
        /// The core says the link is gone.
        case dropped

        public var linkIsDead: Bool { self != .answered }
    }

    /// What coming back did, for the log.
    public enum Outcome: Equatable, Sendable {
        /// The probe answered; the link stays.
        case alive
        /// The probe found the link dead, and a dial went out at once.
        case redialed(Probe)
        /// A dial from before the app went away, older than the probe's
        /// deadline, was started again.
        case restartedDial
        /// Retrying or stopped: a dial went out at once.
        case dialed
        /// Nothing to do: a dial begun since the app came back or still young,
        /// or a question for a person.
        case left
    }

    /// The app came back. Decide, probe at most once, and dial or refresh.
    ///
    /// From `connected`, the probe and the fleet read (`refresh`) go out
    /// together, so a healthy link paints in one round trip and pays for one
    /// small extra call; a dead one is dialed again at the probe's deadline
    /// at the latest. Returns once the verdict is in and acted on, which may be
    /// before `refresh` finishes; the read runs on.
    @MainActor
    public static func onReturn<C: Clock>(
        _ standing: Standing,
        clock: C,
        probe ask: @escaping @Sendable () async -> Probe,
        dialNow: @escaping @MainActor (Outcome) -> Void,
        refresh: @escaping @MainActor @Sendable () async -> Void
    ) async -> Outcome where C.Duration == Duration {
        switch standing {
        case .waitingOnPerson:
            return .left
        case .retrying:
            dialNow(.dialed)
            return .dialed
        case .dialing(let age, let beganBeforeLeaving):
            // Only a dial that set out before the app went away, and has had
            // the probe's deadline: it may be stuck on a route the suspension
            // broke, and a direct TCP dial has no timeout of its own but the
            // OS's minute. One begun since, however slow (a first connect over
            // DERP takes 3 to 8 s), is left to finish: restarting it on every
            // return would keep a person who keeps switching apps from ever
            // connecting.
            guard beganBeforeLeaving, age >= probeDeadline else { return .left }
            dialNow(.restartedDial)
            return .restartedDial
        case .connected:
            Task { @MainActor in await refresh() }
            let found = await probe(on: clock, ask)
            guard found.linkIsDead else { return .alive }
            dialNow(.redialed(found))
            return .redialed(found)
        }
    }

    /// Ask, and take silence by `deadline` on `clock` as the answer.
    ///
    /// The ask is not cancelled at the deadline, because a call into the core
    /// cannot be: it is a ticket the core answers in its own time. Its late
    /// answer is dropped.
    public static func probe<C: Clock>(
        within deadline: Duration = probeDeadline,
        on clock: C,
        _ ask: @escaping @Sendable () async -> Probe
    ) async -> Probe where C.Duration == Duration {
        let first = FirstAnswer()
        return await withCheckedContinuation { continuation in
            first.arm(continuation)
            let timer = Task {
                try? await clock.sleep(for: deadline)
                if !Task.isCancelled { first.give(.silent) }
            }
            Task {
                first.give(await ask())
                timer.cancel()
            }
        }
    }

    /// What a drop the fleet read found does: dial at once while the app is
    /// coming back (somebody is looking at a screen that says "connected"),
    /// otherwise wait out the backoff's first rung. Either way the next failure
    /// climbs from there.
    public enum AfterDrop: Equatable, Sendable {
        case dialNow
        case backoff(attempt: Int)
    }

    public static func afterDrop(returning: Bool) -> AfterDrop {
        returning ? .dialNow : .backoff(attempt: 1)
    }

    /// A new link's reads together, then `afterwards` once every one is in.
    ///
    /// The boards go in `afterwards`, never beside the reads: a sweep builds
    /// its list from the build, the fleet and the repositories, and one that
    /// ran before they landed swept an empty list and read nothing.
    @MainActor
    public static func bringUp(
        _ reads: [@MainActor @Sendable () async -> Void], afterwards: @MainActor () -> Void
    ) async {
        await together(reads)
        afterwards()
    }

    /// Run the reads a new link owes, all at once, and return when every one
    /// has. Each starts before any is answered: the core carries many calls at
    /// once, so reading them in turn only added round trips.
    @MainActor
    public static func together(_ reads: [@MainActor @Sendable () async -> Void]) async {
        await withTaskGroup(of: Void.self) { group in
            for read in reads {
                group.addTask { await read() }
            }
        }
    }
}

/// One answer from whichever finishes first: the ask or the deadline.
private final class FirstAnswer: Sendable {
    private let waiting = OSAllocatedUnfairLock<CheckedContinuation<LinkRevival.Probe, Never>?>(
        initialState: nil)

    func arm(_ continuation: CheckedContinuation<LinkRevival.Probe, Never>) {
        waiting.withLock { $0 = continuation }
    }

    func give(_ probe: LinkRevival.Probe) {
        let continuation = waiting.withLock { held -> CheckedContinuation<LinkRevival.Probe, Never>? in
            defer { held = nil }
            return held
        }
        continuation?.resume(returning: probe)
    }
}
