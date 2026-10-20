import Foundation

// A worktree's panes, and the phone's stack, with no screen attached.
//
// Everything here is a value type or a pure function, which is where the
// tests can reach: the iOS app target has no unit test bundle, so a rule that
// lives in a `View` can only ever be checked by a person looking at it. The
// shell's bar and its gestures are gone (ov-617); what is left is the fleet
// a worktree's panes are positions in, how a position survives a poll, and
// the phone's routes.
//
// Internal, like everything in `CoreModel.swift` and for the same reason its
// header gives: the phone compiles AgentKit's sources straight into the app
// module, while the Mac depends on AgentKit as a real module — so the Mac
// cannot see one name from this file.

/// One tab, as the shell needs it: something to name and something to draw.
///
/// Deliberately not a `Terminal`. The shell shows a worktree's tabs, and a
/// worktree's tabs are its terminals AND its diff — see `TerminalTabStrip`'s
/// `ChangesChip`, which is a tab with no terminal behind it and never can
/// have one. A shape that could only hold terminals would have to special-case
/// the diff at every ribbon, column row and overview card.
struct ShellTab: Identifiable, Hashable {
    var id: String
    /// What the column row says. Short: the column is one bar wide.
    var title: String

    /// **The product's mark, not a fourth vocabulary.**
    ///
    /// This was a `ShellMark` — a private enum of four cases, `needsYou`,
    /// `unreadDiff`, `working` and `stale` — left over from the shell landing
    /// before the glance spec existed. It is gone, and the reason it had to go
    /// is not tidiness: those four cases collapsed three independent axes into
    /// one dimension, and the collapse was LOSSY in a way that reached the
    /// screen. `working` was the catch-all — it meant an agent producing, an
    /// agent idle at a prompt, an agent whose status the daemon had never
    /// stated, AND a Diff tab with nothing new in it. Four different facts
    /// drawn as one dot. So the bar could not say that an agent was working
    /// even in principle, and the owner reported exactly that: the Mac showed a
    /// filled indicator while the phone's bar showed nothing until the turn was
    /// over. Android's `ShellTab` reached this shape first and named the seam.
    ///
    /// A `GlanceMark` says all three axes separately, so nothing has to be
    /// discarded on the way in. `ShellScreen.mark(of:now:)` is where one is
    /// built, and it is the same table `GlanceMark.init(agent:)` uses.
    ///
    /// **`Attention.toReview` on an AGENT tab means a finished turn, and on
    /// the DIFF tab means unread changes — and those are the only two things
    /// it may ever mean here.** The prohibition the old `unreadDiff` case
    /// carried is unchanged and still binding: an unread-diff count comes from
    /// `InboxRow`, which counts a WORKTREE's changed lines and knows nothing
    /// about any agent, and `NeedsYou.swift:94-97` refuses to invent a
    /// per-agent version of it. Putting THAT on an agent tab would draw a ring
    /// meaning "this agent has unread changes", which is not a fact anything on
    /// the wire has an opinion about. `ShellScreen.diffMark` is the only place
    /// allowed to produce it from a diff, and it is only ever called for the
    /// Diff tab.
    var mark: GlanceMark

    /// Whether this tab is asking for a person, by the app's own single
    /// definition — `AgentActivity.wantsAttention`, blocked or done, shared
    /// with the Mac since long before the glance vocabulary existed.
    ///
    /// **Carried rather than re-derived from `mark`, because it is deliberately
    /// BROADER than the amber ring.** Since `done` joined the review tier a
    /// finished turn draws the middle-weight review ring rather than the heavy
    /// amber one — but it is still a tab you should be shown first. What a
    /// mark SAYS and whether somebody is WANTED are different questions.
    ///
    /// **Nothing in the app reads it today.** It ranked worktrees for the
    /// overview (`ShellWorktree.precedence`), and both are gone; `ShellScreen`
    /// still sets it, `said(answering:)` still leaves it alone, and the tests
    /// pin that. Delete it with the first change that touches every tab
    /// fixture, unless a surface has come to want it by then. Android's
    /// `ShellTab` makes the same split.
    var wantsAttention: Bool

    /// Whether this tab is a terminal somebody could close.
    ///
    /// **False for the Diff tab and true for every other tab there is**, which
    /// is the whole of the rule and is why it is a `Bool` rather than a pane
    /// reference. A worktree's column is one synthesized diff followed by its
    /// terminals (`ShellScreen.one(_:naming:now:)`), and the diff is the one
    /// row with no process behind it: it is not removable, and closing the last
    /// terminal in a worktree lands on it rather than on nothing.
    ///
    /// Carried rather than derived from the row's POSITION. "Row 0 is the diff"
    /// is true today and is true in exactly one file; a column that read it off
    /// the index would offer Close on a diff the day anything is ever put in
    /// front of it, and offering the destructive action on the wrong row is not
    /// a defect that shows up in a screenshot.
    ///
    /// What it does NOT say is whether closing needs confirming — that is a
    /// question about the live process, it changes between polls, and
    /// `ShellClose.mustAsk(about:)` answers it from the `Terminal` at the
    /// moment somebody actually swipes. A tab is a drawing; a terminal is a
    /// process.
    var closable: Bool

