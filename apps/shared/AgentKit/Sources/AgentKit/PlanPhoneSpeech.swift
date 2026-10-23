import Foundation

// What the phone's Plan view decides, as values (ov-538). These rules used to
// live in the iOS views' bodies, where only a launched app could check them:
// which of Waiting's groups are named, the sentence VoiceOver reads for a
// train, an agent, a theme tile and the band, which rulings and buttons show,
// and what keeping a ruling does to the plan on screen. The iOS target has no
// unit tests, so they are here, and the views only draw what these answer.

// MARK: Waiting

extension PlanSections.Waiting {
    /// One of Waiting's groups.
    public struct Group: Equatable, Identifiable, Sendable {
        public var title: String
        public var rows: [PlanSections.LaneRow]
        /// Its rows carry their place, 1, 2, 3: the plan's own order.
        public var ranked: Bool
        /// It waits on the owner: drawn with the amber mark.
        public var attention: Bool
        public var id: String

        /// The number a row at `index` carries, or nil in a group that isn't ranked.
        public func rank(at index: Int) -> Int? { ranked ? index + 1 : nil }
    }

    /// The groups that have rows, in the order they are drawn.
    public var groups: [Group] {
        [
            Group(title: "Next Up", rows: next, ranked: true, attention: false, id: "plan-next-up"),
            Group(title: "Not Ranked", rows: notRanked, ranked: false, attention: false, id: "plan-not-ranked"),
            Group(title: "Waiting on You", rows: onYou, ranked: false, attention: true, id: "plan-waiting-on-you"),
        ].filter { !$0.rows.isEmpty }
    }

    /// Each group is named only when something else shares the section.
    public var namesGroups: Bool { groups.count > 1 }
}

// MARK: Trains

extension PlanSections.TrainCard {
    /// What VoiceOver reads for a train row, and what the UI tests read as its
    /// label: "Train 86, Teams on a shared runner, …, Checking on CI · step 2
    /// of 3, 3f78754f · … · Integrator · Opus".
    public func spoken(ci: PlanCIRead?, now: Int64) -> String {
        [train.label, train.subtitle, train.carries, PlanLadder.compact(phase), facts(ci: ci, now: now)]
            .compactMap { $0 }.filter { !$0.isEmpty }.joined(separator: ", ")
    }

    /// A long press says what a train is, then each step of the ladder, the
    /// one it stands on in the phase's own words.
    public var explanation: [String] {
        let current = PlanLadder.step(phase)
        return [PlanWords.trainExplanation]
            + PlanLadder.steps.indices.map { index in
                "\(PlanLadder.label(index, current: current, phase: phase)): \(PlanLadder.explanation(index, current: current, phase: phase))"
            }
    }
}

// MARK: Agents, tiles and the band

extension PlanFleetAgent {
    /// What VoiceOver reads for an agent's row: who, for what and its state,
    /// then its tokens.
    public func spoken(now: Int64) -> String {
        [who, second(now: now), tokens().map { "\($0) tokens" }].compactMap { $0 }.joined(separator: ", ")
    }
}

extension PlanCardGlyph {
    /// What VoiceOver says for a card's mark.
    public var word: String {
        switch self {
        case .done: "Done"
        case .inProgress: "In progress"
        case .notStarted: "Not started"
        }
    }
}

extension PlanThemeTile {
    /// What VoiceOver reads for a tile: its name and progress, each card with
    /// its mark, the budget when it is over, and what waits on the owner.
    /// `budget` is `PlanWords.budgetLine` of an overspent theme, or nil.
    public func spoken(budget: String? = nil) -> String {
        var parts = [name, progress]
        parts += cards.map { "\($0.title.isEmpty ? $0.key : $0.title), \($0.glyph.word.lowercased())" }
        if let budget { parts.append(budget) }
        if let ask = ownerAsk { parts.append("Needs you: \(ask)") }
        return parts.joined(separator: ", ")
    }
}

extension PlanGlanceSentence {
    /// What VoiceOver reads for the band: "No lanes moving. Next: nothing
    /// queued. 2 need you." At the sheet's medium detent (`nowOnly`) the Now
    /// line alone.
    public func spoken(needsYou: Int, nowOnly: Bool = false) -> String {
        let said = nowOnly ? "\(nowLine)." : text
        return needsYou > 0 ? "\(said) \(Self.needsYouWords(needsYou))." : said
    }
}

// MARK: Rulings

extension RulingActions {
    /// Whether this device offers the owner's marks (Keep, Keep All, Reverse,
    /// Discuss): the runner takes them, the grant isn't Read, and the device
    /// isn't someone else's (ov-707). Either would flip a ruling kept and
    /// silently put it back. `viewer` is `RunnerPeople.you`: "" for the
    /// owner, nil until the runner has said.
    public static func canMark(runnerTakesMarks: Bool, grantedScope: String?, viewer: String?) -> Bool {
        runnerTakesMarks && grantedScope != "read" && isOwner(viewer)
    }

    /// Whether `viewer` (`RunnerPeople.you`) may be the owner: "", or not
    /// said yet. A member or a released device isn't.
    public static func isOwner(_ viewer: String?) -> Bool {
        viewer?.isEmpty ?? true
    }
}

/// What Decided For You draws on the Plan, from the plan and what the phone may do.
public struct PlanRulingsShown: Equatable, Sendable {
    /// Open rulings, newest first.
    public var open: [PlanRuling]
    /// Kept and reversed ones, most recently settled first: Past Decisions.
    public var past: [PlanRuling]
    /// Keep All sits in the header: the owner may mark, and more than one is open.
    public var showsKeepAll: Bool

    /// Nothing at all on a runner without `board_rulings`, or with no rulings.
    public init(plan: PlanModel, keeps: Bool, canMark: Bool) {
        guard keeps, !plan.rulings.isEmpty else {
            open = []
            past = []
            showsKeepAll = false
            return
        }
        open = plan.openRulings
        past = plan.pastRulings
        showsKeepAll = canMark && open.count > 1
    }

    public var isEmpty: Bool { open.isEmpty && past.isEmpty }
}

extension PlanModel {
    /// The plan with `ids` shown kept now (ov-333), before the runner's answer:
    /// Keep is the owner's own mark and lands like read state. A ruling that
    /// isn't open is left as it is. The read that follows makes it the
    /// runner's word again, or puts it back.
    public func showingKept(_ ids: Set<String>) -> PlanModel {
        var plan = self
        for index in plan.rulings.indices where ids.contains(plan.rulings[index].id) && plan.rulings[index].isStanding {
            plan.rulings[index].state = .confirmed
            plan.rulings[index].settledBy = "user"
            plan.rulings[index].settledAt = plan.nowMs
        }
        return plan
    }
}

// MARK: Which boards read again

extension PlanNews {
    /// The boards a `plan` notice makes read again: the ones that have asked for
    /// their plan and that it names. Others never read until someone picks Plan.
    public func boardsToRead(asked: Set<String>, among boards: [String]) -> [String] {
        boards.filter { asked.contains($0) && touches($0) }
    }

    /// The boards a task or resync notice moved that read again: a theme's
    /// counts and "needs you" are the board's statuses, derived on read, so
    /// every moved board that has asked for its plan reads it again.
    public static func boardsToReadAfterTasks(moved: [String], asked: Set<String>) -> [String] {
        moved.filter { asked.contains($0) }
    }
}
