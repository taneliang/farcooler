import SwiftUI

// Every workspace at once, arrived at by keeping going.
//
// The overview is not a screen you navigate to. It is the far end of the same
// upward drag that unfurls the column: past the last row the column dissolves
// and this arrives in its place, so "show me everything" is the same gesture
// as "show me this workspace's tabs", continued. That is why there is no
// button anywhere that opens it.
//
// It is also the design's answer to forty workspaces. Every strip-shaped
// design died on that number — a bar that compresses to fit forty of anything
// is a bar with forty illegible things on it — and the answer is that the bar
// never tries: it shows ONE workspace, and the fleet lives here, in a grid
// that can scroll.
//
// ## The platform draws the chrome, this file draws the cards
//
// The header, the search field and the empty state were all hand-built once —
// an `HStack` with a title and a `Done` in it, a `TextField` inside a capsule,
// a `Text` centred between two spacers. Every one of them was a near-miss:
// the header did not blur or collapse as the grid went under it, the search
// field had no cancel button, no dictation, no scroll-to-dismiss, and the
// empty state was a sentence where the platform has a whole layout. They are
// now a `NavigationStack`, a `.searchable` and a `ContentUnavailableView`, and
// what is left in this file is the part iOS has no opinion about: a workspace,
// drawn as a card.
//
// What the platform's chrome cost, and it is the one thing about it that had to
// be designed rather than adopted: a header, a `Done` and a search field all
// answer to this view being MOUNTED, and it is mounted from the first point of
// a lift so the page has a cell to fly to. Left at that, the whole screen's
// furniture faded up under a page somebody might still be putting back. See
// `chrome`, which is when a lift stops being a lift.
//
// The one thing that could NOT become the platform's own is the way out by
// touch. `Done` is a real toolbar button, but the drag back down that also
// closes this used to live on the hand-built header, and there is no header
// left to attach it to. It is now a pull-down on the grid itself, gated on the
// scroll having BEGUN at its top and attached as a SIMULTANEOUS gesture so the
// scroll never loses an argument to it — which is the sheet idiom, and closer
// to what a person will try than the old strip-only version was.
//
// "Begun at its top" is the whole of the gate and it was "ended at its top",
// which is not the same claim and is the one the owner's phone caught: every
// scroll back up through forty cards ends at the top with a large downward
// translation behind it, so browsing the grid dismissed it. See `pullBegan`.

/// Where each laid-out card is, by workspace index, in screen coordinates.
///
/// The page flies INTO one of these, so they have to be the frames the grid
/// actually laid out rather than ones computed twice — a second copy of the
/// grid's arithmetic here would be right until somebody changed a padding.
///
/// Every card and not only the current one, because the journey runs both
/// ways. The page lands in the cell of the workspace it IS; it grows back out
/// of the cell of the workspace you TAP, and which one that is nobody knows
/// until the tap. A key that only carried the current card would have the
/// return leg starting wherever the outbound one finished.
struct ShellTileFrame: PreferenceKey {
    static let defaultValue: [Int: CGRect] = [:]
    static func reduce(value: inout [Int: CGRect], nextValue: () -> [Int: CGRect]) {
        value.merge(nextValue()) { _, new in new }
    }
}

/// A workspace card's FACE: everything drawn on it, and nothing about being in
/// a grid.
///
/// Split out of the card because the flight needs to draw one too. The page
/// that lifts off the display has to ARRIVE as a card rather than as a grey
/// rectangle that a card fades in over afterwards — see `ShellRootView`'s
/// `cardFace` — and the only way that landing can be invisible is if the thing
/// that lands and the thing already in the cell are the same drawing. Two
/// views that merely looked alike would be a handover you could see, and it
/// would drift the first time either was tuned.
struct ShellCardFace: View {
    let workspace: ShellWorkspace
    let isCurrent: Bool
    /// How tall the card's own rectangle is — its ground, its fill, its
    /// corner and its outline.
    ///
    /// The card's SIZE, and never its content's. In the grid this is the
    /// card's own height and the two are the same rectangle; on the flight it
    /// is whatever the crop is still drawing, which starts as a whole page and
    /// closes to a card. Growing the rectangle rather than the layout is what
    /// makes the flight one object changing size: the words inside are laid
    /// out once, at the bottom, and never hear that anything is moving, while
    /// the fill, the corner and the amber edge are the page's own — so what
    /// arrives in the cell is a card that has been a card the whole way rather
    /// than a small card pasted onto a big rectangle.
    var height: CGFloat = ShellCardFace.size.height
    /// The corner that rectangle is drawn with. The display's while the page
    /// is still page-shaped, the card's once it is not.
    var radius: CGFloat = ShellMotion.cardRadius
    /// Whether to put an opaque ground under the platform's translucent fill.
    ///
    /// False in the grid, where the card is drawn ON the overview's ground and
    /// is meant to sample it. True on the flight, where there is a terminal
    /// underneath that would otherwise read through the fill — and where the
    /// ground it needs is the same one the page carries, so the two composite
    /// to the same colour and the handover at the end has nothing to give
    /// away.
    var opaqueGround: Bool = false

