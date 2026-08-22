import SwiftUI

/// One workspace's tabs: its agents, and its diff.
///
/// Ported from nothing — the Mac always has its sidebar on screen, so
/// switching panes there is a click away regardless of which one is
/// open. A phone's workspace screen is full-bleed with no sidebar to fall
/// back to, and "go back to the list, find the row, tap it" is the wrong
/// cost for something as routine as glancing at a second agent. This strip
/// makes every tab one tap away without ever leaving the screen that
/// made checking on it worthwhile.
///
/// ## Why this is scoped to one workspace now
///
/// It used to be flat across the whole fleet, and the argument for that was
/// written down here: the 3am case is "is the OTHER agent still blocked", which
/// is as likely to be in a different worktree as the same one. That is still
/// true, and it stopped being the case this strip has to serve.
///
/// The job the owner described is reviewing what an agent did — reading what it
/// said, judging it, looking at the change, replying — and every one of those is
/// inside ONE worktree. A flat strip cannot hold that worktree's diff, because a
/// diff belongs to a workspace and a flat strip has no workspace; and if it did
/// hold one it would be a lone unlabeled chip in a row of ten unrelated ones.
/// Scoped, the strip is the toggle: the agents that did the work, and the work
/// they did, side by side.
///
/// The cross-worktree jump is not lost, it moved to where it reads better. Back
/// is a list ranked by what needs you, which answers "is the other agent still
/// blocked" with a sentence rather than with a chip's dot — and the switcher
/// sheet in the toolbar still lists the whole runner.
///
/// A host-side `changes` pane is filtered out. The Changes chip already is that
/// pane's review — same `ChangesStore`, keyed by workspace — and two chips onto
/// one diff is a choice with no difference behind it.
struct TerminalTabStrip: View {
    /// The workspace whose tabs these are. Optional only for the moment between
    /// this screen appearing and the fleet's next answer; the Changes chip
    /// stands on its own until then, because the diff is fetched by workspace id
    /// and needs nothing from the fleet.
    let workspace: Workspace?
    /// What this worktree has changed, for the Changes chip's counts. Nil when
    /// the daemon has not answered yet or there is nothing to say.
    let changes: InboxRow?
    let current: Pane
    let onSelect: (Pane) -> Void

    /// The agent chips, in a deliberately unchanging order.
    ///
    /// Fleet order, not `sortRank`. `FleetList` sorts its rows by the runner's
    /// rank and defends that at length, and the argument it had to answer — a
    /// row that moves under a finger already travelling toward it is a tap that
    /// lands on something else — is worse here, not better: these targets are
    /// small, they sit in one line, and an agent going from working to blocked
    /// would slide every chip past the thumb aimed at one of them.
    ///
    /// A host-side `changes` pane is not among them; the Changes chip is that
    /// pane. See `Pane.init(_:)`.
    private var terminals: [Terminal] {
        (workspace?.terminals ?? []).filter { !$0.isChangesPane }
    }

    var body: some View {
        ScrollViewReader { proxy in
            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: 4) {
                    // Changes leads, so the one chip that is always there is
                    // always in the same place — the diff is what a thumb can
                    // find without reading, and the agents shuffle around it as
                    // panes come and go.
                    ChangesChip(
                        counts: changes,
                        isCurrent: current.id == Pane.changes.id,
                        onTap: { onSelect(.changes) }
                    )
                    .id(Pane.changes.id)

                    let numbering = workspace?.ordinals() ?? [:]
                    ForEach(terminals) { terminal in
                        TabChip(
                            terminal: terminal,
                            ordinal: numbering[terminal.id],
                            isCurrent: current.id == Pane(terminal).id,
                            onTap: { onSelect(Pane(terminal)) }
                        )
                        .id(Pane(terminal).id)
                    }
                }
                // Enough that a chip never reaches the bar's corner arc. Less
                // than the radius and the chip's own capsule pokes out through
                // the curve, which is what the clip below is really guarding.
                .padding(.horizontal, 8)
                .padding(.vertical, 5)
            }
            // Scrolled to the open terminal on first layout, not on every
            // fleet refresh: `.onAppear` fires once, `onChange(of: current)`
            // is what re-centers on a genuine tap (below) — a poll landing
            // mid-scroll must not fight the user's own gesture.
            .onAppear { proxy.scrollTo(current.id, anchor: .center) }
            .onChange(of: current.id) { _, id in
                withAnimation(.easeOut(duration: 0.2)) { proxy.scrollTo(id, anchor: .center) }
            }
        }
        // ONE surface, not one per chip.
        //
        // Three versions: a flat dark bar (right when everything above it was a
        // flat dark terminal, wrong under a floating composer), then a chip
        // apiece on glass — which put five or ten separate floating objects
        // below the one floating object that matters, and read as a browser tab
        // strip pasted under a chat.
        //
        // A single pill holding them is one sibling of the composer rather than
        // a crowd competing with it, and it is the shape iOS 26 gives a
        // floating group of controls.
        //
        // CLIPPED to that shape, not merely backed by it. `glassEffect(in:)`
        // draws a surface behind its content and does not constrain it, so a
        // scrolled chip — and in particular the ring around one that wants
        // attention — carried on straight through the bar's rounded corner and
        // out the side. A background is not a boundary.
        .clipShape(RoundedRectangle(cornerRadius: 20))
        .modifier(GlassSurface(radius: 20))
        .padding(.horizontal, 10)
    }
}