    /// Whether this tab is a workspace's orchestrator.
    ///
    /// The daemon opens every orchestrator in its repository's main checkout
    /// (`Service::start_orchestrator`), so Billing's orchestrator is a pane in
    /// a worktree Main may own. It is a tab of the worktree it runs in,
    /// because that is where its pane is, and is titled for its workspace —
    /// see `Fleet.orchestratorTitles`.
    var isOrchestrator: Bool

    init(
        id: String, title: String, mark: GlanceMark, wantsAttention: Bool = false,
        closable: Bool = false, isOrchestrator: Bool = false
    ) {
        self.id = id
        self.title = title
        self.mark = mark
        self.wantsAttention = wantsAttention
        self.closable = closable
        self.isOrchestrator = isOrchestrator
    }
}

/// One worktree: a name, and the tabs in their fixed order.
///
/// **The order is fixed and is never re-sorted by activity.** The ribbon under
/// the worktree name is a map of the worktree, and a map whose landmarks
/// move when something happens is not a map — you would have to read it every
/// time instead of remembering it. The same argument is why nothing sorts the
/// overview's WORKTREES by activity either: each runner's section is in the
/// order that runner keeps.
struct ShellWorktree: Identifiable, Hashable {
    var id: String
    var name: String
    var tabs: [ShellTab]
    /// The runner this worktree lives on, and only when it is worth saying.
    ///
    /// Nil for the local machine. The bar shows it after the name so a fleet
    /// spread across several runners can be told apart at a glance, and shows
    /// nothing at all for the common case — a name that is always there stops
    /// being read, and this bar has one job before it has two.
    var server: String?
    /// Which tab this worktree should be REOPENED on, as an index into
    /// `tabs`.
    ///
    /// "Reopened" is the whole of the distinction, and it is why this is on the
    /// worktree rather than being a rule inside `step`. There are two ways to
    /// arrive at another worktree and they are not the same journey:
    ///
    /// - **The content swipe walks a continuum.** One flat sequence of tabs
    ///   through the whole fleet, spilling into the next worktree's nearest
    ///   tab — its first going forward, its last coming back. That has to stay
    ///   literal: a sequence that skipped to a remembered tab would not be a
    ///   sequence, it would drift as you swiped back and forth, and swiping
    ///   right then left would land you somewhere you had not been. See
    ///   `contentStep`, which does not read this.
    /// - **The bar swipe, the carried lift and an overview card are all
    ///   GOING somewhere**, by name, deliberately. Going back to a worktree
    ///   should put you where you were in it, which is the same F4 argument
    ///   `docs/jobs-to-be-done.md` makes about the app being put down every
    ///   ninety seconds — landing on tab 0 is the app half-remembering.
    ///
    /// Resolved by whoever builds the fleet, from the ONE memory the app
    /// already keeps: `Connection.lastFocus`, written only when a person
    /// chooses a tab. A second memory living in here would be a second thing
    /// to disagree with it. Nil means nobody has ever chosen a tab in this
    /// worktree, and nil resolves to the first tab, which is where a
    /// worktree nobody has an opinion about opens.
    ///
    /// An index rather than a tab id because that is what a `ShellPosition`
    /// is, and because the caller has just built `tabs` and is the only thing
    /// that can turn a remembered id into one honestly. A remembered pane that
    /// has since gone must not resolve to "index 0 by accident"; see
    /// `Route.Focus.rule(for:inbox:)`, which is what the caller falls back to.
    var resume: Int?
    init(
        id: String, name: String, server: String? = nil, resume: Int? = nil,
        tabs: [ShellTab]
    ) {
        self.id = id
        self.name = name
        self.tabs = tabs
        self.server = server
        self.resume = resume
    }

    /// The tab a deliberate arrival lands on: the remembered one where it
    /// still exists, and the first otherwise.
    ///
    /// Clamped rather than trusted. `resume` is resolved against the fleet the
    /// caller was holding, and a poll between building it and reading it can
    /// have taken a terminal away — a stale index is an ordinary event here
    /// and must land on a tab rather than trap.
    var resumeTab: Int {
        guard let resume, tabs.indices.contains(resume) else { return 0 }
        return resume
    }
}

