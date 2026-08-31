import Foundation

// The navigation shell's arithmetic, with no screen attached.
//
// Everything here is a value type or a pure function, and that is not a style
// preference — it is where the tests can reach. The iOS app target has no unit
// test bundle at all (`generate-project.py`'s `UI_TEST_SOURCES` is the only
// test target it generates, and a UI test needs a booted simulator), so a
// threshold that lives in a `View` can only ever be checked by a person
// swiping at it. The rules below are the ones that are wrong in ways a
// screenshot cannot show: stepping off the end of a workspace, the axis lock,
// which end of the fleet rubber-bands, and the order the overview sorts in.
//
// So the gesture state machine transcribed in
// `.claude/agent/briefs/ios-shell-mechanics.md` is split in two. The numbers
// and the decisions are here and are covered by `swift test`; the views under
// `apps/ios/FarCooler/Shell*.swift` do nothing but read a finger and draw the
// answer.
//
// Internal, like everything in `CoreModel.swift` and for the same reason its
// header gives: the phone compiles AgentKit's sources straight into the app
// module, while the Mac depends on AgentKit as a real module — so the Mac
// cannot see one name from this file, and this is a shared PACKAGE rather than
// shared CODE. The Mac's window chrome is not this shell and must not come to
// be written against it by accident.

/// The prototype's constants, transcribed.
///
/// Named after what they measure rather than after the CSS they came out of,
/// but the numbers themselves are verbatim from the mechanics doc and are not
/// to be tuned here. A number that wants to change is a change to the design,
/// and the design is a prototype somebody built and looked at.
enum ShellMetrics {
    /// One tab row in the column, and therefore the travel that reveals one.
    /// The two are the same number on purpose: the row you are selecting is
    /// under your finger, not a row ahead of it or behind it.
    /// 44, not the prototype's 34, and the constant is doing two jobs so both
    /// move together: it is the row's HEIGHT and it is how much lift reveals
    /// one row's worth of selection. A visual row taller than the mapping
    /// would put the highlight somewhere other than under the thumb.
    ///
    /// 44 is the platform's own row height — a menu row, a table row, the
    /// minimum comfortable target. The owner's note that these numbers are "a
    /// tuned starting point, not gospel" is what makes this movable, and
    /// looking native was worth more than the four points.
    static let rowHeight: CGFloat = 44

    /// The bar itself, and one item of its rail. The same 44 the rest of this
    /// app calls `PaneMetrics.target`, arrived at from the other side — the
    /// bar is a thing you hit as well as a thing you read.
    static let barRow: CGFloat = 44

    /// Below this much lift, a release abandons and costs nothing.
    ///
    /// It is deliberately far below `rowHeight`: the gap between "I touched
    /// the bar and moved a little" and "I am choosing the first tab" has to be
    /// crossable by accident in neither direction.
    static let openMin: CGFloat = 16

    /// Travel PAST the last row before the overview arrives.
    ///
    /// Past the last row and not from the bar: the column has to be fully
    /// unfurled first, so a fleet of two tabs and a fleet of nine both reach
    /// the overview by "keep going after there is nothing left to reveal"
    /// rather than at some absolute distance that means different things to
    /// the two of them.
    static let overRun: CGFloat = 76

    /// Horizontal distance that commits a page turn.
    static let pageCommit: CGFloat = 70

    /// The device width the prototype was built at, and one content pane.
    ///
    /// The views take their real width from the geometry they are handed and
    /// pass it in; this is what the numbers above were chosen against and what
    /// the tests measure with, so that a test asserting on a commit target is
    /// asserting on the prototype's own arithmetic.
    static let pageWidth: CGFloat = 393

    /// The bar's inset from each side of the page.
    ///
    /// 16, not the 10 that `PaneMetrics.surfaceInset` gives every other
    /// floating surface in this app. The mechanics doc derives `RAIL_W 361`
    /// from `393 - 2*16` and the prototype is the authority on this shell's
    /// own metrics; the day the shell replaces the tab strip is the day the
    /// two insets have to be reconciled, and reconciling them now would mean
    /// changing a number nobody has looked at on a screen.
    static let barInset: CGFloat = 16

    /// First movement past this decides the axis, once, for the whole gesture.
    static let axisLock: CGFloat = 6

    /// What a drag is multiplied by when there is nothing to bring in.
    static let rubberBand: CGFloat = 0.34

