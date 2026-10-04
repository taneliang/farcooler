import Foundation

/// How one finished agent's last turn ended, as `/v1/pulse` carries it (ov-239).
///
/// The watch hears from the relay over the pulse route and no other: it has no
/// push channel, and the phone's app sends it a fleet only while that app runs.
/// So a failure the runner has since resolved clears on the watch only if this
/// answer says so. It names an opaque terminal id and a boolean, never a
/// runner, label or path.
public struct PulseTurn: Sendable, Equatable {
    /// The agent's terminal id: `FleetSnapshot.Agent.id`.
    public var terminal: String
    /// Whether the turn failed, as the runner said. An agent the runner said
    /// nothing about is not listed at all.
    public var failed: Bool
    /// When the relay filed this word: its own clock, which the card's
    /// `updatedAt` reads too. What makes a success newer than a failure.
    public var at: Date

    public init(terminal: String, failed: Bool, at: Date) {
        self.terminal = terminal
        self.failed = failed
        self.at = at
    }
}

extension PulseTurn: Decodable {
    private enum CodingKeys: String, CodingKey { case terminal, failed, at }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        terminal = try container.decode(String.self, forKey: .terminal)
        failed = try container.decode(Bool.self, forKey: .failed)
        // Milliseconds, like the card's `updatedAt` (`AgentCardClock`, which
        // the watch doesn't compile). A word with no usable time can't be
        // newer than anything, so it's refused.
        let milliseconds = try container.decode(Double.self, forKey: .at)
        guard milliseconds > 0 else {
            throw DecodingError.dataCorruptedError(
                forKey: .at, in: container, debugDescription: "no usable time")
        }
        at = Date(timeIntervalSince1970: milliseconds / 1000)
    }
}

extension RunnerPulse {
    /// The turns `/v1/pulse` answered with: none from a relay that predates
    /// them, and none for an answer that isn't one. One entry this build can't
    /// read is dropped alone; it doesn't take the rest down.
    public static func decodeTurns(_ data: Data) -> [PulseTurn] {
        struct Answer: Decodable {
            var turns: [Entry]?
            struct Entry: Decodable {
                var turn: PulseTurn?
                init(from decoder: Decoder) throws { turn = try? PulseTurn(from: decoder) }
            }
        }
        return ((try? JSONDecoder().decode(Answer.self, from: data))?.turns ?? [])
            .compactMap(\.turn)
    }
}

extension FleetSnapshot {
    /// This snapshot with a failed mark taken off every agent the relay's pulse
    /// says finished well since, or nil when nothing changed.
    ///
    /// Only a `failed: false` stamped after the failure on disk clears it, so
    /// last turn's success can't erase this turn's failure.
    public func clearingFailures(vouchedByPulse turns: [PulseTurn], at now: Date) -> FleetSnapshot? {
        clearingFailures(
            resolved: turns.filter { !$0.failed }.map { ($0.terminal, $0.at) }, at: now)
    }

    /// Take the failed mark off each agent in `resolved` whose word is dated
    /// after the failure it clears, or nil when nothing changed.
    ///
    /// The one rule behind both the Live Activity's card (`clearingFailures
    /// (vouchedBy:at:)`, ov-186) and the pulse's turns (ov-239). It lives here
    /// rather than beside the card's rows because the watch compiles this file
    /// and not `AgentCardRows.swift`.
    ///
    /// A word with no date, or an older one, is last turn's news and clears
    /// nothing: the card can hold turn N's success while turn N+1's failure is
    /// on disk, and clearing on that would erase a real failure that nothing
    /// writes again until the app polls. Folded in with `merging`, so the agent
    /// is stamped as freshly heard from, which a relay's word is. The glyph goes
    /// with the mark: a cleared turn drawn as ✗ would be the same bug in
    /// `accessoryCircular`, which draws only the glyph.
    func clearingFailures(resolved: [(terminal: String, at: Date?)], at now: Date) -> FleetSnapshot? {
        var next = self
        var changed = false
        for word in resolved {
            guard var agent = next.agents.first(where: { $0.id == word.terminal }),
                agent.turnFailed,
                let said = word.at,
                said > (agent.observedAt ?? agent.activityChangedAt ?? .distantPast)
            else { continue }
            agent.turnFailed = false
            if agent.glyph == "✗" { agent.glyph = "✓" }
            next = next.merging(agent, at: now)
            changed = true
        }
        return changed ? next : nil
    }

    /// What a surface draws once the relay has been asked: the failed marks the
    /// pulse says are resolved taken off, then the quiet runners' working
    /// agents no longer stated as now. The one place the watch's list, the
    /// complication and the phone's widget apply a `RunnerPulse.Plan`, so none
    /// of them can read it differently. Nothing to apply is `self`.
    public func settled(by plan: RunnerPulse.Plan, at now: Date) -> FleetSnapshot {
        (clearingFailures(vouchedByPulse: plan.turns, at: now) ?? self).quietened(plan.unstated)
    }
}