extension ShellWorktree {
    /// This worktree as it may be drawn while its runner is, or isn't,
    /// answering. Every tab's mark through `GlanceMark.said(answering:)`.
    ///
    /// The Changes tab too: "nothing new to read" is a claim about now as
    /// well, and an unread diff holds. `wantsAttention` is left alone, because blocked and
    /// done are the latched states and hold.
    func said(answering: Bool) -> ShellWorktree {
        guard !answering else { return self }
        var worktree = self
        worktree.tabs = tabs.map { tab in
            var tab = tab
            tab.mark = tab.mark.said(answering: false)
            return tab
        }
        return worktree
    }
}

/// A position in the fleet: which worktree, which of its tabs.
struct ShellPosition: Hashable {
    var worktree: Int
    var tab: Int

    init(worktree: Int, tab: Int) {
        self.worktree = worktree
        self.tab = tab
    }
}

/// The whole fleet, in the order the worktrees are shown.
///
/// A plain array and not a dictionary: every rule below is about adjacency,
/// and adjacency is the order. The caller owns that order — each runner's own,
/// appended a runner at a time — and it does not change under a gesture. The
/// overview groups a COPY of the indices by runner and never reorders the
/// array; a drag changes it only by way of the runner's answer.
struct ShellFleet: Hashable {
    var worktrees: [ShellWorktree]

    init(worktrees: [ShellWorktree]) {
        self.worktrees = worktrees
    }

    var isEmpty: Bool { worktrees.isEmpty }

    /// How many tabs the worktree at `index` has, or 0 for an index that is
    /// not in the fleet.
    ///
    /// Answering 0 rather than trapping because every caller here is doing
    /// arithmetic on a position that a poll could have invalidated a moment
    /// ago, and a fleet that shrinks under a finger is ordinary — see
    /// `FleetView`'s removed-worktree rule. A shell that crashed when a
    /// worktree went away mid-gesture would be a worse answer than a shell
    /// that springs back.
    func tabCount(ofWorktree index: Int) -> Int {
        guard worktrees.indices.contains(index) else { return 0 }
        return worktrees[index].tabs.count
    }

    /// Whether a position names something that exists right now.
    func contains(_ position: ShellPosition) -> Bool {
        worktrees.indices.contains(position.worktree)
            && worktrees[position.worktree].tabs.indices.contains(position.tab)
    }

    /// The tab at a position, or nil for a position the fleet no longer has.
    ///
    /// Nil rather than a trap for the reason `tabCount(ofWorktree:)` gives:
    /// a poll can shrink the fleet under a finger, and every caller here is
    /// already holding a position taken a moment ago.
    func tab(at position: ShellPosition) -> ShellTab? {
        guard contains(position) else { return nil }
        return worktrees[position.worktree].tabs[position.tab]
    }

    /// Where a tab is right now, by its id, or nil once the fleet has stopped
    /// having it.
    ///
    /// **This is what lets a mounted pane survive the fleet moving underneath
    /// it.** A pane is retained by tab ID and drawn at whatever SLOT that id
    /// currently occupies, so a worktree that gains a terminal — which
    /// renumbers every index after it — moves the panes rather than rebuilding
    /// them. An index cached at mount time would silently start naming a
    /// different tab, which is `Connection.swift:190-199`'s bug reached from
    /// the other direction: there a value changed under a stable identity,
    /// here an identity would change under a stable value.
    ///
    /// Nil is also the whole of the prune rule. A retained pane whose id no
    /// longer resolves is a pane for something the runner has forgotten, and
    /// `ShellPaneTrack` unmounts exactly those.
    ///
    /// Linear, and deliberately not an index built once: the fleet is a value
    /// that arrives whole from the daemon every poll, so an index would have
    /// to be rebuilt every poll to answer questions about a handful of
    /// retained panes.
    func position(ofTab id: String) -> ShellPosition? {
        for (w, worktree) in worktrees.enumerated() {
            if let t = worktree.tabs.firstIndex(where: { $0.id == id }) {
                return ShellPosition(worktree: w, tab: t)
            }
        }
        return nil
    }

    /// The tab to land on for a terminal, or nil when this shell has none.
    ///
    /// The ONE answer to "can the shell go to that pane", for the two things
    /// that ask it: a deep link (a tapped Live Activity) and a board card's
    /// Agent button. `tabOfTerminal` is the fleet map's own composition of
    /// each terminal's tab id (`ShellFleetMap.tabOfTerminal`), and a tab id is
    /// only an answer if this fleet has the tab: a pane that exited since the
    /// card drew is in neither, and a terminal the map knows but whose tab the
    /// shell does not draw must not resolve to a place nobody can land.
    func landing(forTerminal id: String, tabOfTerminal: [String: String]) -> String? {
        guard let tab = tabOfTerminal[id], position(ofTab: tab) != nil else { return nil }
        return tab
    }

