import Foundation

// References that draw live (ov-269 design 3.4, the owner's ruling on Q4): a
// page names a card, a lane or a theme, and the app draws that thing's
// *current* words from what it already holds, so a page can't go stale about
// anything the board knows. A name nothing answers to (a dropped lane, a
// runner without the plan layer, a key on another board) draws its label or
// its name as plain text with no link: never an error, never an empty block.
//
// Navigation is the only action (Q3): a reference opens something the app
// already opens. A link outside the app opens only for `https`, with its
// domain drawn beside it (Q5), so a label can't hide where it goes.

/// Where a reference opens.
public enum PageDestination: Equatable, Hashable, Sendable {
    /// A task, by id.
    case task(String)
    /// A task's open question, by the task's id.
    case ask(String)
    /// A lane's page, by id.
    case lane(String)
    /// A theme's page, by id.
    case theme(String)
    /// Another page, by slot.
    case page(String)
    /// A worktree, by id.
    case worktree(String)
    /// A terminal pane in a worktree, by the worktree's id and the pane's name.
    case terminal(worktree: String, name: String)
    /// A web page, opened in the system browser. Only ever `https`.
    case url(URL)
}

/// What the app holds that a page's references are drawn from. Each platform
/// fills one from its own board, plan and worktrees; nothing here asks the
/// runner for anything.
public struct PageWorld: Equatable, Sendable {
    /// The board's cards by key (`ov-274`), lowercased.
    public var tasks: [String: TaskRow]
    /// The plan, or nil on a runner without `board_plan`: lane and theme
    /// references then draw as plain text.
    public var plan: PlanModel?
    /// The board's pages, by slot.
    public var pages: [String: BoardPage]
    /// Worktree ids by name.
    public var worktrees: [String: String]
    /// The terminals the app knows by name, as `worktree id/name`.
    public var terminals: Set<String>
    /// Now, for "Updated 12 min ago".
    public var nowMs: Int64

    public init(
        tasks: [TaskRow] = [], plan: PlanModel? = nil, pages: [BoardPage] = [], worktrees: [String: String] = [:],
        terminals: Set<String> = [], nowMs: Int64 = 0
    ) {
        self.tasks = Dictionary(tasks.map { ($0.key.lowercased(), $0) }, uniquingKeysWith: { a, _ in a })
        self.plan = plan
        self.pages = Dictionary(pages.map { ($0.slot, $0) }, uniquingKeysWith: { a, _ in a })
        self.worktrees = worktrees
        self.terminals = terminals
        self.nowMs = nowMs
    }

    /// A terminal's key in `terminals`.
    public static func terminalKey(worktree: String, name: String) -> String { "\(worktree)/\(name)" }
}

/// A reference as it's drawn now.
public struct PageResolved: Equatable, Sendable {
    /// Its name: a key, a lane's name, a theme's, a page's title, a link's
    /// label, or for one that resolves to nothing, its label or raw name.
    public var name: String
    /// Its live words, when it has some: a card's status, a lane's state, a
    /// theme's progress, a link's domain.
    public var status: String?
    /// Amber only for a question that's still open.
    public var statusTone: PageTone
    /// Where it opens; nil when it resolves to nothing, so it draws as plain
    /// text.
    public var destination: PageDestination?
    /// What VoiceOver says for it.
    public var spoken: String

    public init(name: String, status: String? = nil, statusTone: PageTone = .neutral, destination: PageDestination? = nil, spoken: String? = nil) {
        self.name = name
        self.status = status
        self.statusTone = statusTone
        self.destination = destination
        self.spoken = spoken ?? ([name] + [status].compactMap { $0 }).joined(separator: ", ")
    }

    /// Whether it resolved to something the app can open.
    public var resolved: Bool { destination != nil }
}

/// The words pages draw that aren't the orchestrator's.
public enum PageWords {
    /// What a question still waiting on the owner reads as.
    public static let needsYou = "Needs you"
    public static let answered = "Answered"
    /// A block from a newer runner that brought no `alt`.
    public static let newerBlock = "This part needs a newer Far Cooler."
    /// A page past the design's limits: what fits is drawn above it.
    public static let tooLarge = "This page is too large to show in full."
    /// A page listed without a document this build can read.
    public static let unreadable = "Far Cooler can’t draw this page. Update Far Cooler to see it."
    public static let couldntRead = "Far Cooler couldn’t read this board’s pages."
    public static let fromTheOrchestrator = "From the orchestrator"

    /// "Updated 12 min ago", or past the page's own limit, "Not updated for
    /// 3 hours": secondary, never amber, since a stale page isn't the owner's
    /// problem.
    public static func updated(_ page: BoardPage, now: Int64) -> String {
        let minutes = max(0, now - page.updatedAtMs) / 60_000
        if let limit = page.doc?.staleAfterMin, limit > 0, minutes > Int64(limit) {
            return "Not updated for \(span(minutes: minutes))"
        }
        return "Updated \(PlanWords.ago(page.updatedAtMs, now: now))"
    }

