import Foundation

// What a task key is, at a glance (ov-299): the card a key's hovercard (the
// Mac) or long-press preview (the phones) shows. The app name-drops keys
// everywhere, in terminals, notes, plan rows and pages, and without this a key
// meant hunting for it in Tasks.
//
// Built from what the app has already read: the boards (`TaskBoardModel`), a
// board's plan when it was read (themes and lanes), and the records of the
// cards that were opened (their latest note). A hover is a dictionary lookup,
// never a read of the runner. A key that isn't on a board this runner's app
// has read, or a link to another runner's task, has no card, and shows
// nothing: a card for the wrong task is worse than none. Kotlin's
// `TaskKeyCards.kt` is the same rules.

/// One task, as its key's hovercard draws it.
public struct TaskKeyCard: Equatable, Hashable, Sendable {
    public var key: String
    public var title: String
    public var status: TaskStatus
    /// The plan theme it's in, by name; nil when no plan was read or no
    /// theme names it.
    public var theme: String?
    /// The plan lane working it, by name: a live one if any, else the last
    /// to finish. Nil when no plan was read or no lane names it.
    public var lane: String?
    /// The latest thing written on it, one line: its newest note a person or
    /// agent wrote, when its record was read, else its intent's first line.
    /// Empty when it has neither.
    public var excerpt: String

    public init(
        key: String, title: String, status: TaskStatus, theme: String? = nil, lane: String? = nil,
        excerpt: String = ""
    ) {
        self.key = key
        self.title = title
        self.status = status
        self.theme = theme
        self.lane = lane
        self.excerpt = excerpt
    }

    /// The line under the title: status, theme and lane, those it has.
    /// "In Progress · Task-key hovercards · key-hover".
    public var details: String {
        ([status.title] + [theme, lane].compactMap { $0 }).joined(separator: " · ")
    }

    /// What VoiceOver and TalkBack say for the key: the key, then the title,
    /// so a listener knows what "ov-299" is without opening it.
    public var accessibilityLabel: String { "\(key), \(title)" }

    /// The longest excerpt shown, in characters, before an ellipsis.
    public static let excerptLimit = 160

    /// `text`'s first non-empty line, trimmed, Markdown's leading markers
    /// dropped, and cut at `excerptLimit` with an ellipsis.
    public static func excerpt(_ text: String) -> String {
        let line =
            text.split(whereSeparator: \.isNewline)
            .map { $0.trimmingCharacters(in: .whitespaces) }
            .first { !$0.isEmpty } ?? ""
        var trimmed = Substring(line)
        while let first = trimmed.first, "#>-*".contains(first) { trimmed = trimmed.dropFirst() }
        let plain = trimmed.trimmingCharacters(in: .whitespaces)
        guard plain.count > excerptLimit else { return plain }
        let cut = plain.prefix(excerptLimit - 1)
        // At a word, when one ends in the last fifth, so the line doesn't
        // stop mid-word.
        if let space = cut.lastIndex(of: " "), cut.distance(from: space, to: cut.endIndex) < excerptLimit / 5 {
            return cut[..<space].trimmingCharacters(in: .whitespaces) + "…"
        }
        return cut.trimmingCharacters(in: .whitespaces) + "…"
    }
}

/// One runner's cards, by key: what every hovercard and preview looks up.
public struct TaskKeyCards: Equatable, Sendable {
    public var runner: String
    public var cards: [String: TaskKeyCard]

    public static let empty = TaskKeyCards(runner: "", cards: [:])

    public init(runner: String, cards: [String: TaskKeyCard]) {
        self.runner = runner
        self.cards = cards
    }

    /// A runner's cards from what was read of it: its boards and their plans
    /// by workspace id, and the notes of the cards whose records were read,
    /// by task id (oldest first, as the store sends them). The first board in
    /// workspace order wins a key two boards share, as `TaskKeyIndex` does.
    public init(
        runner: String, boards: [String: TaskBoardModel], plans: [String: PlanModel] = [:],
        notes: [String: [TaskNoteRow]] = [:]
    ) {
        var cards: [String: TaskKeyCard] = [:]
        for workspace in boards.keys.sorted() {
            let plan = plans[workspace].map(PlanPlacement.init) ?? PlanPlacement()
            for row in boards[workspace]?.rows ?? [] where cards[row.key] == nil {
                let note = notes[row.id].flatMap(Self.latestWritten)
                cards[row.key] = TaskKeyCard(
                    key: row.key, title: row.title, status: row.status, theme: plan.themes[row.id],
                    lane: plan.lanes[row.id], excerpt: TaskKeyCard.excerpt(note ?? row.intent))
            }
        }
        self.init(runner: runner, cards: cards)
    }

    /// `key`'s card, or nil for a key no board read has.
    public func card(for key: String) -> TaskKeyCard? { cards[key] }

    /// The card a task link names (`TaskKeyLinks.url`), or nil: another
    /// runner's task, an unknown key, or any other URL.
    public func card(for url: URL) -> TaskKeyCard? {
        guard let (runner, key) = TaskKeyLinks.parse(url), runner == self.runner else { return nil }
        return cards[key]
    }

    /// The newest note a person or agent wrote, skipping the store's own
    /// (moves, creation, starts, subagents), whose bodies aren't anybody's
    /// word on the task.
    static func latestWritten(_ notes: [TaskNoteRow]) -> String? {
        notes.filter { !$0.kind.isMachineWritten && !$0.body.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty }
            .max { $0.at < $1.at }?.body
    }
}

/// Which theme and lane a plan puts each task in, by task id.
private struct PlanPlacement {
    var themes: [String: String] = [:]
    var lanes: [String: String] = [:]

    init() {}

    init(_ plan: PlanModel) {
        for theme in plan.themes.sorted(by: { $0.ordinal < $1.ordinal }) {
            for card in theme.cards where themes[card.task] == nil { themes[card.task] = theme.name }
        }
        // A live lane first, then the most recently moved finished one.
        let lanes = plan.lanes.sorted {
            $0.state.isLive != $1.state.isLive ? $0.state.isLive : $0.stateSince > $1.stateSince
        }
        for lane in lanes {
            for card in lane.cards where self.lanes[card.task] == nil { self.lanes[card.task] = lane.name }
        }
    }
}

/// The cards for one runner, built once per change in what was read rather
/// than on every pass of every view that draws a key. A window rebuilds its
/// linker on each pass; this hands back the cards it built last while the
/// boards, plans and notes are the ones it built them from.
@MainActor
public final class TaskKeyCardCache {
    private var inputs: Inputs?
    private var built: TaskKeyCards = .empty
    /// How many times it has built, for a test.
    public private(set) var builds = 0

    private struct Inputs: Equatable {
        var runner: String
        var boards: [String: TaskBoardModel]
        var plans: [String: PlanModel]
        var notes: [String: [TaskNoteRow]]
    }

    public init() {}

    /// The cards for these reads: the last ones built, when the reads are the
    /// same.
    public func cards(
        runner: String, boards: [String: TaskBoardModel], plans: [String: PlanModel] = [:],
        notes: [String: [TaskNoteRow]] = [:]
    ) -> TaskKeyCards {
        let next = Inputs(runner: runner, boards: boards, plans: plans, notes: notes)
        if next == inputs { return built }
        inputs = next
        built = TaskKeyCards(runner: runner, boards: boards, plans: plans, notes: notes)
        builds += 1
        return built
    }
}