    /// Where a shell holding `tab` sits in THIS fleet.
    ///
    /// **A `ShellPosition` is a pair of INDICES, and the fleet it indexes into
    /// is re-merged from N daemons on every poll.** `ShellFleetMap.of` walks
    /// the store's entries in runner-list order, so a second runner answering —
    /// which on a cold launch happens seconds after the first — inserts its
    /// worktrees into the middle of that array and renumbers everything after
    /// them. The value `(worktree: 0, tab: 1)` did not change; what it names
    /// did. The pane on screen becomes a different runner's worktree with no
    /// gesture, and because the VALUE held still nothing downstream can notice:
    /// `onRest` never fires, so `markVisible` goes on claiming the old pane on
    /// the old daemon and Quick Task starts work on the wrong runner.
    ///
    /// It was unreachable while the whole tree was keyed `.id(host)` — a runner
    /// arriving destroyed and rebuilt the screen, position included — and
    /// `b104e6b` removed that key on purpose, to stop a crossing throwing away
    /// every mounted pane. This is the half of that change that was missing:
    /// the position has to be anchored to something the merge cannot renumber,
    /// and a tab id is that thing. `ShellIdentity.tab` composes it out of the
    /// runner, the worktree and the pane, so it survives every reordering the
    /// merge can perform and is not reused by anything else.
    ///
    /// Falling back to a CLAMP rather than to `first` when the tab is gone.
    /// The tab going away is the ordinary shrink — a terminal exited, a
    /// worktree was removed — and the neighboring pane is where a person
    /// already was; sending them to the front of the fleet would be the app
    /// moving them somewhere they never asked to go. An empty fleet has nothing
    /// to clamp to and gets the position back unchanged, for
    /// `tabCount(ofWorktree:)`'s reason: every caller here is holding a
    /// position a poll could have invalidated a moment ago, and a fleet that
    /// shrinks under a finger is ordinary.
    ///
    /// Nil `tab` is a shell that has not come to rest on anything yet, which
    /// clamps for the same reason.
    func reseat(_ position: ShellPosition, holding tab: String?) -> ShellPosition {
        if let tab, let found = self.position(ofTab: tab) { return found }
        return clamping(position)
    }

    /// `reseat(_:holding:)`, for a fleet that arrived after `previous`.
    ///
    /// The one difference is a fleet that arrives after an EMPTY one with the
    /// anchor gone. A clamp keeps somebody near where they were, and after an
    /// empty fleet there is nowhere they were: the index is left over from a
    /// fleet that has gone entirely. That is a switch with one runner at a
    /// time — A is retired, nothing is left, then B answers — and a clamp
    /// there stood the shell on B's worktree at A's old index, which is
    /// whatever B happens to keep at that number. So it lands where a launch
    /// does instead: the first worktree, on the tab it resumes on.
    func reseat(
        _ position: ShellPosition, holding tab: String?, after previous: ShellFleet
    ) -> ShellPosition {
        if let tab, let found = self.position(ofTab: tab) { return found }
        if previous.worktrees.isEmpty, let first {
            return clamping(
                ShellPosition(
                    worktree: first.worktree, tab: worktrees[first.worktree].resumeTab))
        }
        return clamping(position)
    }

    /// The nearest position this fleet actually has, or the one handed in when
    /// it has none at all.
    func clamping(_ position: ShellPosition) -> ShellPosition {
        guard !worktrees.isEmpty else { return position }
        let worktree = min(max(position.worktree, 0), worktrees.count - 1)
        let tabs = worktrees[worktree].tabs
        guard !tabs.isEmpty else { return ShellPosition(worktree: worktree, tab: 0) }
        return ShellPosition(
            worktree: worktree, tab: min(max(position.tab, 0), tabs.count - 1))
    }

    /// The first position in the fleet, or nil for a fleet with nothing in it.
    var first: ShellPosition? {
        for (i, worktree) in worktrees.enumerated() where !worktree.tabs.isEmpty {
            return ShellPosition(worktree: i, tab: 0)
        }
        return nil
    }
}

// MARK: - The phone's stack (ov-55 4A)
//
// The iPhone opens on Needs You, and everything else is pushed over it: a
// workspace, then a task, then the worktree an agent runs in (spec §6.1). The
// shell above is still what a worktree looks like once you're in one; these are
// the rules for how you get there, kept here for this file's reason: the iOS
// target has no unit tests, and a stack that lands on the wrong screen looks
// fine in a screenshot.

/// A workspace, as the phone's stack names it: the runner it's on, and its id.
///
/// A runner without `workstreams` has one implicit workspace per repository,
/// whose id is the repository's (`WorkspaceSummary.implicit`), so this names
/// every board a phone can show.
struct PhoneWorkspace: Hashable, Codable, Sendable {
    var runner: String
    var workspace: String

    init(runner: String, workspace: String) {
        self.runner = runner
        self.workspace = workspace
    }