    /// One rail item, for a page of the given width.
    ///
    /// A function of the page rather than a constant so the rail tracks the
    /// content proportionally on a device that is not 393 wide. At 393 this is
    /// the doc's `RAIL_W 361` exactly.
    static func railWidth(page: CGFloat = pageWidth) -> CGFloat { page - 2 * barInset }
}

/// What one tab's mark says it is doing.
///
/// Four states and no fifth. Three of them are the daemon's own answer about a
/// terminal — the phone never computes one, see `FleetView.swift:168-171` —
/// and `stale` is the AGE of that answer rather than a competing claim about
/// the terminal, which is the argument the mechanics doc makes at length.
enum ShellMark: Hashable {
    /// An agent is waiting on a person. The one thing amber means in this app.
    case needsYou

    /// A diff nobody has read yet.
    ///
    /// **Only a Diff tab may ever be in this state.** It comes from
    /// `InboxRow`, which counts a WORKSPACE's changed lines and knows nothing
    /// about any agent — `NeedsYou.swift:94-97` refuses to invent a per-agent
    /// version of it for exactly that reason. Whatever maps a real fleet into
    /// a `ShellFleet` owes this: putting it on an agent tab would draw a cyan
    /// ring that means "this agent has unread changes", which is not a fact
    /// anything on the wire has an opinion about.
    case unreadDiff

    /// The agent is working. Nothing is being asked of anybody.
    case working

    /// The daemon's answer about this tab is old.
    ///
    /// Not "less important" — less KNOWN. Drawn under whatever the daemon
    /// already said rather than instead of it, which is why this is a state
    /// here but a dashed ring rather than a different colour on screen.
    case stale
}

/// Where a workspace sorts in the overview, and nowhere else.
///
/// The mechanics doc's `needsYou > unreadDiff > (all stale) > working`, as a
/// number so it can be sorted on. `all stale` is the odd one because it is the
/// only rank that is a property of the whole workspace rather than of some tab
/// in it: one stale tab beside a working one is a workspace being worked in,
/// and only a workspace where nothing has been heard from at all is a
/// workspace that has gone quiet.
enum ShellPrecedence: Int, Comparable {
    case needsYou = 0
    case unreadDiff = 1
    case allStale = 2
    case working = 3

    static func < (a: ShellPrecedence, b: ShellPrecedence) -> Bool {
        a.rawValue < b.rawValue
    }
}

/// One tab, as the shell needs it: something to name and something to draw.
///
/// Deliberately not a `Terminal`. The shell shows a workspace's tabs, and a
/// workspace's tabs are its terminals AND its diff — see `TerminalTabStrip`'s
/// `ChangesChip`, which is a tab with no terminal behind it and never can
/// have one. A shape that could only hold terminals would have to special-case
/// the diff at every ribbon, column row and overview card.
struct ShellTab: Identifiable, Hashable {
    var id: String
    /// What the column row says. Short: the column is one bar wide.
    var title: String
    var mark: ShellMark

    init(id: String, title: String, mark: ShellMark) {
        self.id = id
        self.title = title
        self.mark = mark
    }
}

