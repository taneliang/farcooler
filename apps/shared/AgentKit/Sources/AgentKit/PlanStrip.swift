import Foundation

// The plan in one line, on the orchestrator's screen (ov-300; Concept C of
// .claude/agent/reports/ov-298/research-layout.md, 3.3): the orchestrator's
// state as a mark, what needs you, what's moving and what's next. A tap peeks
// the whole plan. The phones draw it over the orchestrator's pane, the way
// the Mac draws its strip under the chat when the canvas is folded away.
//
// Every rule is here, as values: which parts are said, in what words, and
// what the orchestrator's state and line are. The views only lay it out. The
// words are the Mac's strip's (`PlanStrip.words` in apps/macos), so a person
// with both reads one sentence.

/// The orchestrator's state, as the strip marks it and the peek says it.
public enum PlanStripOrchestrator: Equatable, Sendable {
    /// No orchestrator runs this workspace.
    case none
    case starting
    case working
    /// It's asking you something.
    case needsYou
    /// Its last turn failed.
    case failed
    /// It finished a turn.
    case done
    case idle
    /// Its pane exited, was lost or can't be read.
    case stopped

    /// The word, as the Mac's orchestrator row says it.
    public var word: String {
        switch self {
        case .none: "No Orchestrator"
        case .starting: "Starting"
        case .working: "Working"
        case .needsYou: "Needs You"
        case .failed: "Failed"
        case .done: "Done"
        case .idle: "Idle"
        case .stopped: "Stopped"
        }
    }

    /// Its mark: an SF Symbol, shape before color.
    public var glyph: String {
        switch self {
        case .none: "person.crop.circle.dashed"
        case .starting: "hourglass"
        case .working: "circle.dotted"
        case .needsYou: "hand.raised.fill"
        case .failed, .stopped: "exclamationmark.triangle.fill"
        case .done: "checkmark.circle"
        case .idle: "pause.circle"
        }
    }

    /// Whether the mark takes a color: only a state that wants the owner.
    public var tone: PlanStripTone {
        switch self {
        case .needsYou: .attention
        case .failed, .stopped: .failure
        default: .quiet
        }
    }
}

/// Color only for what needs attention.
public enum PlanStripTone: Equatable, Sendable {
    case quiet
    case attention
    case failure
}

/// The strip's model.
public struct PlanStrip: Equatable, Sendable {
    /// A lane being worked, as the strip names it.
    public struct Lane: Equatable, Sendable {
        public var name: String
        public var state: LaneState

        public init(name: String, state: LaneState) {
            self.name = name
            self.state = state
        }
    }

    public var orchestrator: PlanStripOrchestrator
    /// The orchestrator's one line: the question it's blocked on, what it's
    /// doing, or the last thing it said. Said by the peek, not the strip,
    /// which sits over the pane that already shows it.
    public var line: String?
    /// The workspace's Needs You count (`WorkspaceNeedsYou.count`), the one
    /// number the Mac's title bar and tree say.
    public var needsYou: Int
    /// Up to `nowShown` lanes in Now, in the plan's order.
    public var now: [Lane]
    /// How many more lanes are in Now than are named.
    public var moreNow: Int
    /// The lane next up.
    public var next: String?
    /// The glance band's two lines (ov-503): Now by train and section, Next by
    /// rank. Set by `init(plan:)`; the strip's own parts above are unchanged.
    public var band: PlanGlanceSentence?

    /// How many Now lanes the strip names, as the Mac's does.
    public static let nowShown = 2

    public init(
        orchestrator: PlanStripOrchestrator, line: String? = nil, needsYou: Int, now: [Lane], moreNow: Int = 0,
        next: String? = nil, band: PlanGlanceSentence? = nil
    ) {
        self.orchestrator = orchestrator
        self.line = line
        self.needsYou = max(0, needsYou)
        self.now = now
        self.moreNow = max(0, moreNow)
        self.next = next
        self.band = band
    }

    /// The strip for `plan`: its working lanes and its next one. `statuses` is
    /// the board's card statuses by task id: without them a lane waiting on a
    /// decision would be called Next.
    public init(
        plan: PlanModel, needsYou: Int, orchestrator: PlanStripOrchestrator, line: String? = nil,
        statuses: [String: TaskStatus] = [:]
    ) {
        let working = plan.working
        // One reading of the plan for the band and for Next, so a lane
        // waiting on a decision is in neither.
        let sections = PlanSections(plan: plan, needsYou: needsYou, statuses: statuses)
        self.init(
            orchestrator: orchestrator, line: line, needsYou: needsYou,
            now: working.prefix(Self.nowShown).map { Lane(name: PlanWords.short($0.heading), state: $0.state) },
            moreNow: working.count - min(working.count, Self.nowShown),
            next: sections.waiting.next.first.map { PlanWords.short($0.lane.heading) },
            band: PlanGlanceSentence(sections: sections))
    }

    // MARK: Words

    /// "2 need you", "1 needs you"; nil when nothing does.
    public var needsYouWords: String? {
        needsYou > 0 ? PlanGlanceSentence.needsYouWords(needsYou) : nil
    }

    /// "mac-vis Building".
    public static func words(_ lane: Lane) -> String { "\(lane.name) \(PlanWords.state(lane.state))" }

    /// "next: plan-phones".
    public var nextWords: String? { next.map { "next: \($0)" } }

    /// "+2": the Now lanes not named.
    public var moreWords: String? { moreNow > 0 ? "+\(moreNow)" : nil }

    /// What the strip says after its mark, part by part.
    public var parts: [String] {
        [needsYouWords].compactMap { $0 } + now.map(Self.words) + [moreWords, nextWords].compactMap { $0 }
    }

    /// Whether there's anything to draw: with no plan, no asks and no lanes,
    /// the strip isn't drawn at all.
    public var isEmpty: Bool { parts.isEmpty }

    /// The parts as one line: "2 need you · mac-vis Building · next: plan-phones".
    public var text: String { parts.joined(separator: " · ") }

    /// The orchestrator's state as a sentence: "Orchestrator, working." A
    /// blocked orchestrator is "waiting on you", so it never says "need you" twice.
    private var stateSentence: String {
        switch orchestrator {
        case .none: "No orchestrator."
        case .needsYou: "Orchestrator, waiting on you."
        default: "Orchestrator, \(orchestrator.word.lowercased())."
        }
    }

    /// What VoiceOver and TalkBack say, each thing once: "Orchestrator,
    /// working. 2 need you, mac-vis Building, next: plan-phones." A blocked
    /// orchestrator is "waiting on you", so it never says "need you" twice.
    public var accessibilityLabel: String {
        parts.isEmpty ? stateSentence : "\(stateSentence) \(parts.joined(separator: ", "))."
    }

    /// The same for a strip that draws the glance band: the state, then the
    /// band's Now and Next lines, then what needs you. Without a band it is
    /// `accessibilityLabel`.
    public var bandAccessibilityLabel: String {
        guard let band else { return accessibilityLabel }
        let needs = needsYouWords.map { " \($0)." } ?? ""
        return "\(stateSentence) \(band.text)\(needs)"
    }
}