/// The worktree's own tab: what the branch changed.
///
/// Always there, including on a workspace with no panes at all, because the
/// diff is asked for by workspace id — see `Pane`. That is also why it needs no
/// terminal, no dot and no state: nothing about it can be starting, exited or
/// lost.
///
/// **Deliberately not amber.** Orange means an agent is waiting on an answer,
/// on this strip and on the widget, the Live Activity, the complication and the
/// inbox, and a glance at any of them has to answer "does this need me" without
/// reading a word. A diff waiting to be read is worth showing; it is not worth
/// the color that means someone is stuck. `NeedsYouView.workspaceRow` makes the
/// same argument about the same fact.
///
/// So the counts carry it instead, in the green and red they have everywhere
/// else in this app — which says how big the change is as well as that there is
/// one, in the same space a badge would have taken.
private struct ChangesChip: View {
    let counts: InboxRow?
    let isCurrent: Bool
    let onTap: () -> Void

    var body: some View {
        Button(action: onTap) {
            HStack(spacing: 5) {
                Image(systemName: "doc.text.magnifyingglass")
                    .font(.system(size: 11))
                Text("Changes")
                    .font(.caption)
                    .lineLimit(1)
                // Absent entirely on a clean worktree. `+0 -0` on every branch
                // with nothing on it is noise in the shape of information —
                // `FleetList`'s workspace header leaves it out for the same
                // reason.
                if let counts, counts.hasDiff {
                    HStack(spacing: 3) {
                        Text("+\(counts.insertions)").foregroundStyle(.green)
                        Text("-\(counts.deletions)").foregroundStyle(.red)
                    }
                    .font(.caption2.monospaced())
                }
            }
            .padding(.horizontal, 10)
            .padding(.vertical, 5)
            .modifier(ChipGlass(isCurrent: isCurrent))
            .foregroundStyle(isCurrent ? Color.white : Color.white.opacity(0.75))
        }
        .buttonStyle(.plain)
        .accessibilityIdentifier("workspace-tab-changes")
        .accessibilityLabel(accessibilityLabel)
        .accessibilityValue(isCurrent ? "current" : "")
        .accessibilityAddTraits(isCurrent ? .isSelected : [])
    }

    /// The counts, and what they count.
    ///
    /// The third surface fed by the fleet inbox, and it says the same clause as
    /// the other two: this is everything the worktree has changed, not the
    /// branch total the screen behind the chip shows under Branch. There is no
    /// hover on a phone, so an accessibility label is the only place any of the
    /// three can say it — see `FleetList`'s workspace header and `NeedsYou`.
    private var accessibilityLabel: String {
        guard let counts, counts.hasDiff else { return "Changes" }
        return "Changes, \(counts.insertions) added, \(counts.deletions) removed, "
            + "including work that isn’t committed yet"
    }
}

/// One agent's tab. Compact by necessity — this is a phone, and the strip has
/// to hold a workspace's panes and its diff on one line.
private struct TabChip: View {
    let terminal: Terminal
    let ordinal: Int?
    let isCurrent: Bool
    let onTap: () -> Void

    private var kind: StateKind { StateKind.parse(terminal.state) }
    private var wantsAttention: Bool { terminal.agent.wantsAttention }