/// One workspace: a name, and the tabs in their fixed order.
///
/// **The order is fixed and is never re-sorted by activity.** The ribbon under
/// the workspace name is a map of the workspace, and a map whose landmarks
/// move when something happens is not a map — you would have to read it every
/// time instead of remembering it. Precedence sorts WORKSPACES in the
/// overview; it never sorts tabs.
struct ShellWorkspace: Identifiable, Hashable {
    var id: String
    var name: String
    var tabs: [ShellTab]
    /// The runner this workspace lives on, and only when it is worth saying.
    ///
    /// Nil for the local machine. The bar shows it after the name so a fleet
    /// spread across several runners can be told apart at a glance, and shows
    /// nothing at all for the common case — a name that is always there stops
    /// being read, and this bar has one job before it has two.
    var server: String?
    /// The last few lines of this workspace's most recent terminal.
    ///
    /// The overview card's reason to exist. A grid of forty names says which
    /// workspaces you have; a grid of forty names each showing what its agent
    /// last said is the thing you actually scan. Empty is fine and draws
    /// nothing — a workspace whose terminal has said nothing has nothing to
    /// show, and a placeholder there would be forty lies.
    var tail: [String]
    /// Which tab this workspace should be REOPENED on, as an index into
    /// `tabs`.
    ///
    /// "Reopened" is the whole of the distinction, and it is why this is on the
    /// workspace rather than being a rule inside `step`. There are two ways to
    /// arrive at another workspace and they are not the same journey:
    ///
    /// - **The content swipe walks a continuum.** One flat sequence of tabs
    ///   through the whole fleet, spilling into the next workspace's nearest
    ///   tab — its first going forward, its last coming back. That has to stay
    ///   literal: a sequence that skipped to a remembered tab would not be a
    ///   sequence, it would drift as you swiped back and forth, and swiping
    ///   right then left would land you somewhere you had not been. See
    ///   `contentStep`, which does not read this.
    /// - **The bar swipe, the carried lift and an overview card are all
    ///   GOING somewhere**, by name, deliberately. Going back to a workspace
    ///   should put you where you were in it, which is the same F4 argument
    ///   `docs/jobs-to-be-done.md` makes about the app being put down every
    ///   ninety seconds — landing on tab 0 is the app half-remembering.
    ///
    /// Resolved by whoever builds the fleet, from the ONE memory the app
    /// already keeps: `Connection.lastFocus`, written only when a person
    /// chooses a tab. A second memory living in here would be a second thing
    /// to disagree with it. Nil means nobody has ever chosen a tab in this
    /// workspace, and nil resolves to the first tab, which is where a
    /// workspace nobody has an opinion about opens.
    ///
    /// An index rather than a tab id because that is what a `ShellPosition`
    /// is, and because the caller has just built `tabs` and is the only thing
    /// that can turn a remembered id into one honestly. A remembered pane that
    /// has since gone must not resolve to "index 0 by accident"; see
    /// `Route.Focus.rule(for:inbox:)`, which is what the caller falls back to.
    var resume: Int?

    init(
        id: String, name: String, server: String? = nil,
        tail: [String] = [], resume: Int? = nil, tabs: [ShellTab]
    ) {
        self.id = id
        self.name = name
        self.tabs = tabs
        self.server = server
        self.tail = tail
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

    /// Where this workspace sorts in the overview.
    ///
    /// An empty workspace ranks as `working`, which is the "nothing to say"
    /// rank: it is not waiting on anybody, has no diff to read, and has no
    /// tabs that could have gone quiet.
    var precedence: ShellPrecedence {
        if tabs.contains(where: { $0.mark == .needsYou }) { return .needsYou }
        if tabs.contains(where: { $0.mark == .unreadDiff }) { return .unreadDiff }
        if !tabs.isEmpty && tabs.allSatisfy({ $0.mark == .stale }) { return .allStale }
        return .working
    }
}

/// Which way a step goes along the flat sequence.
enum ShellDirection: Hashable {
    case previous
    case next

    /// The sign a drag of this direction moves the track by.
    ///
    /// Going to the NEXT pane moves the track LEFT, because the next pane is
    /// the one waiting off the right edge. Written down once rather than as a
    /// `-1` in three call sites, which is where a sign error hides.
    var trackSign: CGFloat { self == .next ? -1 : 1 }
}

/// A position in the fleet: which workspace, which of its tabs.
struct ShellPosition: Hashable {
    var workspace: Int
    var tab: Int

    init(workspace: Int, tab: Int) {
        self.workspace = workspace
        self.tab = tab
    }
}

/// Where a step lands, and whether taking it leaves the workspace.
///
/// The flag is not a convenience. It is what the incoming pane's title uses to
/// name the other workspace before you commit — the crossing has to be visible
/// while it is still abandonable — and it is what decides whether the BAR
/// translates: within a workspace the bar holds still, because the workspace
/// is not changing.
struct ShellStep: Hashable {
    var position: ShellPosition
    var crossesWorkspace: Bool
}

/// Which gesture is driving the track, which decides what its neighbours are.
///
/// The same three-pane track serves both, and the only difference is what
/// `previous` and `next` mean. From the bar they mean the adjacent WORKSPACE,
/// opened at its first tab, because the bar is the workspace. From the content
/// they mean the adjacent TAB along one flat sequence that runs through the
/// whole fleet, so a swipe never dead-ends at a workspace boundary.
enum ShellTrack: Hashable {
    case bar
    case content
}

/// The whole fleet, in the order the workspaces are shown.
///
/// A plain array and not a dictionary: every rule below is about adjacency,
/// and adjacency is the order. The caller owns that order and it does not
/// change under a gesture — the overview sorts a COPY of the indices.
struct ShellFleet: Hashable {
    var workspaces: [ShellWorkspace]

    init(workspaces: [ShellWorkspace]) {
        self.workspaces = workspaces
    }

    var isEmpty: Bool { workspaces.isEmpty }

