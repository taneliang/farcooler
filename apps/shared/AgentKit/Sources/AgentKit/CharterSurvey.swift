import Foundation

// The first run's charter (ov-519 phase 2b): what `farcooler --json repo
// survey` found in a repository, the confirm screen's rows and choices made
// from it, and the `workspace write-charter` arguments those choices make.
//
// Pure, so `swift test --package-path apps/shared/AgentKit` reads every rule
// back. The Mac draws it (`CharterSetupView`); the words are
// `FirstRunCopy.Charter`'s.
//
// **Every string from the survey is drawn as plain text.** The runner already
// passed each through `farcooler_core::phrase`, but a repository, GitHub or
// another runner wrote them: they go into `Text(verbatim:)`, never into
// Markdown or HTML (ov-519 2a review, R2).

/// A repository's survey, as the Charter screen reads it.
public struct CharterSurvey: Equatable, Sendable {
    /// One charter answer.
    public struct Answer: Equatable, Sendable {
        /// `workflow`, `done_means`, `review`, `who_decides`, `reaching_me`,
        /// `lanes`, `autonomy`, `initiative`, `instructions`, `commit_style`
        /// or `harness`; a newer runner may send more, which are left out.
        public let key: String
        /// A word to switch on, for the answers the screen can change:
        /// `workflow` is `direct` or `pull_requests`, `lanes` a count or
        /// `no_limit`, `initiative` `on` or `off`, `harness` a program.
        public let value: String
        /// The answer as the charter writes it.
        public let text: String
        /// What it was read from, or nil when nothing was: the default.
        public let signal: String?

        public init(key: String, value: String = "", text: String = "", signal: String? = nil) {
            self.key = key
            self.value = value
            self.text = text
            self.signal = signal
        }
    }

    public let answers: [Answer]
    /// The agents the runner found, by program (`Host.agents_found`).
    public let agents: [String]
    /// How long the read took.
    public let tookMilliseconds: Int

    public init(answers: [Answer], agents: [String] = [], tookMilliseconds: Int = 0) {
        self.answers = answers
        self.agents = agents
        self.tookMilliseconds = tookMilliseconds
    }

    /// The survey in `repo survey --json`'s output, or nil when it isn't one.
    public init?(json data: Data) {
        guard let body = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
            let answers = body["answers"] as? [[String: Any]]
        else { return nil }
        self.answers = answers.compactMap { a in
            guard let key = a["key"] as? String, !key.isEmpty else { return nil }
            let source = a["source"] as? String ?? "default"
            return Answer(
                key: key, value: a["value"] as? String ?? "", text: a["text"] as? String ?? "",
                signal: source == "default" || source.isEmpty ? nil : source)
        }
        agents = body["agents"] as? [String] ?? []
        tookMilliseconds = body["took_ms"] as? Int ?? 0
    }

    public func answer(_ key: String) -> Answer? { answers.first { $0.key == key } }
}

/// How finished work lands: `--landing`.
public enum CharterLanding: String, CaseIterable, Sendable {
    case pullRequests = "pull_requests"
    case direct

    /// What `--landing` takes.
    public var argument: String { self == .pullRequests ? "pull-requests" : "direct" }

    public var title: String {
        self == .pullRequests ? FirstRunCopy.Charter.pullRequests : FirstRunCopy.Charter.direct
    }
}

/// How many agents at once: `--lanes`.
public enum CharterLanes: Hashable, Sendable, Comparable {
    case count(Int)
    case noLimit

    /// From the survey's `value`: a count, or `no_limit`.
    public init?(value: String) {
        if value == "no_limit" {
            self = .noLimit
        } else if let n = Int(value), (1...99).contains(n) {
            self = .count(n)
        } else {
            return nil
        }
    }

    /// What `--lanes` takes.
    public var argument: String {
        switch self {
        case .count(let n): return String(n)
        case .noLimit: return "none"
        }
    }