    /// `server · N tabs`, with the server left out when it is the local one —
    /// the same rule the bar follows, so a card and the bar never disagree
    /// about whether a workspace is worth naming a runner for.
    private static func subtitle(for workspace: ShellWorkspace) -> String {
        let tabs = "\(workspace.tabs.count) \(workspace.tabs.count == 1 ? "tab" : "tabs")"
        guard let server = workspace.server else { return tabs }
        return "\(server) · \(tabs)"
    }

    var body: some View {
        VStack(alignment: .leading, spacing: PaneMetrics.step) {
            Text(workspace.name)
                // `.subheadline`, which IS the 15 the brief asks for —
                // said as a role rather than as a number, so it tracks the
                // reader's text size instead of ignoring it.
                .font(.subheadline.weight(.medium))
                .lineLimit(2)
                .multilineTextAlignment(.leading)
                .truncationMode(.middle)
                .frame(maxWidth: .infinity, alignment: .leading)

            // What this workspace last said, which is the whole reason a
            // card beats a list row. Mono because it came off a machine.
            //
            // The one size on this card still written as a number, and
            // deliberately: the smallest text style is `.caption2` at 11,
            // and 11-point SF Mono fits 22 characters across a 168-point
            // card where 9 fits 27. Three tail lines all truncating mid-
            // word is a tail that has stopped being readable to save a
            // number, so the number stays and the card is clamped below
            // instead.
            //
            // `Spacer` AFTER it rather than before: a tail of one line and
            // a tail of three must both sit under the name, or the text
            // floats at a different height on every card and the grid stops
            // being scannable. The empty case still spaces correctly
            // because the spacer is doing the work either way.
            VStack(alignment: .leading, spacing: 1) {
                ForEach(Array(workspace.tail.suffix(3).enumerated()), id: \.offset) { _, line in
                    Text(line)
                        .font(.system(size: 9, design: .monospaced))
                        .foregroundStyle(.tertiary)
                        .lineLimit(1)
                        .truncationMode(.tail)
                        .frame(maxWidth: .infinity, alignment: .leading)
                }
            }

            Spacer(minLength: 0)

            // The same ribbon the bar carries, at 5 rather than 6 — the
            // card is a smaller thing saying the same thing, and a second
            // way of summarizing a workspace would be a second thing that
            // can disagree with the bar about it.
            //
            // No current mark: `current` is -1 because the card is a
            // WORKSPACE, and which tab you are on inside it is not a fact
            // about the workspace. The amber outline says which workspace
            // you are in.
            ShellRibbon(tabs: workspace.tabs, current: -1, size: 5)

            Text(Self.subtitle(for: workspace))
                // Mono, because the runner's name came off a machine and
                // the rule in the brief is that mono means data. SF Mono's
                // digits are already tabular, so a count that changes value
                // does not change width and a grid of cards does not
                // twitch.
                //
                // `.caption2` rather than the `.caption` the brief's 12
                // maps to, and measured rather than preferred: mono is
                // wider than the system face, and `eu-runner-1 · 2 tabs`
                // at 12 does not fit the 144 points a card has left after
                // its padding — it truncated the RUNNER, which is the half
                // of the line that is information. One step down fits it
                // whole.
                .font(.system(.caption2, design: .monospaced))
                .foregroundStyle(.secondary)
                .lineLimit(1)
                .truncationMode(.middle)
        }
        .padding(PaneMetrics.card)
        // 168×132, from the mechanics doc. Two across on a phone, which at 393
        // points leaves the grid its own margins without any of the cards
        // having to be a different width from the others.
        .frame(width: Self.size.width, height: Self.size.height, alignment: .topLeading)
        // And then the rectangle the card IS, which is only a different size
        // while a flight is in the air. Bottom-aligned, because the bottom
        // edge is the one the flight's crop keeps: the words hold perfectly
        // still against it while the rectangle closes around them.
        .frame(width: Self.size.width, height: max(Self.size.height, height), alignment: .bottom)
        .background(
            opaqueGround
                ? AnyShapeStyle(Themes.shared.current.backgroundColor) : AnyShapeStyle(.clear),
            in: RoundedRectangle(cornerRadius: radius, style: .continuous))
        // A system card: the platform's own fill hierarchy over this
        // surface's ground, not a hand-mixed white at 6%. `TranscriptFill` is
        // where this app already keeps that vocabulary, and `container` is the
        // tier it uses for exactly this — a thing drawn ON the ground rather
        // than a ground of its own.
        //
        // `ShellMotion.cardRadius`, not a 16 written here: the page flies into
        // this shape, and a corner that only matched by coincidence would come
        // apart the first time either end was tuned.
        .background(
            TranscriptFill.container,
            in: RoundedRectangle(cornerRadius: radius, style: .continuous))
        // No hairline on an ordinary card. A system card is a fill, and
        // the border that used to be here was a second, weaker way of
        // saying the same edge — which left the amber one competing with
        // forty greys instead of standing alone.
        .overlay {
            if isCurrent {
                RoundedRectangle(cornerRadius: radius, style: .continuous)
                    // Amber, and only on the one you are in. It is the same
                    // "you are here" the elongated mark is in the ribbon, at
                    // the scale where a mark would be lost.
                    .strokeBorder(Color.orange.opacity(0.7), lineWidth: 1.5)
            }
        }
    }