    /// How many tabs the workspace at `index` has, or 0 for an index that is
    /// not in the fleet.
    ///
    /// Answering 0 rather than trapping because every caller here is doing
    /// arithmetic on a position that a poll could have invalidated a moment
    /// ago, and a fleet that shrinks under a finger is ordinary — see
    /// `FleetView`'s removed-workspace rule. A shell that crashed when a
    /// workspace went away mid-gesture would be a worse answer than a shell
    /// that springs back.
    func tabCount(ofWorkspace index: Int) -> Int {
        guard workspaces.indices.contains(index) else { return 0 }
        return workspaces[index].tabs.count
    }

    /// Whether a position names something that exists right now.
    func contains(_ position: ShellPosition) -> Bool {
        workspaces.indices.contains(position.workspace)
            && workspaces[position.workspace].tabs.indices.contains(position.tab)
    }

    /// The tab at a position, or nil for a position the fleet no longer has.
    ///
    /// Nil rather than a trap for the reason `tabCount(ofWorkspace:)` gives:
    /// a poll can shrink the fleet under a finger, and every caller here is
    /// already holding a position taken a moment ago.
    func tab(at position: ShellPosition) -> ShellTab? {
        guard contains(position) else { return nil }
        return workspaces[position.workspace].tabs[position.tab]
    }

    /// Where a tab is right now, by its id, or nil once the fleet has stopped
    /// having it.
    ///
    /// **This is what lets a mounted pane survive the fleet moving underneath
    /// it.** A pane is retained by tab ID and drawn at whatever SLOT that id
    /// currently occupies, so a workspace that gains a terminal — which
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
        for (w, workspace) in workspaces.enumerated() {
            if let t = workspace.tabs.firstIndex(where: { $0.id == id }) {
                return ShellPosition(workspace: w, tab: t)
            }
        }
        return nil
    }

    /// The first position in the fleet, or nil for a fleet with nothing in it.
    var first: ShellPosition? {
        for (i, workspace) in workspaces.enumerated() where !workspace.tabs.isEmpty {
            return ShellPosition(workspace: i, tab: 0)
        }
        return nil
    }

    /// One step along the sequence `track` walks, or nil when there is
    /// genuinely nothing there.
    ///
    /// Nil is the interesting answer and is the ONLY thing that rubber-bands.
    /// From the content that means the two true ends of the whole fleet: the
    /// very first tab of the first workspace and the very last tab of the
    /// last, and nowhere in between — walking off the end of a workspace lands
    /// on its neighbour, so a workspace boundary is not an end.
    ///
    /// Workspaces with NO tabs are skipped rather than landed on. An empty
    /// workspace is reachable from the bar and from the overview, which is
    /// where a person goes deliberately; stepping through the content into one
    /// would be a swipe that lands on nothing and then cannot be swiped out of
    /// in the same direction.
    func step(
        from position: ShellPosition, _ direction: ShellDirection, along track: ShellTrack
    ) -> ShellStep? {
        guard workspaces.indices.contains(position.workspace) else { return nil }
        switch track {
        case .bar:
            return barStep(from: position.workspace, direction)
        case .content:
            return contentStep(from: position, direction)
        }
    }

    /// The adjacent workspace, on the tab you last had open there. Always a
    /// crossing — that is what the bar's gesture is for.
    ///
    /// `resumeTab` and not 0, and that is the difference between this and
    /// `contentStep`. The bar is the workspace, so swiping it is going to
    /// another workspace by name; arriving somewhere you have been before and
    /// finding the tab you left is what makes the fleet a place rather than a
    /// list you re-navigate every time. The content swipe is a continuum and
    /// stays literal — see `ShellWorkspace.resume`, which sets out both halves.
    private func barStep(from workspace: Int, _ direction: ShellDirection) -> ShellStep? {
        let next = direction == .next ? workspace + 1 : workspace - 1
        guard workspaces.indices.contains(next) else { return nil }
        return ShellStep(
            position: ShellPosition(workspace: next, tab: workspaces[next].resumeTab),
            crossesWorkspace: true)
    }