    public var title: String {
        switch self {
        case .count(let n): return String(n)
        case .noLimit: return FirstRunCopy.Charter.noLimit
        }
    }

    /// In a sentence: "2 at once", "no limit".
    public var phrase: String {
        switch self {
        case .count(let n): return FirstRunCopy.Charter.suggestedLanes(n)
        case .noLimit: return FirstRunCopy.Charter.suggestedLanes(nil)
        }
    }
}

/// The three things the Charter screen can change.
public struct CharterChoices: Equatable, Sendable {
    public var landing: CharterLanding
    public var lanes: CharterLanes
    public var initiative: Bool

    public init(landing: CharterLanding, lanes: CharterLanes, initiative: Bool) {
        self.landing = landing
        self.lanes = lanes
        self.initiative = initiative
    }
}

/// One row of the Charter screen.
public struct CharterRow: Identifiable, Equatable, Sendable {
    /// What the row lets you change, if anything.
    public enum Control: Equatable, Sendable {
        case landing, lanes, initiative
        /// Read only: the charter's own words.
        case none
    }

    /// Where the answer came from, as the line under it says.
    public enum Provenance: Equatable, Sendable {
        /// A signal in the repository: "From" and what it was.
        case from(String)
        /// Nothing was found: "Default", and why that is fine where it says.
        case byDefault(String?)
        /// You changed it here: "Changed", and what Far Cooler suggested.
        case changed(suggested: String)

        /// The tag before the detail: "From", "Default" or "Changed".
        public var tag: String {
            switch self {
            case .from: return FirstRunCopy.Charter.from
            case .byDefault: return FirstRunCopy.Charter.byDefault
            case .changed: return FirstRunCopy.Charter.changed
            }
        }

        /// The words after the tag, if any.
        public var detail: String? {
            switch self {
            case .from(let signal): return signal
            case .byDefault(let why): return why
            case .changed(let suggested): return FirstRunCopy.Charter.changedFrom(suggested)
            }
        }
    }

    /// The survey's key, or `also` for the files and habits row.
    public let id: String
    public let title: String
    /// The charter's own words, for a row with no control; or what the
    /// control means, for one with.
    public let text: [String]
    public let provenance: Provenance
    public let control: Control
    /// A line under the text saying it will change: Who reviews and Agents
    /// may follow Changes Land, and show the survey's words for its way.
    public var note: String? = nil
}

/// The Charter screen's state: the survey, what it suggested, and what the
/// owner has chosen.
public struct CharterForm: Equatable, Sendable {
    public let survey: CharterSurvey
    /// What the survey suggested.
    public let suggested: CharterChoices
    /// What is chosen now; the suggestion to begin with.
    public var choices: CharterChoices

    /// The count the screen offers when the survey said nothing it can read.
    public static let defaultLanes = 2

    public init(survey: CharterSurvey) {
        self.survey = survey
        let landing = survey.answer("workflow").flatMap { CharterLanding(rawValue: $0.value) } ?? .direct
        let lanes = survey.answer("lanes").flatMap { CharterLanes(value: $0.value) } ?? .count(Self.defaultLanes)
        let initiative = survey.answer("initiative").map { $0.value != "off" } ?? true
        suggested = CharterChoices(landing: landing, lanes: lanes, initiative: initiative)
        choices = suggested
    }

    /// The counts Agents at Once offers: 1 to 4 and No Limit, and the
    /// survey's own count where it's another.
    public var laneOptions: [CharterLanes] {
        var options: [CharterLanes] = (1...4).map { .count($0) } + [.noLimit]
        if !options.contains(suggested.lanes) { options.append(suggested.lanes) }
        return options.sorted()
    }