    /// The card's own size, in one place because two things are laid out
    /// against it: the grid's columns, and the flight that scales a page down
    /// until it is exactly this.
    static let size = CGSize(width: 168, height: 132)
}

/// One workspace as a card in the grid: a face, a frame it publishes, and a
/// tap that opens it.
private struct ShellOverviewCard: View {
    let workspace: ShellWorkspace
    /// Where this card sits in the fleet, which is the key its frame is
    /// published under and the only stable name a grid this is sorted and
    /// filtered has for it.
    let index: Int
    let isCurrent: Bool
    /// Whether this cell is a HOLE: the space reserved, and nothing drawn in
    /// it, because the page on its way here is the card.
    let isEmpty: Bool
    let onOpen: () -> Void

    var body: some View {
        Button(action: onOpen) {
            ShellCardFace(workspace: workspace, isCurrent: isCurrent)
                // Every card reports where it is: any of them can be the end
                // of a flight, and the one that is stops being known the
                // moment a finger lands on a different one.
                //
                // Measured OUTSIDE the face's fixed frame, so what is
                // published is the rectangle the card actually occupies rather
                // than whatever its contents happened to lay out to.
                .background {
                    GeometryReader { geo in
                        Color.clear.preference(
                            key: ShellTileFrame.self, value: [index: geo.frame(in: .global)])
                    }
                }
        }
        .buttonStyle(.plain)
        // The card is content, so its type does not grow: the grid is 168 by
        // 132 by design — the number the flight lands on — and text that keeps
        // scaling inside a frame that cannot walks straight out of the corner.
        // The chrome around it, which is the platform's, scales all the way.
        .dynamicTypeSize(...DynamicTypeSize.large)
        // A hole, and made of opacity rather than of an `if`.
        //
        // The space has to stay reserved — the grid must not close up around a
        // cell whose card is in the air, or the page would be flying at a spot
        // that moves as it goes — and the frame above has to go on being
        // published, since that spot is the flight's destination. Opacity
        // changes neither, and it is the only one of the three that can be
        // faded: the handover at the end of a flight is a crossfade between
        // this card and the page landing on it, in one rectangle, and an `if`
        // has no in-between to fade through.
        .opacity(isEmpty ? 0 : 1)
        .allowsHitTesting(!isEmpty)
        .accessibilityHidden(isEmpty)
        .accessibilityIdentifier("shell-card-\(workspace.id)")
        .accessibilityLabel(workspace.name)
    }
}