    var body: some View {
        Button(action: onTap) {
            HStack(spacing: 5) {
                // Same dot, same colors as the fleet list — `processColor`
                // is shared rather than redefined here so a terminal cannot
                // read green in one screen and red in the other.
                Circle().fill(processColor(kind)).frame(width: 6, height: 6)
                // Not monospaced. The strip names TERMINALS, which are things
                // in this app, not text a terminal is showing — and monospace
                // beside a chat's body font is what made it read as a piece of
                // some other program's chrome.
                // Capped, because a chip now carries the CONVERSATION's name
                // rather than "claude 2", and an agent will happily call one
                // "Complete D17 authorization decision for Far Cooler" — which
                // filled the strip with a single tab and pushed every other
                // pane off the end of it.
                Text(terminal.displayName(ordinal: ordinal))
                    .font(.caption)
                    .fontWeight(wantsAttention ? .semibold : .regular)
                    .lineLimit(1)
                    .truncationMode(.tail)
                    .frame(maxWidth: 150, alignment: .leading)
                    .fixedSize(horizontal: false, vertical: true)
            }
            .padding(.horizontal, 10)
            .padding(.vertical, 5)
            .modifier(ChipGlass(isCurrent: isCurrent))
            .overlay(
                Capsule()
                    .strokeBorder(
                        wantsAttention ? attentionColor(terminal) : .clear, lineWidth: 1.5)
            )
            .foregroundStyle(isCurrent ? Color.white : Color.white.opacity(0.75))
        }
        .buttonStyle(.plain)
        .accessibilityIdentifier("terminal-tab-\(terminal.id)")
        .accessibilityLabel(accessibilityLabel)
        .accessibilityValue(isCurrent ? "current" : "")
        .accessibilityAddTraits(isCurrent ? .isSelected : [])
    }

    /// The chip's name, plus the one thing its two ornaments are drawn to say.
    ///
    /// Read aloud, a chip was its name and nothing else: the ring and the dot
    /// are color, and color is the one channel VoiceOver cannot carry. The ring
    /// in particular is the amber "this wants you" signal the whole product
    /// turns on — on the widget, the Live Activity, the complication and the
    /// inbox — so a reader who cannot see it was the one reader getting none of
    /// it.
    ///
    /// `ChangesChip`'s shape exactly: a name, and a clause only when there is a
    /// fact to put in it. The restraint matters more here than there, because
    /// there is one Changes chip and there can be eight of these. A strip where
    /// every chip recites an activity is a list nobody can hear the end of, and
    /// a strip exists to be scrubbed past. `NeedsYouView` made the same call
    /// when it labelled the workspace header and left the `TerminalRow`s under
    /// it to speak for themselves.
    ///
    /// So a running pane whose agent is working, idle, or not an agent at all
    /// says only its name. That is not a fact withheld: it is the chip with no
    /// ring and a green dot, and a reader learns that state by hearing nothing
    /// after a name rather than by hearing "Working" eight times on the way to
    /// the tab they wanted.
    private var spokenStatus: String? {
        // The two ornaments are not independent, so they resolve to one clause
        // rather than two — activity only means anything while the process is
        // alive, so a live pane speaks its agent and a dead one speaks its
        // process. That derivation and these words are the Mac's `Status`; see
        // `apps/macos/Sources/FarCooler/Model.swift`, so one pane cannot be
        // called two different things by the two apps.
        switch kind {
        case .running: return wantsAttention ? terminal.activityLabel : nil
        case .starting: return "Starting"
        // How it ended, not merely that it did — the same split `runDidFail`
        // draws, so a chip cannot say "Exited" about a command that died.
        case .exited: return terminal.runDidFail ? "Failed" : "Exited"
        case .error: return "Failed to start"
        case .lost: return "Lost"
        // Not "Unknown", which is an internal word covering two situations.
        // The daemon could not read this pane, which is a claim about the
        // reading and not about the pane — the distinction the Mac's
        // `unreadable` exists to make.
        case .unknown: return "Not answering"
        }
    }

    private var accessibilityLabel: String {
        let name = terminal.displayName(ordinal: ordinal)
        guard let status = spokenStatus else { return name }
        return "\(name), \(status)"
    }
}


/// A chip's surface: glass on iOS 26, the nearest material before it.
///
/// A capsule rather than a rounded rectangle, because that is the shape the
/// platform gives a floating, tappable pill — and because a rounded rectangle
/// next to the composer's rounded rectangle read as a smaller version of the
/// same thing rather than as a different kind of control.
private struct ChipGlass: ViewModifier {
    let isCurrent: Bool

    func body(content: Content) -> some View {
        // A fill, not a second pane of glass. Glass inside glass has nothing
        // new to refract and just muddies the edge that was doing the work.
        content.background(
            Capsule().fill(isCurrent ? Color.white.opacity(0.16) : Color.clear))
    }
}