    /// The next tab along, over the whole fleet, crossing workspaces.
    private func contentStep(from position: ShellPosition, _ direction: ShellDirection)
        -> ShellStep?
    {
        let tabs = workspaces[position.workspace].tabs
        switch direction {
        case .next:
            if tabs.indices.contains(position.tab + 1) {
                return ShellStep(
                    position: ShellPosition(workspace: position.workspace, tab: position.tab + 1),
                    crossesWorkspace: false)
            }
            var w = position.workspace + 1
            while workspaces.indices.contains(w) {
                if !workspaces[w].tabs.isEmpty {
                    return ShellStep(
                        position: ShellPosition(workspace: w, tab: 0), crossesWorkspace: true)
                }
                w += 1
            }
            return nil
        case .previous:
            if position.tab - 1 >= 0, tabs.indices.contains(position.tab - 1) {
                return ShellStep(
                    position: ShellPosition(workspace: position.workspace, tab: position.tab - 1),
                    crossesWorkspace: false)
            }
            var w = position.workspace - 1
            while w >= 0 {
                let count = workspaces[w].tabs.count
                if count > 0 {
                    // The PREVIOUS workspace's LAST tab, which is the half of
                    // this rule that is easy to get wrong: a sequence that
                    // stepped backward onto tab 0 would not be a sequence, it
                    // would be two different orders depending on which way you
                    // walked, and swiping back and forth would drift.
                    return ShellStep(
                        position: ShellPosition(workspace: w, tab: count - 1),
                        crossesWorkspace: true)
                }
                w -= 1
            }
            return nil
        }
    }

    /// Whether a drag in this direction has nothing to bring in, and should
    /// therefore be rubber-banded.
    ///
    /// Defined as "the step returned nothing" and not as "we are at a
    /// workspace boundary", because those are different places and confusing
    /// them is the bug: a rubber band at every boundary would make the fleet
    /// feel like a list of separate lists, which is the thing the flat
    /// sequence exists to stop it being.
    func rubberBands(at position: ShellPosition, _ direction: ShellDirection, along track: ShellTrack)
        -> Bool
    {
        step(from: position, direction, along: track) == nil
    }
}

// MARK: - The gesture

/// The one axis a gesture is allowed to have.
enum ShellAxis: Hashable {
    case horizontal
    case vertical
}

/// The arithmetic between a finger and the shell.
///
/// Static functions rather than methods on a state object, because none of
/// this has any state: the view holds the drag channel and asks these
/// questions of it. That is also what makes them testable without a screen.
enum ShellGesture {
    /// Which axis this gesture is, or nil while it is still too small to say.
    ///
    /// **Decided once and never revisited.** The caller stores the first
    /// non-nil answer and keeps asking nothing further for the rest of the
    /// gesture; an axis that could change mid-drag means a swipe that starts
    /// sideways and ends up opening the column, which is every accidental
    /// gesture in the design review.
    ///
    /// `dy` is measured UP-positive — `startY - y` — which is why the
    /// comparison is `abs(dx) > dy` and not `abs(dx) > abs(dy)`. That is
    /// verbatim from the prototype and it is right: a DOWNWARD drag has no
    /// meaning on this bar (only up unfurls the column), so a downward flick
    /// gives a negative `dy`, loses to any `abs(dx)` at all, and is called
    /// horizontal — where its near-zero `dx` springs it straight back. The
    /// alternative, calling it vertical, would be a gesture that is locked to
    /// an axis on which it can do nothing.
    static func axis(dx: CGFloat, up dy: CGFloat) -> ShellAxis? {
        guard max(abs(dx), abs(dy)) > ShellMetrics.axisLock else { return nil }
        return abs(dx) > dy ? .horizontal : .vertical
    }

    /// How far the track actually moves for a drag of `dx`.
    ///
    /// One-to-one with the finger, except where there is nothing to bring in.
    static func translation(dx: CGFloat, rubberBanding: Bool) -> CGFloat {
        rubberBanding ? dx * ShellMetrics.rubberBand : dx
    }

    /// Which direction a horizontal drag of `dx` is asking for.
    ///
    /// Sign only — this says nothing about whether the drag is big enough to
    /// commit. A drag of exactly zero has no direction, which is why this is
    /// optional rather than defaulting one way.
    static func direction(dx: CGFloat) -> ShellDirection? {
        if dx < 0 { return .next }
        if dx > 0 { return .previous }
        return nil
    }

    /// Whether `dx` has travelled far enough to commit a page turn.
    static func commits(dx: CGFloat) -> Bool { abs(dx) >= ShellMetrics.pageCommit }

    /// How many column rows a lift of `up` has revealed.
    static func columnSteps(up: CGFloat, tabCount: Int) -> Int {
        guard tabCount > 0, up > 0 else { return 0 }
        return min(tabCount, Int((up / ShellMetrics.rowHeight).rounded(.up)))
    }