    /// `runner|workspace`: how `PhoneLaunch.lastWorkspaceKey` keeps it.
    var stored: String { "\(runner)|\(workspace)" }

    init?(stored: String) {
        let parts = stored.split(separator: "|", maxSplits: 1).map(String.init)
        guard parts.count == 2, !parts[0].isEmpty, !parts[1].isEmpty else { return nil }
        self.init(runner: parts[0], workspace: parts[1])
    }
}

/// Which pane a worktree screen opens on.
enum WorktreeLanding: Hashable, Codable, Sendable {
    /// The one it would open on anyway: the last chosen, else
    /// `PaneFocus.rule`'s answer.
    case resume
    /// Its Changes.
    case changes
    /// This terminal.
    case terminal(String)
}

/// One screen pushed over Needs You.
enum PhoneRoute: Hashable, Codable, Sendable {
    case workspace(PhoneWorkspace)
    /// A task on that workspace's board, by the task's id.
    case task(PhoneWorkspace, task: String)
    /// A finished status's History page on that workspace's board (ov-103):
    /// `done` or `cancelled`, the status's wire word.
    case history(PhoneWorkspace, status: String)
    /// A theme's or lane's page of the plan layer on that workspace's board
    /// (ov-274). Not reopened by a relaunch: the workspace is.
    case plan(PhoneWorkspace, page: PhonePlanPage)
    /// One level of the workspace's One tree (ov-300): the children of the
    /// node with this id (`OneTreeNode.id`, its path from the root).
    /// Not reopened by a relaunch: the workspace is.
    case tree(PhoneWorkspace, node: String)
    /// One worktree, scoped: its panes and nothing else. By the runner's id
    /// and the daemon's own worktree id.
    case worktree(runner: String, worktree: String, landing: WorktreeLanding)
    /// All Agents (ov-617): every agent at work on every runner, from the
    /// workspace list.
    case agents
}

/// What a link says a workspace should show: its conversation, or its Board
/// or Worktrees, which on an iPhone are its Plan sheet (ov-616). The words
/// are the iPad's toolbar's and its tree's.
enum WorkspaceSegment: String, CaseIterable, Hashable, Sendable {
    case orchestrator, board, worktrees, tree

    var title: String {
        switch self {
        case .orchestrator: "Orchestrator"
        case .board: "Board"
        case .worktrees: "Worktrees"
        case .tree: "Themes"
        }
    }
}

/// Where a stack of screens opens, and where a link lands.
enum PhoneLaunch {
    /// The last workspace somebody was in, as `PhoneWorkspace.stored`.
    static let lastWorkspaceKey = "workspace.last"

    /// The stack a launch opens with, over the Needs You root (ruling 4).
    ///
    /// Needs You itself when anything is in it: the front door is the inbox.
    /// Otherwise the last workspace, pushed so Back still reaches Needs You,
    /// because an empty inbox is a screen with nothing to do. Nothing is
    /// pushed for a workspace that's gone, or when none was ever opened.
    static func stack(
        itemCount: Int, last: PhoneWorkspace?, exists: (PhoneWorkspace) -> Bool
    ) -> [PhoneRoute] {
        guard itemCount == 0, let last, exists(last) else { return [] }
        return [.workspace(last)]
    }

    /// What one runner has said about what needs you, for deciding a launch.
    enum Reading: Equatable, Sendable {
        /// Still being asked.
        case waiting
        /// Answered: its own list, or one derived from an older runner's fleet.
        case read
        /// Not answering, and not about to.
        case unreachable
    }

    /// Whether a launch can decide yet: every runner has either answered or
    /// isn't going to. Deciding before would push the last workspace over
    /// an inbox a second runner was about to fill.
    static func canDecide(_ runners: [Reading]) -> Bool {
        !runners.isEmpty && !runners.contains(.waiting)
    }

    /// How long a launch waits for every runner before it gives up and
    /// stays on Needs You. A last workspace
    /// pushed a minute after launch would land over whatever somebody had
    /// started reading.
    static let decideWithin: TimeInterval = 10

    /// What a launch does now.
    enum Decision: Equatable, Sendable {
        /// Not yet: a runner is still being asked.
        case wait
        /// Nothing, for good: somebody moved first, a link is landing, or
        /// the runners took too long.
        case stay
        /// Open these screens over Needs You, and decide nothing again.
        case open([PhoneRoute])
    }