    /// Whether `updated` says the page is past its limit.
    public static func isStale(_ page: BoardPage, now: Int64) -> Bool {
        guard let limit = page.doc?.staleAfterMin, limit > 0 else { return false }
        return max(0, now - page.updatedAtMs) / 60_000 > Int64(limit)
    }

    /// "40 minutes", "3 hours", "2 days".
    static func span(minutes: Int64) -> String {
        func plural(_ n: Int64, _ unit: String) -> String { n == 1 ? "1 \(unit)" : "\(n) \(unit)s" }
        switch minutes {
        case ..<60: return plural(minutes, "minute")
        case ..<1440: return plural(minutes / 60, "hour")
        default: return plural(minutes / 1440, "day")
        }
    }

    /// "2 of 4".
    public static func progress(done: Int, total: Int) -> String { "\(done) of \(total)" }

    /// A lane's tokens for a table cell: "470K", or "Not reported".
    public static func tokens(_ spend: PlanSpend) -> String {
        spend.totalTokens > 0 ? TaskUsageFormat.tokens(spend.totalTokens) : PlanWords.notReported
    }
}

extension PageWorld {
    /// `ref` as it's drawn now.
    public func resolve(_ ref: PageRef) -> PageResolved {
        let label = ref.label.flatMap { $0.isEmpty ? nil : $0 }
        let plain = PageResolved(name: label ?? ref.target.rawName)
        switch ref.target {
        case .task(let key):
            guard let row = tasks[key.lowercased()] else { return plain }
            return PageResolved(
                name: label ?? row.key, status: row.status.title, destination: .task(row.id),
                spoken: [label ?? row.key, row.title, row.status.title].joined(separator: ", "))
        case .ask(let key):
            guard let row = tasks[key.lowercased()] else { return plain }
            let open = row.status == .needsDecision
            return PageResolved(
                name: label ?? row.key, status: open ? PageWords.needsYou : PageWords.answered,
                statusTone: open ? .attention : .neutral, destination: open ? .ask(row.id) : .task(row.id),
                spoken: [label ?? row.key, row.title, open ? PageWords.needsYou : PageWords.answered].joined(separator: ", "))
        case .lane(let name):
            guard let lane = plan?.lanes.first(where: { $0.name == name }) else { return plain }
            return PageResolved(name: label ?? lane.name, status: PlanWords.status(lane), destination: .lane(lane.id))
        case .theme(let name):
            guard let theme = plan?.themes.first(where: { $0.name == name || $0.short == name }) else { return plain }
            return PageResolved(name: label ?? theme.name, status: PlanWords.progress(theme.counts), destination: .theme(theme.id))
        case .page(let slot):
            guard let page = pages[slot] else { return plain }
            return PageResolved(name: label ?? page.title, destination: .page(slot))
        case .worktree(let name):
            guard let id = worktrees[name] else { return plain }
            return PageResolved(name: label ?? name, destination: .worktree(id))
        case .terminal(let worktree, let name):
            guard let id = worktrees[worktree], terminals.contains(Self.terminalKey(worktree: id, name: name)) else { return plain }
            return PageResolved(name: label ?? name, destination: .terminal(worktree: id, name: name))
        case .url(let raw):
            guard let url = PageLinks.https(raw), let host = url.host() else { return plain }
            return PageResolved(
                name: label ?? host, status: label == nil ? nil : host, destination: .url(url),
                spoken: label.map { "\($0), link to \(host)" } ?? "Link to \(host)")
        case .ci:
            let name = label ?? Self.ciName(ref.target)
            guard let subject = ref.target.ciSubject, let read = plan?.ci(subject) else { return PageResolved(name: name) }
            let stale = PlanWords.ciStale(read, now: plan?.nowMs ?? nowMs)
            return PageResolved(
                name: name, status: ([PlanWords.ciSummary(read)] + [stale].compactMap { $0 }).joined(separator: " · "),
                statusTone: read.needsAttention ? .attention : .neutral,
                destination: PageLinks.https(read.url).map(PageDestination.url))
        case .cards(let status):
            let name = label ?? Self.statusName(status)
            guard let count = plan?.cardCount(status) else { return PageResolved(name: name) }
            return PageResolved(name: name, status: "\(count)", spoken: "\(name), \(count)")
        case .unknown:
            return plain
        }
    }

    /// What a CI reference is called without a label: "Main", "Run 812", or
    /// the commit's first eight digits.
    static func ciName(_ target: PageTarget) -> String {
        guard case .ci(let s) = target else { return target.rawName }
        if s == "main" { return "Main" }
        if s.hasPrefix("run:") { return "Run \(s.dropFirst(4))" }
        return String(s.prefix(8))
    }

    /// A card-count reference's status, as the board says it.
    static func statusName(_ word: String) -> String {
        if word == "open" { return OneTreeFilter.open.title }
        return TaskStatus(rawValue: word)?.title ?? word
    }