/// One workspace on a runner this app is not connected to.
///
/// The same FACE as an ordinary card — deliberately, because it is the same
/// thing: a worktree with a name, a ribbon and a tail. What is different is
/// everything about it being live.
///
/// - **It publishes no tile frame.** `ShellTileFrame` is keyed by an index
///   into `fleet.workspaces`, and it is what a lifted page flies INTO. A card
///   that is not in that fleet has no index to publish under, and publishing
///   one anyway would collide with a live workspace's cell — a page flying to
///   a card that names a different worktree on a different machine.
/// - **It is never `isCurrent`.** The amber outline means "you are here", and
///   you are not.
/// - **Its rings are dashed**, because `RunnerDirectory.group` decayed them
///   on the way in: `working` became `stale`, while blocked and unread-diff
///   held. That is this app's existing staleness rule (`GlanceMark.Link`) and
///   not a second one invented for this grid.
/// - **A tap does not open it.** There is no pane to open — the terminal it
///   names is on a machine this app has no connection to — so the tap goes to
///   `onOpen`, which crosses runners and says so first.
private struct ShellElsewhereCard: View {
    let workspace: ShellWorkspace
    let onOpen: () -> Void

    var body: some View {
        Button(action: onOpen) {
            ShellCardFace(workspace: workspace, isCurrent: false)
        }
        .buttonStyle(.plain)
        .dynamicTypeSize(...DynamicTypeSize.large)
        .accessibilityIdentifier("shell-elsewhere-\(workspace.id)")
        // Named with its runner, because that is the whole of what makes this
        // card different from the one above it and a label reading only the
        // branch would be two cards with one name.
        .accessibilityLabel("\(workspace.name), on \(workspace.server ?? "another runner")")
    }
}

/// The grid, its search, and the way out.
///
/// A `LazyVGrid`, and the pane invariant does not reach here. The rule that
/// forbids a lazy container — `ShellPaneTrack.swift:5-30`, and the four
/// comments the mechanics doc cites — is about PANES: a recycled pane loses a
/// scroll position, a half-typed message, an open stream. A card holds a name
/// and some dots, it is rebuilt from the fleet every time the fleet changes
/// anyway, and forty of them realized at once is forty views' worth of layout
/// for a surface somebody is about to leave.
struct ShellOverview<Actions: View>: View {
    let fleet: ShellFleet
    let current: Int
    /// Whether the current workspace's cell is a hole rather than a card,
    /// because its page is in the air between the screen and that cell.
    let currentIsEmpty: Bool
    /// Whether this is the DESTINATION rather than something a lift is
    /// revealing — and therefore whether the platform's chrome belongs on it.
    ///
    /// **The grid arrives with the gesture and the chrome arrives with the
    /// answer, and those are two different moments.** This view is mounted from
    /// the first point of lift, because the page has to know where its cell is
    /// before it starts flying there, and it is revealed by opacity as the page
    /// stops covering it. That is right for the CARDS: what a lift uncovers is
    /// the fleet, and uncovering is the whole sentence the gesture says. It is
    /// wrong for the header, the `Done` and the search field. A lift is
    /// abandonable for its whole length, and a screen's furniture fading up
    /// underneath a page you might still put back is the screen claiming to
    /// have arrived while you are still deciding — the owner's words were that
    /// it breaks the illusion.
    ///
    /// So the chrome is not faded, it is ABSENT, and it appears when the
    /// release has resolved. What makes that affordable is that none of the
    /// three costs the grid a single point of layout — see each of them below.
    let chrome: Bool
    /// What to call the runner these cards are on, or nil where there is
    /// nothing to tell it apart from.
    ///
    /// Drawn on a section header over the live cards and ONLY when there is a
    /// second section under them. A heading that is always there is a heading
    /// that stops being read — the same rule `ShellCardFace.subtitle` follows
    /// when it leaves the local runner's name off a card.
    var liveServer: String? = nil
    /// The worktrees on the OTHER runners this app knows, as it last saw them.
    ///
    /// Cached, and they say so: see `ShellServerGroup`, and `RunnerDirectory`
    /// for why they are cached rather than live. Empty is the ordinary case —
    /// one runner, one section, no headings at all.
    var elsewhere: [ShellServerGroup] = []
    @Binding var search: String
    let onOpen: (Int) -> Void
    /// A card on another runner, tapped. The grid does not know what crossing
    /// costs; `ShellScreen` does, and it is the one that asks.
    var onCross: (ShellServerGroup, ShellWorkspace) -> Void = { _, _ in }
    let onDismiss: () -> Void
    /// What this app puts in the navigation bar opposite `Done`.
    ///
    /// A builder rather than anything this file knows about, and that seam is
    /// deliberate: `ShellOverview` is the grid, and the grid is the same grid
    /// whether it is standing on a real runner or on `ShellHarness`'s canned
    /// fleet. What goes here is everything that is a fact about THIS app —
    /// which runner you are looking at, and the two ways of starting work —
    /// and none of it means anything to a fixture. See
    /// `ShellScreen.overviewActions`, which is the only caller that passes
    /// any.
    ///
    /// **Why this screen and not some other.** The overview IS the fleet
    /// screen: it lists every workspace on the runner, sorted by what needs
    /// you, with a search field and a real navigation bar. That is what the
    /// pushed workspace list was, and the toolbar it had — a sparkle for
    /// "describe it", a plus for "fill in the form" — was on that screen
    /// because it was the one place work could be started from. It still is.
    @ViewBuilder var actions: () -> Actions