    /// Where a launch opens when there's no saved place to go back to,
    /// `elapsed` seconds after it began (ruling 4).
    ///
    /// Decided once. A stack somebody already moved, or a notification's
    /// link, wins outright. Past `decideWithin` with a runner still silent,
    /// the launch stays where it is rather than deciding late.
    ///
    /// **A stack saved by the last run isn't decided here** (ov-66's ruling 1,
    /// ov-182): it reopens where it was whatever is waiting on Needs You, as a
    /// `Destination` held for its runner by `DestinationResolver`, which
    /// opens the nearest level that's still there when the screen it names is
    /// gone. An empty saved stack was Needs You itself, and the rule above
    /// decides.
    static func decide(
        _ runners: [Reading], elapsed: TimeInterval, moved: Bool, linking: Bool,
        itemCount: Int, last: PhoneWorkspace?, exists: (PhoneWorkspace) -> Bool
    ) -> Decision {
        if moved || linking { return .stay }
        let late: Decision = elapsed >= decideWithin ? .stay : .wait
        guard canDecide(runners) else { return late }
        return .open(stack(itemCount: itemCount, last: last, exists: exists))
    }

    /// Where the stack is kept between runs, as `encode` writes it.
    static let stackKey = "phone.stack"

    /// A stack as `stackKey` keeps it: the pushed screens, then the worktree
    /// over them when one is open.
    static func encode(_ stack: [PhoneRoute]) -> Data? {
        try? JSONEncoder().encode(stack)
    }

    /// A kept stack, or none at all: one written by a build whose routes
    /// this one can't read is not a partial place to reopen.
    static func decode(_ data: Data?) -> [PhoneRoute] {
        guard let data, let stack = try? JSONDecoder().decode([PhoneRoute].self, from: data)
        else { return [] }
        return stack
    }
}

/// A deep link, resolved: the screens to push, and the segment the
/// workspace screen should show when it's the top of them.
struct PhoneLink: Equatable, Sendable {
    var stack: [PhoneRoute]
    var segment: WorkspaceSegment?
}

extension Fleet {
    /// The workspace a terminal is counted under: its own, else its
    /// worktree's owner, else, on a runner without workspaces, its
    /// repository's implicit one. Nil for an unclaimed worktree's shell.
    func workspace(of terminal: Terminal, in worktree: Worktree) -> String? {
        if let own = terminal.workspace, !own.isEmpty { return own }
        if let owner = worktree.workspace, !owner.isEmpty { return owner }
        if workspaces == nil { return worktree.repository }
        return nil
    }

    /// Where `farcooler://terminal/<id>` lands on a phone (spec §6.1): its
    /// workspace, then its task when the pane has one, then the pane, so
    /// Back walks up that chain. An orchestrator's pane is its workspace's
    /// Orchestrator segment. Nil when this runner has no such terminal.
    ///
    /// The task is `TaskLink`'s: the one the pane's header names.
    func phoneLink(toTerminal id: String, runner: String) -> PhoneLink? {
        for worktree in worktrees {
            guard let terminal = worktree.terminals.first(where: { $0.id == id }) else { continue }
            let pane = PhoneRoute.worktree(
                runner: runner, worktree: worktree.id, landing: .terminal(id))
            guard let workspace = workspace(of: terminal, in: worktree) else {
                return PhoneLink(stack: [pane], segment: nil)
            }
            let place = PhoneWorkspace(runner: runner, workspace: workspace)
            if terminal.isOrchestrator {
                return PhoneLink(stack: [.workspace(place)], segment: .orchestrator)
            }
            var stack: [PhoneRoute] = [.workspace(place)]
            if let task = TaskLink.task(of: terminal, in: worktree) {
                stack.append(.task(place, task: task))
            }
            stack.append(pane)
            return PhoneLink(stack: stack, segment: nil)
        }
        return nil
    }
}

/// A tapped decision push: the task's key, and the runner it's on when the
/// push says.
///
/// The relay forwards `runner` beside `task` (ov-72): the runner's
/// `Host.runner_id`, the id its daemon derives from its install id and
/// tells every client, lowercased. It is NOT this app's own id for the
/// runner (`Host.id` on the phone, a pairing-local UUID the daemon and relay
/// never see), so it's matched against `PhoneDecisionLink.Source.hostRunner`.
/// A task key is only unique on its own runner's workspace (`bil-7` can be
/// on two runners at once), so with no runner in the push, which is every
/// push from a daemon older than the field, `PhoneDecisionLink.find`
/// refuses to guess between two.
struct DecisionPush: Equatable, Sendable {
    var key: String
    var runner: String?
}

/// Where a decision push lands: its workspace, then its task, with the
/// question on it (ruling 3), as Android's `DecisionLink` finds it.
enum PhoneDecisionLink {
    /// One runner, as the search reads it.
    struct Source {
        var runner: String
        /// Its Needs You list.
        var items: [NeedsYouItem]
        /// Its boards read so far, by workspace id.
        var boards: [String: TaskBoardModel]
        /// Whether it has no workspaces, so an item's board is its
        /// repository's implicit one.
        var implicit: Bool
        /// Its `Host.runner_id`, which a push names it by; nil until the
        /// phone has read it, and from a runner too old to say.
        var hostRunner: String?
    }