    /// Which row a lift of `up` is selecting, or nil when it is selecting none.
    ///
    /// Nil below `openMin` and not "row 0": the difference between them is
    /// whether letting go costs you the tab you were on, and a bar that
    /// switched tabs on a 10-point twitch would make the bar untouchable.
    ///
    /// **Counted DOWN from the last tab, because the menu reads top to
    /// bottom.** The column lists tab 0 at the TOP — the same order the
    /// ribbon draws its marks in, leftmost mark to topmost row — so the row
    /// nearest the bar, which is the one the first `rowHeight` of lift puts
    /// under the thumb, is the LAST tab. Every further row of lift walks one
    /// step UPWARD through the list toward tab 0, and a lift long enough to
    /// fill the column selects tab 0.
    ///
    /// It used to be `steps - 1`, against a column drawn bottom-up. Both were
    /// self-consistent and the pair of them was wrong in the way that matters:
    /// a menu whose first item is at the bottom is a menu you read backwards,
    /// and it disagreed with the ribbon two points below it about which end of
    /// a workspace tab 0 lives at. Fixing the order therefore had to invert
    /// this — the mapping and the drawing are one decision, and this is the
    /// half of it a test can hold.
    static func columnSelection(up: CGFloat, tabCount: Int) -> Int? {
        guard up >= ShellMetrics.openMin else { return nil }
        let steps = columnSteps(up: up, tabCount: tabCount)
        return steps > 0 ? tabCount - steps : nil
    }

    /// The lift at which the column has nothing left to reveal.
    ///
    /// The join. Everything about this gesture is measured from it rather than
    /// from an absolute distance — the column below it, the overview above it,
    /// and the page's own travel — so a workspace with one tab and a workspace
    /// with nine both reach the same places by "keep going until there is
    /// nothing left, then keep going".
    static func columnFull(tabCount: Int) -> CGFloat {
        CGFloat(tabCount) * ShellMetrics.rowHeight
    }

    /// The column's height right now.
    ///
    /// `pinned` is a SEPARATE input from `up` and that separation is the whole
    /// point: the prototype keeps `colOpen` distinct from `dragY` because
    /// deriving the column's visibility from both of them produced a tap that
    /// toggled the wrong way. One source of truth per thing — a tap owns
    /// `pinned`, a drag owns `up`, and this function is the only place they
    /// meet.
    static func columnHeight(up: CGFloat, tabCount: Int, pinned: Bool) -> CGFloat {
        let full = columnFull(tabCount: tabCount)
        // Whole, or nothing. The prototype tied the height to the lift —
        // `min(up, full)` — so the column unrolled a row at a time and you
        // could not see what you were choosing between until you had already
        // dragged past most of it. A menu you can only read one item of is not
        // a menu; the owner asked for it to spring open, and it is right.
        //
        // The lift still drives the SELECTION — `columnSelection` is unchanged
        // — so the finger keeps choosing among rows that are all already on
        // screen. That is the part worth keeping from the original: the
        // continuous drag from "next workspace" through the tabs and on into
        // the overview still works, it just stops hiding its options.
        //
        // The threshold is the same `openMin` a release uses to decide between
        // landing and abandoning, so the column is open exactly when a release
        // would do something.
        if pinned { return full }
        return up >= ShellMetrics.openMin ? full : 0
    }

    /// How far into the overview a lift of `up` has got, 0…1.
    ///
    /// Measured from the point where the column has nothing left to reveal, so
    /// the overview is always "keep going" and never "go a specific distance".
    static func overviewProgress(up: CGFloat, tabCount: Int) -> CGFloat {
        min(1, max(0, pageRise(up: up, tabCount: tabCount) / ShellMetrics.overRun))
    }

    /// How far the PAGE itself has travelled for a lift of `up`.
    ///
    /// Zero for the whole column phase, and that is the rule rather than an
    /// implementation detail: picking a tab is a light action taken INSIDE the
    /// workspace, so the menu opens over a page that has not moved. The page
    /// only becomes something you are holding once the finger goes past the
    /// last row and there is nothing left to pick — which is exactly where
    /// `overviewProgress` starts, so the two say the same thing about the same
    /// point and cannot come apart.
    ///
    /// Unclamped above, unlike `overviewProgress`. Past the overview's own run
    /// the page has finished shrinking but the finger has not finished moving,
    /// and a card that stopped following the thumb at some invisible line
    /// would be a card you had let go of without letting go.
    static func pageRise(up: CGFloat, tabCount: Int) -> CGFloat {
        max(0, up - columnFull(tabCount: tabCount))
    }