    /// Whether the grid is scrolled to its top, which is the only place a
    /// pull-down means "close this" rather than "scroll up".
    @State private var atTop = true

    /// Whether the grid was at its top when the drag now under way BEGAN, or
    /// nil between drags.
    ///
    /// **A pull-down is a gesture that starts at the top, not one that ends
    /// there**, and reading `atTop` at the release alone is what made the
    /// overview close itself on the owner's phone. Every scroll back up
    /// through forty cards finishes at the top with a large downward
    /// translation behind it — which is character for character the same
    /// release a deliberate pull-down produces — so browsing the grid threw
    /// you back onto the workspace you came from, with nothing about the
    /// gesture to say why.
    ///
    /// Both halves are still required: it must have begun at the top AND still
    /// be there. Begun alone would dismiss a scroll that started at the top
    /// and travelled; still-there alone is the bug.
    @State private var pullBegan: Bool?

    private var order: [Int] { fleet.overviewOrder(matching: search) }

    /// The worktrees this runner has been told to stop showing.
    private var hidden: [Int] { fleet.hiddenOrder(matching: search) }

    /// The other runners worth drawing a section for. See
    /// `ShellServerGroup.arrange`: a group a search emptied is not a heading.
    private var groups: [ShellServerGroup] {
        ShellServerGroup.arrange(elsewhere, matching: search)
    }

    /// Whether the hidden section is open.
    ///
    /// Collapsed by default, which is the Mac's rule and the point of hiding:
    /// these are not meant to be in the way. Not remembered across a lift
    /// either — the overview is opened and left many times a minute, and a
    /// section that stayed open would be the put-away worktrees quietly
    /// becoming permanent again.
    @State private var hiddenShown = false

    var body: some View {
        overviewBody
            // A ground of its own, and nearly opaque.
            //
            // Without one this view is transparent, so the page and the bar
            // behind it — which recede to 0.2 and 0.1, not to nothing — show
            // THROUGH the cards as legible text: a workspace name ghosted
            // across a card that names a different workspace. The prototype
            // avoids it by being 94% opaque over the app's ground, and nets
            // about a percent of bleed; glass alone nets far more.
            //
            // So: the theme's own ground at the prototype's alpha, with the
            // material above it. Depth, not a double exposure.
            .background { ground }
    }

    /// The theme's ground, drawn edge to edge.
    ///
    /// Applied TWICE, and the second one is not a mistake. A navigation stack
    /// brings its own opaque background — the system's, black in a dark
    /// appearance — and it sits between this ground and the grid, which turned
    /// the whole surface from the theme's slate into a flat black and took the
    /// reveal with it. Putting the same ground inside the stack as well is
    /// what the navigation bar and the grid actually sample; the outer one
    /// still covers the strip the safe area holds open around them.
    private var ground: some View {
        Themes.shared.current.backgroundColor
            .opacity(0.94)
            .ignoresSafeArea()
    }