    /// The stack for the task `push` names, or nil until one runner has it.
    ///
    /// On the runner the push names, when it names one, by its
    /// `hostRunner`: a runner whose id isn't known yet isn't it, until
    /// `waitEnded`, when a name nobody answers to is dropped and the key
    /// is searched for alone. Otherwise on the one
    /// runner that has a task under that key: its Needs You item first,
    /// which knows its workspace, then any board read so far with a card
    /// under the key. Two runners with a task under one key is nil, not the
    /// first of them: a push landing on another runner's task is worse than
    /// Needs You, where both are.
    static func find(
        _ push: DecisionPush, in sources: [Source], waitEnded: Bool = false
    ) -> [PhoneRoute]? {
        guard !push.key.isEmpty else { return nil }
        var named = sources
        if let runner = push.runner {
            let matching = sources.filter { $0.hostRunner?.lowercased() == runner.lowercased() }
            // Known: only its word counts. Unknown, once the wait is over:
            // the key alone, as for a push naming nobody.
            if !matching.isEmpty || !waitEnded { named = matching }
        }
        let found = named.compactMap { stack(for: push.key, on: $0) }
        return found.count == 1 ? found[0] : nil
    }

    /// The task under `key` on one runner.
    private static func stack(for key: String, on source: Source) -> [PhoneRoute]? {
        for item in source.items where item.task?.key == key {
            guard let task = item.task,
                let workspace = item.workspaceID ?? (source.implicit ? item.repositoryID : nil)
            else { continue }
            let place = PhoneWorkspace(runner: source.runner, workspace: workspace)
            return [.workspace(place), .task(place, task: task.id)]
        }
        for workspace in source.boards.keys.sorted() {
            guard let row = source.boards[workspace]?.rows.first(where: { $0.key == key })
            else { continue }
            let place = PhoneWorkspace(runner: source.runner, workspace: workspace)
            return [.workspace(place), .task(place, task: row.id)]
        }
        return nil
    }
}

/// The one task write a phone makes: an answer to an agent's question.
///
/// The orchestrator owns the task list (ov-184). A phone reads the board and
/// answers decisions; it never creates, edits, re-statuses or deletes a task.
/// The client core refuses any `task.note` that isn't an answer
/// (`crates/client/src/ffi.rs`, `task_note_of`) and has no arm for
/// `task.create`, `task.update` or `task.set_status`. Android's `TaskAnswer`,
/// word for word.
enum PhoneTaskAnswer {
    /// The wire method an answer goes out on.
    static let method = "task.note"

    /// `task.note`'s arguments for `body` as the answer to `task`'s question.
    /// The runner writes it as the person, takes the decision off Needs You,
    /// and wakes the agent waiting on it.
    static func request(task: String, body: String) -> [String: String] {
        ["task": task, "kind": "answer", "body": body]
    }
}

/// What a Needs You row says when an answer from it didn't land (spec §2.5).
enum PhoneAnswer {
    /// The sentence for a refusal: by the runner's `what` where it named
    /// one, else by its code. `agent` is who the answer was for (`claude`).
    ///
    /// `not_held` is an ask nothing holds any more: someone got there first.
    /// `not_delivered` is one still held that the agent didn't take, so
    /// trying again can work. Both are one code on the wire, which is why
    /// the code alone can't choose between them.
    static func refusal(word: String?, what: String?, agent: String) -> String {
        switch what {
        case "not_held": return "Someone already answered this."
        case "not_delivered": return "Couldn’t reach \(agent). Try again."
        // A folder-trust prompt's (ov-493): `unconfirmed` comes after the
        // Enter was sent, so it says so.
        case "too_soon": return "An answer was just sent. Check the terminal before answering again."
        case "unconfirmed": return "Sent, but \(agent) hasn’t moved on yet. Check the terminal before answering again."
        case "not_on_yes": return "The prompt’s cursor isn’t on Yes. Answer it in the terminal."
        default: break
        }
        guard let word, !word.isEmpty else {
            return "Your answer may not have reached the runner. Try again."
        }
        return RunnerRefusal.trouble(
            forWord: word, message: "", after: "The runner didn’t take your answer."
        ).sentence
    }
}

// MARK: - Needs You's Workspaces list

/// One repository's rows in Needs You's Workspaces list, on one runner: its
/// workspaces, then the worktrees none of them owns, then the hidden ones.
struct PhoneRepositorySection: Identifiable, Equatable, Sendable {
    var runner: String
    var repository: String
    var workspaces: [PhoneWorkspaceRow]
    /// Worktrees no workspace owns, shown and not hidden, in runner order.
    var unclaimed: [String]
    /// Items counted under no workspace in this repository: the Unclaimed
    /// group's count.
    var unclaimedCount: Int
    /// Hidden worktrees, whoever owns them.
    var hidden: [String]