    /// Whether the page has left the display and is in your hand.
    ///
    /// The whole of what makes a lift able to answer the OTHER axis, and the
    /// place the decision the owner asked for is written down.
    ///
    /// **The axis lock is never released; the lift takes both axes.** The
    /// alternative — letting `axis` be re-decided once a lift is established
    /// — was considered and is wrong here, because the two axes are not
    /// competing for one meaning: the lift decides WHERE you end up (holding
    /// the page, or in the overview) and sideways decides WHICH workspace you
    /// end up holding. Handing the gesture over to `.horizontal` mid-drag
    /// would drop the page back onto the display while your thumb was still
    /// up in the air holding it, which is the same "a swipe that started as
    /// one thing finished as another" the lock exists to stop. Owning both is
    /// what a card in the app switcher does: you can move it sideways among
    /// its neighbours without ever stopping holding it.
    ///
    /// Not held for the whole column phase, and that is the same line
    /// `pageRise` draws. Below it the finger is choosing a column ROW and
    /// `dx` is nothing but the sideways wander of a thumb travelling up a
    /// phone; a page turn read out of that would make every tab choice a coin
    /// toss. Past the last row there is no row left to choose and the page has
    /// left the glass, so sideways has nothing else it could mean.
    static func pageIsHeld(up: CGFloat, tabCount: Int) -> Bool {
        pageRise(up: up, tabCount: tabCount) > 0
    }

    /// How far through the COLUMN's own stretch of the lift `up` has got, 0…1.
    ///
    /// The other half of the same drag. `overviewProgress` starts where this
    /// one finishes, so between them they cover the whole upward gesture with
    /// no gap and no overlap — this one for the stretch where the COLUMN
    /// answers the finger, the other for the stretch where the PAGE does.
    ///
    /// Nothing is drawn straight off this number: the column springs open at
    /// `openMin` rather than unrolling, and the page reads `pageRise`. What it
    /// is for is the join. It reaching 1 at precisely the lift where
    /// `overviewProgress` leaves 0 is what the tests pin down, and that single
    /// point is where the whole gesture changes hands.
    ///
    /// Normalized by the column's length rather than by an absolute distance,
    /// for the reason `overRun` gives: this gesture is "keep going until there
    /// is nothing left to reveal, then keep going", and a workspace with one
    /// tab has less to reveal than one with nine. A join fixed at some number
    /// of points would fall in the middle of a nine-tab column and past the
    /// end of a one-tab one.
    ///
    /// A workspace with no tabs has no column, so there is nothing for the
    /// lift to be a fraction OF; it reports fully travelled, which hands the
    /// whole gesture to `overviewProgress` and is the only answer that does
    /// not divide by zero.
    static func columnProgress(up: CGFloat, tabCount: Int) -> CGFloat {
        let full = columnFull(tabCount: tabCount)
        guard full > 0 else { return 1 }
        return min(1, max(0, up / full))
    }
}

/// What letting go does.
///
/// One enum for both gestures, because the two differ in what a step MEANS and
/// not in what happens after one — and because a view that switched on a
/// tuple of booleans instead is a view where "abandon" and "spring back" drift
/// apart into two subtly different nothings.
enum ShellRelease: Hashable {
    /// Animate the track to the neighbour, then re-seat on it. See the
    /// no-bounce commit in `ShellRootView`.
    case commit(ShellStep)
    /// A horizontal drag that did not go far enough. The track slides home.
    case springBack
    /// The column was open far enough and a row was under the finger.
    case land(tab: Int)
    /// Dragged past the last row, all the way into the overview.
    case openOverview
    /// Dragged past the last row AND far enough sideways: the page is carried
    /// into the neighbouring workspace's cell and the overview opens on that
    /// workspace instead of this one.
    ///
    /// Both axes answered at once, which is the point of it. It is not
    /// `commit` followed by `openOverview` — those are two springs and two
    /// arrivals — and it is not a third threshold: it is the same 70 points
    /// sideways and the same run past the last row, read off one release.
    case carry(ShellStep)
    /// A vertical drag that did not open the column far enough. Costs nothing.
    case abandon
    /// No axis at all: this was a tap.
    case toggleColumn
}