    /// A navigation stack, because everything a navigation stack does here is
    /// something this screen was faking.
    ///
    /// The title collapses as the grid goes under it, the bar picks up the
    /// standard material over whatever is scrolling beneath, `Done` sits where
    /// a confirming action sits on this platform, and the search field is the
    /// system's — cancel button, dictation, scroll-to-dismiss and all. None of
    /// those were worth hand-building and all of them were noticed missing.
    private var overviewBody: some View {
        NavigationStack {
            content
                .background { ground }
                // The count in the title, which is where a count belongs on
                // this platform and where the hand-built header already had
                // it. Large, so it collapses into the bar as the grid rises
                // under it — the collapse is the thing that tells you the
                // content is moving behind the chrome.
                //
                // Emptied rather than hidden while the overview is only being
                // revealed, and the difference is the grid's own position. A
                // large title with no text keeps the bar at exactly the height
                // it had — measured, not assumed: hiding the navigation bar
                // instead moves every card 184 points up the screen, and the
                // cards are what the page is flying INTO, so the destination
                // would jump the moment the chrome resolved. An empty string
                // changes nothing but the glyphs.
                .navigationTitle(chrome ? "\(fleet.workspaces.count) Workspaces" : "")
                .navigationBarTitleDisplayMode(.large)
                .toolbar {
                    // Opposite `Done`, which on this platform is the leading
                    // edge — and free, because a stack with one screen in it
                    // has no back button.
                    //
                    // Faded rather than removed, for the same reason `Done`
                    // below is: a toolbar item that comes and goes is a
                    // toolbar that re-lays-out, and this whole bar is only
                    // ever invisible while a page is still covering it.
                    // `disabled` as well as faded, because an invisible
                    // control that still answers a tap is worse than one that
                    // is merely there.
                    ToolbarItemGroup(placement: .topBarLeading) {
                        actions()
                            .opacity(chrome ? 1 : 0)
                            .disabled(!chrome)
                    }

                    // "Done" in words rather than a glyph, because the gesture
                    // that opened this is a drag and the gesture that closes it
                    // is not obvious: a person who arrived here by accident
                    // needs a way out that does not require guessing which
                    // direction to swipe. `.confirmationAction` puts it at the
                    // trailing edge on iOS and lets the platform decide what
                    // that means everywhere else.
                    ToolbarItem(placement: .confirmationAction) {
                        Button("Done", action: onDismiss)
                            .accessibilityIdentifier("shell-overview-done")
                            // Opacity rather than an `if`, because a toolbar
                            // item that comes and goes is a toolbar that
                            // re-lays-out, and this one is only ever invisible
                            // while the whole surface is behind a page anyway.
                            .opacity(chrome ? 1 : 0)
                    }
                }
                // On an anchor of its own rather than on the grid, and that
                // is the one structural thing in this file.
                //
                // The field is the half of the chrome a lift actually uncovers
                // — it is a floating capsule at the BOTTOM of the display,
                // exactly where the bar was and exactly where the rising page
                // stops covering first — so it is the one that has to be
                // absent rather than merely faint. There is no handle on it:
                // it inherits the opacity of the whole surface and nothing
                // smaller, `isPresented: false` still draws it, and hiding the
                // bars it might belong to either does nothing or moves the
                // grid. Mounting is the only switch.
                //
                // Which is why it hangs off a `Color.clear` and not off the
                // scroll view. `.searchable` finds the navigation stack from
                // anywhere beneath it, so the field is the same field; what
                // changes is whose identity is at stake when the branch flips.
                // On the grid, flipping it would destroy and rebuild the
                // `ScrollView` at the instant of release — and the tile frames
                // the page is mid-flight toward are published from inside it,
                // so for one frame there would be no cell to fly to and the
                // page would snap back to the display. On an empty overlay,
                // the only thing rebuilt is the empty overlay.
                .overlay { searchField }
        }
        // `.contain`, not a bare identifier on the stack.
        //
        // An accessibility modifier on a container that is not ITSELF an
        // element is pushed down onto every element inside it, so naming this
        // stack renamed the Done button, the search field and all forty cards
        // to "shell-overview" — a grid whose every handle was the same word.
        // Declaring it a container keeps its own name and leaves theirs alone.
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("shell-overview")
    }

    /// The search field, carried by nothing.
    ///
    /// `.toolbar` placement, which on iOS 26 is the field's own Liquid Glass
    /// capsule rather than a field ruled into the navigation bar's drawer. The
    /// drawer placement was the right answer on iOS 18 and is the wrong one
    /// here: it draws a bordered rectangle inside the bar's material — a shape
    /// with no material of its own, sitting in somebody else's — which is
    /// exactly the "hand-styled capsule" this screen had already been rebuilt
    /// once to get rid of, arrived at a second time by asking for the
    /// platform's version of last year's design. The toolbar placement gives
    /// the field its own piece of glass, floating, with the grid passing behind
    /// it; it is the same vocabulary as the bar this shell puts at the bottom
    /// of every other screen, and it is the vocabulary the rest of this view
    /// was made native to join.
    ///
    /// Always showing when it shows at all, and NOT `.searchToolbarBehavior(
    /// .minimize)`. Minimizing collapses the field to a glyph until it is
    /// tapped, and this screen exists to answer "where is that workspace" at
    /// forty of them: a search field you have to find first is one you have to
    /// remember is there.
    ///
    /// `allowsHitTesting(false)` because the anchor fills the grid: a
    /// `Color.clear` is a real surface to SwiftUI, and an overlay of one over
    /// forty cards is forty cards nobody can tap.
    @ViewBuilder
    private var searchField: some View {
        if chrome {
            Color.clear
                .allowsHitTesting(false)
                .searchable(text: $search, placement: .toolbar, prompt: "Search")
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
        }
    }