    /// The rows, in the charter's order. Who decides and Reaching me aren't
    /// drawn: both are the skill's own defaults, and a paired phone always
    /// gets questions (owner, ov-519).
    public var rows: [CharterRow] {
        let words = FirstRunCopy.Charter.self
        var rows: [CharterRow] = []
        rows.append(
            CharterRow(
                id: "workflow", title: words.landingTitle, text: [words.landingDetail],
                provenance: provenance(
                    "workflow",
                    changed: choices.landing != suggested.landing
                        ? words.suggestedLanding(suggested.landing == .pullRequests) : nil,
                    byDefault: words.landingDefault),
                control: .landing))
        for (key, title) in [("done_means", words.doneTitle), ("review", words.reviewTitle), ("autonomy", words.autonomyTitle)] {
            guard let a = survey.answer(key), !a.text.isEmpty else { continue }
            // Both are written from the landing mode: changed here, the
            // daemon writes them for the other way (review S1).
            let follows = key != "done_means" && choices.landing != suggested.landing
            rows.append(
                CharterRow(
                    id: key, title: title, text: [a.text], provenance: provenance(key), control: .none,
                    note: follows ? words.followsLanding : nil))
        }
        rows.append(
            CharterRow(
                id: "lanes", title: words.lanesTitle, text: [words.lanesDetail],
                provenance: provenance(
                    "lanes", changed: choices.lanes != suggested.lanes ? suggested.lanes.phrase : nil,
                    byDefault: words.lanesDefault),
                control: .lanes))
        rows.append(
            CharterRow(
                id: "initiative", title: words.initiativeTitle, text: [words.initiativeDetail],
                provenance: provenance(
                    "initiative",
                    changed: choices.initiative != suggested.initiative ? words.suggestedInitiative(suggested.initiative) : nil,
                    byDefault: nil),
                control: .initiative))
        // The files agents already read, and the commit habit, under one
        // heading as the charter keeps them.
        let also = ["instructions", "commit_style"].compactMap { survey.answer($0) }
            .filter { $0.signal != nil && !$0.text.isEmpty }
        if !also.isEmpty {
            rows.append(
                CharterRow(
                    id: "also", title: words.alsoTitle, text: also.map(\.text),
                    provenance: .from(also.compactMap(\.signal).joined(separator: "; ")), control: .none))
        }
        return rows
    }

    private func provenance(_ key: String, changed suggestion: String? = nil, byDefault why: String? = nil)
        -> CharterRow.Provenance
    {
        if let suggestion { return .changed(suggested: suggestion) }
        if let signal = survey.answer(key)?.signal { return .from(signal) }
        return .byDefault(why)
    }

    /// `workspace write-charter` for `workspace` (its id).
    ///
    /// `--landing` always: the runner surveys again before it writes, and
    /// `gh` may answer the second time and not the first, or the other way
    /// round, so the way work lands is the one on screen, never the second
    /// read's (review M1). A choice that agrees with the signal keeps it in
    /// the charter's comment. Lanes and initiative are always the default,
    /// which can't drift, so they go only when changed, keeping "default".
    public func writeArguments(workspace: String, draft: Bool) -> [String] {
        var args = ["workspace", "write-charter", workspace]
        if draft { args.append("--draft") }
        args += ["--landing", choices.landing.argument]
        if choices.lanes != suggested.lanes { args += ["--lanes", choices.lanes.argument] }
        if choices.initiative != suggested.initiative { args += ["--initiative", choices.initiative ? "on" : "off"] }
        return args + ["--json"]
    }

    /// Whether anything was changed from the survey.
    public var isChanged: Bool { choices != suggested }
}

/// What `workspace write-charter --json` answered.
public struct CharterWritten: Equatable, Sendable {
    /// False when the workspace already had a charter, which is never
    /// overwritten: the orchestrator starts on that one.
    public let wrote: Bool

    public init(wrote: Bool) { self.wrote = wrote }

    public init?(json data: Data) {
        guard let body = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
            let wrote = body["wrote"] as? Bool
        else { return nil }
        self.wrote = wrote
    }
}