extension ShellFleet {
    /// What letting go of the BAR does.
    ///
    /// `up` is the lift, up-positive and already floored at zero by the
    /// caller; `axis` is the one decided on the first 6 points, or nil if the
    /// gesture never grew that big — which is a tap and is why this function
    /// can return `.toggleColumn` at all.
    func barRelease(axis: ShellAxis?, dx: CGFloat, up: CGFloat, at position: ShellPosition)
        -> ShellRelease
    {
        guard let axis else { return .toggleColumn }
        switch axis {
        case .horizontal:
            guard ShellGesture.commits(dx: dx), let direction = ShellGesture.direction(dx: dx),
                let step = step(from: position, direction, along: .bar)
            else { return .springBack }
            return .commit(step)
        case .vertical:
            let tabs = tabCount(ofWorkspace: position.workspace)
            // The sideways half of a lift, and it only exists once the page
            // is off the display — see `ShellGesture.pageIsHeld`, which is
            // also where the decision not to release the axis lock is
            // argued. Along `.bar`, because what a lifted page is holding is
            // a WORKSPACE: the cards it can be moved between are the
            // overview's cards, and those are workspaces.
            let sideways: ShellStep? =
                ShellGesture.pageIsHeld(up: up, tabCount: tabs)
                    && ShellGesture.commits(dx: dx)
                ? ShellGesture.direction(dx: dx).flatMap { step(from: position, $0, along: .bar) }
                : nil
            if up >= ShellGesture.columnFull(tabCount: tabs) + ShellMetrics.overRun {
                // The lift says you are staying in the overview; the sideways
                // says which cell the page lands in. At the ends of the fleet
                // there is no neighbour to carry to, and the lift's answer
                // stands on its own.
                return sideways.map(ShellRelease.carry) ?? .openOverview
            }
            if let sideways {
                // Lifted, but not far enough to stay up. The page comes back
                // down — onto the neighbour, because the finger asked for it
                // on the way. The same crossing a swipe along the bar makes,
                // reached from a few points higher up.
                return .commit(sideways)
            }
            if let row = ShellGesture.columnSelection(up: up, tabCount: tabs) {
                return .land(tab: row)
            }
            return .abandon
        }
    }

    /// What letting go of the CONTENT does.
    ///
    /// The same thresholds as the bar, over a different sequence — the flat
    /// one that runs through the whole fleet.
    ///
    /// There is deliberately no vertical arm. The content is a terminal, and
    /// vertical on a terminal is its scrollback (`TerminalScrollTests` is
    /// about exactly that gesture); the shell taking it would be the shell
    /// stealing a gesture the pane already owns and has a regression test for.
    /// A gesture that locks vertical on the content therefore resolves to
    /// nothing here and the pane keeps it.
    func contentRelease(axis: ShellAxis?, dx: CGFloat, at position: ShellPosition) -> ShellRelease {
        guard axis == .horizontal, ShellGesture.commits(dx: dx),
            let direction = ShellGesture.direction(dx: dx),
            let step = step(from: position, direction, along: .content)
        else { return .springBack }
        return .commit(step)
    }
}

// MARK: - The overview

extension ShellFleet {
    /// The workspaces in the order the overview shows them, as indices into
    /// `workspaces`.
    ///
    /// Indices and not values, because the current workspace's card is
    /// outlined and "current" is an index — matching by id afterwards would be
    /// a second way to say the same thing, and the two ways would eventually
    /// disagree about a fleet holding two workspaces on one branch.
    ///
    /// **Stable.** Equal precedence keeps fleet order, so the overview is the
    /// fleet with the loud ones lifted to the front rather than a new
    /// arrangement to learn. `sorted(by:)` in Swift is not guaranteed stable,
    /// hence the explicit tiebreak on the original index.
    func overviewOrder() -> [Int] {
        workspaces.indices.sorted { a, b in
            let pa = workspaces[a].precedence
            let pb = workspaces[b].precedence
            return pa == pb ? a < b : pa < pb
        }
    }

    /// The same order, filtered by a search over workspace NAMES.
    ///
    /// Case- and diacritic-insensitive, and a substring rather than a prefix:
    /// the names are branch-shaped (`feat/handle-retries-on-429`), so the word
    /// somebody remembers is very often in the middle. An all-whitespace query
    /// is an empty one — a stray space must not empty a grid of forty cards.
    func overviewOrder(matching query: String) -> [Int] {
        let needle = query.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !needle.isEmpty else { return overviewOrder() }
        return overviewOrder().filter {
            workspaces[$0].name.range(
                of: needle, options: [.caseInsensitive, .diacriticInsensitive]) != nil
        }
    }
}