    @ViewBuilder
    private var content: some View {
        if order.isEmpty && hidden.isEmpty && groups.isEmpty {
            // The platform's empty state, which is a layout and not a
            // sentence: glyph, title and explanation, centred and sized the
            // way every other iOS app's is. The wording is still this app's —
            // what failed is a match against a fleet, and "No Results" alone
            // would not say that.
            ContentUnavailableView {
                Label("No Workspaces", systemImage: "rectangle.on.rectangle.slash")
            } description: {
                // Two reasons a fleet can be empty, and they are not the same
                // sentence: nothing MATCHED, or there is nothing to match. The
                // hand-built version quoted the search either way, so a runner
                // with no workspaces at all was told that none of them matched
                // the empty string.
                if search.isEmpty {
                    Text("This runner has no workspaces yet.")
                } else {
                    Text("No workspace matches “\(search)”.")
                }
            }
        } else {
            grid
        }
    }

    /// One card of the fleet this app is connected to. The same view for the
    /// shown cards and the hidden ones: hiding changes where a card is drawn,
    /// not what it is, and two card views would be two things to keep in step.
    private func liveCard(_ index: Int) -> some View {
        ShellOverviewCard(
            workspace: fleet.workspaces[index],
            index: index,
            isCurrent: index == current,
            isEmpty: index == current && currentIsEmpty,
            onOpen: { onOpen(index) })
            .id(index)
    }

    /// How wide the two columns of cards actually are.
    ///
    /// A section header is offered the whole container, and the columns are
    /// `.fixed` and therefore CENTERED in it — so a header at `maxWidth:
    /// .infinity` starts 27 points to the left of the cards it stands over on
    /// a 402-point display, which on this one clipped the first letter of the
    /// runner's name off the edge of the screen. Measured, not reasoned: the
    /// cards' left edge is at x=28 and the header's text was at x=0.
    ///
    /// Said as the same expression the columns are built from, so the two
    /// cannot drift.
    static var gridWidth: CGFloat { ShellCardFace.size.width * 2 + PaneMetrics.card }