    /// A theme by its name or short id.
    private func theme(of ref: PageRef) -> PlanTheme? {
        guard case .theme(let name) = ref.target else { return nil }
        return plan?.themes.first { $0.name == name || $0.short == name }
    }

    /// What a figure draws (ov-306): its value as written, or its reference's
    /// live value, with a detail and whether it needs the owner. A CI figure is
    /// its status word, with how its jobs stand as the detail unless the page
    /// gave one; a card count is the number.
    public func statText(_ stat: PageStat) -> (value: String, detail: String?, tone: PageTone) {
        guard let ref = stat.ref else { return (stat.value, stat.detail, stat.tone) }
        let resolved = resolve(ref)
        switch ref.target {
        case .ci:
            guard let subject = ref.target.ciSubject, let read = plan?.ci(subject) else {
                return (resolved.name, stat.detail, stat.tone)
            }
            // A stale read says how old it is before anything else under it.
            let detail = PlanWords.ciStale(read, now: plan?.nowMs ?? nowMs) ?? stat.detail ?? PlanWords.ciJobs(read)
            return (PlanWords.ciStatus(read.status), detail, read.needsAttention ? .attention : stat.tone)
        case .cards:
            return (resolved.status ?? resolved.name, stat.detail, stat.tone)
        case .lane:
            let value = stat.show == .spend ? lane(of: ref).map { PageWords.tokens($0.spend) } : lane(of: ref).map(PlanWords.status)
            return (value ?? resolved.name, stat.detail, stat.tone)
        case .theme:
            let value = stat.show == .spend ? theme(of: ref).map { PageWords.tokens($0.spend ?? PlanSpend()) } : resolved.status
            return (value ?? resolved.name, stat.detail, stat.tone)
        default:
            return (resolved.status ?? resolved.name, stat.detail, resolved.statusTone == .attention ? .attention : stat.tone)
        }
    }

    /// What a reference cell draws: its own text when it has some, else the
    /// target's name, a lane's state words or a lane's tokens.
    public func cellText(_ cell: PageCell) -> String {
        if let text = cell.text { return text }
        guard let ref = cell.ref else { return "" }
        let resolved = resolve(ref)
        switch cell.show {
        case .name:
            // Live data draws its value (ov-306): CI's status, a count.
            switch ref.target {
            case .ci, .cards: return resolved.status ?? resolved.name
            default: return resolved.name
            }
        case .state:
            return lane(of: ref).map(PlanWords.status) ?? resolved.name
        case .spend:
            if let theme = theme(of: ref) { return PageWords.tokens(theme.spend ?? PlanSpend()) }
            return lane(of: ref).map { PageWords.tokens($0.spend) } ?? resolved.name
        }
    }

    private func lane(of ref: PageRef) -> PlanLane? {
        guard case .lane(let name) = ref.target else { return nil }
        return plan?.lanes.first { $0.name == name }
    }
}

/// The one rule for links leaving the app.
public enum PageLinks {
    /// `raw` as a URL the browser may open: `https`, with a host, and no
    /// user name or password to dress one domain up as another. The host is
    /// plain ASCII (letters, digits, dots and hyphens; punycode shows as
    /// `xn--`), so the domain drawn beside a link is the one it goes to, never
    /// a look-alike in another script. Anything else is nil, and draws as
    /// plain text.
    public static func https(_ raw: String) -> URL? {
        guard let url = URL(string: raw), url.scheme?.lowercased() == "https", let host = url.host(), !host.isEmpty,
            url.user() == nil, url.password() == nil,
            host.unicodeScalars.allSatisfy({ $0.isASCII && (CharacterSet.alphanumerics.contains($0) || $0 == "." || $0 == "-") })
        else { return nil }
        return url
    }
}

/// Where a board's pages are drawn (design 6.1): in the Plan view's Pages
/// section, or as sections of the theme they're anchored to. A page whose
/// theme is gone or dropped falls back into the Pages section, so it's never
/// lost. The phones read it from here (ov-285); the Mac's `PlanStore` has the
/// same rule beside its own hidden pages.
public enum PageShelf {
    /// The themes a page can be drawn inside: those the plan still has and
    /// hasn't dropped. None without a plan.
    public static func liveThemes(_ plan: PlanModel?) -> Set<String> {
        Set((plan?.themes ?? []).filter { $0.state != "dropped" }.map(\.id))
    }

    /// The Pages section's rows: pages of their own, and anchored pages whose
    /// theme is gone, in the runner's order.
    public static func listed(_ pages: [BoardPage], plan: PlanModel?) -> [BoardPage] {
        let themes = liveThemes(plan)
        return pages.filter { page in page.themeAnchor.map { !themes.contains($0) } ?? true }
    }

    /// The pages drawn inside `theme`'s page.
    public static func anchored(_ pages: [BoardPage], to theme: String, plan: PlanModel?) -> [BoardPage] {
        guard liveThemes(plan).contains(theme) else { return [] }
        return pages.filter { $0.themeAnchor == theme }
    }
}