    var id: String { "\(runner)/\(repository)" }
}

/// One workspace's row: its name, its orchestrator, and what needs you in it.
struct PhoneWorkspaceRow: Identifiable, Equatable, Sendable {
    var place: PhoneWorkspace
    var name: String
    /// A repository's one board on a runner without workspaces.
    var isImplicit: Bool
    /// The live orchestrator's terminal id, or nil for none.
    var orchestrator: String?
    /// Its items: the workspace's needs-you count.
    var count: Int
    /// The orchestrator finished a turn nobody has looked at yet: a dot, not
    /// an item (ruling 10).
    var unread: Bool

    var id: String { place.stored }
}

extension Fleet {
    /// Needs You's Workspaces list for this runner's fleet (spec §6.1).
    ///
    /// Every workspace is a row, empty or not, and so is a repository's
    /// implicit one on a runner without workspaces: an empty board has to be
    /// reachable (spec §8). `names` are the repositories' display names, by
    /// id, for the implicit rows. `items` are this runner's.
    ///
    /// An item on a runner without workspaces names no workspace, so it's
    /// counted under its repository's implicit one, which is where its
    /// worktrees are.
    func phoneSections(
        runner: String, names: [String: String], items: [NeedsYouItem]
    ) -> [PhoneRepositorySection] {
        let terminals = Dictionary(
            worktrees.flatMap(\.terminals).map { ($0.id, $0) }, uniquingKeysWith: { a, _ in a })
        let hiddenIDs = Set(worktrees.filter(\.isHidden).map(\.id))
        return repositoryGroups().map { group in
            let rows = group.workspaces.map { owned in
                let workspace = owned.workspace
                let count = items.filter { item in
                    if workspace.isImplicit {
                        return item.workspaceID == workspace.id
                            || (item.workspaceID == nil && item.repositoryID == group.repository)
                    }
                    return item.workspaceID == workspace.id
                }.count
                let orchestrator = owned.orchestrator.flatMap { terminals[$0] }
                return PhoneWorkspaceRow(
                    place: PhoneWorkspace(runner: runner, workspace: workspace.id),
                    name: workspace.isImplicit
                        ? (names[group.repository] ?? workspace.name) : workspace.name,
                    isImplicit: workspace.isImplicit,
                    orchestrator: orchestrator?.id,
                    count: count,
                    unread: orchestrator?.agent == .done)
            }
            let implicit = group.workspaces.contains { $0.workspace.isImplicit }
            return PhoneRepositorySection(
                runner: runner,
                repository: group.repository,
                workspaces: rows,
                unclaimed: group.unclaimed.filter { !hiddenIDs.contains($0) },
                unclaimedCount: implicit
                    ? 0 : items.unclaimedCount(inRepository: group.repository),
                hidden: worktrees.filter {
                    $0.isHidden && ($0.repository ?? "") == group.repository
                }.map(\.id))
        }
    }
}

/// What Needs You says under "Nothing needs you" when it can't be sure.
public enum PhoneInbox {
    /// Every item Needs You shows, across runners: each runner's own list
    /// where it has one, and for a runner whose list isn't read yet (still
    /// coming, or the read failed), the blocked agents its fleet shows, as
    /// derived items.
    ///
    /// The glances' rule (`FleetPublication.merged`), so the app and the
    /// widget, the watch and the Live Activity count the same thing while a
    /// list is on its way, instead of the app saying nothing and the widget
    /// counting that runner's blocked agents.
    ///
    /// `unread` is by runner id, each runner's panes (`Fleet.olderPanes`).
    public static func shown(
        lists: [String: [NeedsYouItem]], unread: [String: [NeedsYou.OlderPane]]
    ) -> [NeedsYouItem] {
        var all = lists
        for (runner, panes) in unread where all[runner] == nil {
            all[runner] = NeedsYou.derived(fromTerminals: panes)
        }
        return NeedsYou.merge(all)
    }

    /// Nil when every runner answered. Otherwise names the ones that didn't,
    /// since their items aren't in the list: "Nothing needs you" about a
    /// runner nobody could ask would be a claim nobody made.
    public static func caveat(unanswered: [String]) -> String? {
        switch unanswered.count {
        case 0: return nil
        case 1: return "\(unanswered[0]) isn’t answering, so this may not be everything."
        default:
            return "\(unanswered.count) runners aren’t answering, so this may not be everything."
        }
    }
}

extension DaemonBuild {
    /// Whether this phone may act on the runner, not only read it: answer an
    /// ask or a decision, or start, restart or replace an orchestrator. A
    /// Read grant sees the items and the panes and does none of these (spec
    /// §2.5), as Android decides (`grantedScope != "read"`). A runner that
    /// hasn't said is not "read", so its phone keeps what it offered before.
    var mayAct: Bool { grantedScope != "read" }
}