    /// A section heading: the runner, and one line about how current it is.
    ///
    /// Left-aligned across the whole grid rather than sitting over one column,
    /// and not pinned. A pinned header would sit over the cards as they scroll
    /// under it — which is right in a `List` of rows and wrong here, because
    /// the thing it would cover is the amber outline that says which workspace
    /// you are in.
    private func header(_ name: String, detail: String?) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: PaneMetrics.step) {
            Text(name)
                .font(.subheadline.weight(.semibold))
                .lineLimit(1)
                .truncationMode(.middle)
            if let detail {
                Text(detail)
                    // Mono for the same reason the card's subtitle is: this
                    // half of the line is data about a machine.
                    .font(.system(.caption2, design: .monospaced))
                    .foregroundStyle(.tertiary)
                    .lineLimit(1)
            }
            Spacer(minLength: 0)
        }
        .frame(width: Self.gridWidth, alignment: .leading)
        .padding(.top, PaneMetrics.card)
        .accessibilityElement(children: .combine)
        .accessibilityIdentifier("shell-section-\(name)")
    }

    /// The way back from hiding, which is the whole reason this is a section
    /// and not a filter. Collapsed by default; the count is on the header so
    /// it is answerable without opening it.
    private var hiddenHeader: some View {
        Button {
            withAnimation(.snappy(duration: 0.2)) { hiddenShown.toggle() }
        } label: {
            HStack(spacing: PaneMetrics.step) {
                Image(systemName: hiddenShown ? "chevron.down" : "chevron.right")
                    .font(.caption2.weight(.semibold))
                    .foregroundStyle(.tertiary)
                Text("Hidden")
                    .font(.subheadline.weight(.semibold))
                Text("\(hidden.count)")
                    .font(.system(.caption2, design: .monospaced))
                    .foregroundStyle(.tertiary)
                Spacer(minLength: 0)
            }
            .contentShape(.rect)
        }
        .buttonStyle(.plain)
        .frame(width: Self.gridWidth, alignment: .leading)
        .padding(.top, PaneMetrics.card)
        .accessibilityIdentifier("shell-hidden-section")
        // "Hidden Workspaces" and not "Show Hidden Workspaces": the control is
        // a disclosure, so what it does depends on which way it is, and a
        // label that named one direction would be wrong half the time. The
        // count is the value, which is what VoiceOver reads after the name.
        .accessibilityLabel("Hidden Workspaces")
        .accessibilityValue("\(hidden.count)")
    }

    /// How old this runner's answer is, in words.
    ///
    /// **Never "just now" for a runner nobody has heard from.** Nil is "not
    /// told", which is a different thing from recent and must not be drawn as
    /// it — the rule `FleetSnapshot.observedAt` states, kept here.
    static func lastSeen(_ date: Date?, now: Date = Date()) -> String {
        guard let date else { return "Not seen yet" }
        return "Last seen \(date.formatted(.relative(presentation: .numeric)))"
    }

    private var grid: some View {
        ScrollView {
            LazyVGrid(
                columns: [
                    GridItem(.fixed(ShellCardFace.size.width), spacing: PaneMetrics.card),
                    GridItem(.fixed(ShellCardFace.size.width), spacing: PaneMetrics.card),
                ],
                spacing: PaneMetrics.card
            ) {
                // The runner you are ON, first and unlabeled unless there is
                // something under it to tell it apart from.
                Section {
                    ForEach(order, id: \.self) { index in liveCard(index) }
                } header: {
                    if let liveServer, !groups.isEmpty {
                        header(liveServer, detail: "Connected")
                    }
                }

                // Then what this runner has been told to stop showing.
                if !hidden.isEmpty {
                    Section {
                        if hiddenShown {
                            ForEach(hidden, id: \.self) { index in liveCard(index) }
                        }
                    } header: {
                        hiddenHeader
                    }
                }

                // Then every other runner, as it was when this app last saw
                // it. These are the only cards in this grid that are not a
                // place you can go by swiping: a tap crosses runners, and
                // `ShellScreen` is what says what that costs.
                ForEach(groups) { group in
                    Section {
                        ForEach(group.order(matching: search), id: \.self) { index in
                            let workspace = group.workspaces[index]
                            ShellElsewhereCard(
                                workspace: workspace,
                                onOpen: { onCross(group, workspace) })
                                .id("\(group.id)/\(workspace.id)")
                        }
                    } header: {
                        header(group.name, detail: Self.lastSeen(group.lastSeen))
                    }
                }
            }
            .padding(.vertical, PaneMetrics.edge)
        }
        .scrollDismissesKeyboard(.immediately)
        // Nothing of its own under the cards: the ground this screen draws is
        // one layer, applied once, at the bottom of `body`. A scroll view that
        // also painted one would put the system's background between that
        // ground and the grid, and the reveal behind the lifted page would go
        // opaque.
        .scrollContentBackground(.hidden)
        .onScrollGeometryChange(for: Bool.self) { geometry in
            // "At the top" including the content inset the search field and
            // the navigation bar occupy, which is why this is not a bare
            // `contentOffset.y <= 0`.
            geometry.contentOffset.y <= -geometry.contentInsets.top + 0.5
        } action: { _, isAtTop in
            atTop = isAtTop
        }
        // SIMULTANEOUS, and gated on the top of the scroll.
        //
        // The drag back down is the gesture that opened this, reversed, and it
        // has to keep working now that the header it used to live on is a
        // navigation bar. Attached as an ordinary gesture it would compete
        // with the scroll — two vertical gestures over one view, and the
        // scroll should always win that — so it runs alongside instead, and
        // only acts where a downward drag has nothing else to mean.
        .simultaneousGesture(
            DragGesture(minimumDistance: 20)
                // The first movement of each drag, and only the first: this is
                // where "was the grid at its top" is still a fact about the
                // gesture rather than about what the gesture has since done.
                .onChanged { _ in if pullBegan == nil { pullBegan = atTop } }
                .onEnded { value in
                    let began = pullBegan ?? atTop
                    pullBegan = nil
                    guard began, atTop else { return }
                    // Predominantly downward as well as far enough. A diagonal
                    // flick across the cards is not a pull-down, and the
                    // distance alone cannot tell the two apart.
                    guard value.translation.height > 40,
                        value.translation.height > abs(value.translation.width)
                    else { return }
                    onDismiss()
                })
    }
}
