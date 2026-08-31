import AgentKit
import SwiftUI

/// The status indicator.
///
/// **This is `GlanceMarkView` now.** The rules below are the ones this file
/// kept for itself before the product had one mark; §03 of the glance spec is
/// the vocabulary the phone, the widgets and the wrist already draw, and the
/// argument for adopting it here is the argument this file already made about
/// its own six diameters, one level up: a mark is worth having only if it is
/// the same mark everywhere, or it is something to be read rather than
/// recognised. See `Status.glanceMark` for what carried across and what did
/// not.
///
/// Three rules, and everything here still follows from them.
///
/// **Color means one thing.** Selection is structural — where am I — and
/// status is semantic — what is happening. Encoding both in color put an
/// orange label on an accent-blue row and made a list of four terminals look
/// like a warning panel. Selection is now neutral, and color belongs entirely
/// to status. §01 states the strong form of the same rule and this file now
/// obeys it: the one saturated hue is `GlancePalette.amber`, it means "an
/// agent is waiting on you", and nothing else in the app may be amber.
///
/// **Silence is no longer the default, and that is the one behaviour this
/// change reverses.** An idle terminal used to draw nothing at all, on the
/// argument that "an icon on every row is an icon on none". §03 draws quiet as
/// a 1pt hairline ring at every size — "Drawn, not omitted" — and the phone's
/// ribbon has drawn it that way since `8b08c1f`. A hairline is not the filled
/// dot that argument was made against: it is a ring one point wide in
/// `GlancePalette.ink2`, which reserves the column, keeps the twelve states a
/// person learns on the phone all present on the Mac, and still leaves amber
/// as the only thing in the list with any weight. Run `GlanceTests` and open
/// `.build/glance/status-column-*.png` before taking a view on it; the whole
/// point of that target is that this is a question about pixels.
///
/// **One shape, one size.** A ring, with a core inside it while the agent is
/// producing. Not a moon for idle and a gearwheel for working — literal
/// imagery reads as a sticker sheet, and a column of different silhouettes
/// will not align no matter how carefully it is spaced.
///
/// "One size" was the rule and it had stopped being the practice: eleven call
/// sites passed 6, 7, 8, 10 and 14 between them, and the declared default of 9
/// was used by nobody, so the app drew six diameters of a glyph whose whole
/// argument is that it is always the same mark. There are two situations, not
/// six, and they are named below — and they are now `GlanceMarkSize` cases
/// rather than numbers this file chose, so the Mac cannot drift from the
/// ladder the phone is drawn on.
struct StatusGlyph: View {
    /// A dot in a row of text — a sidebar row, a palette result, a tool call,
    /// a pane's header strip. Every one of these sits beside 11–13pt text.
    ///
    /// Eight, because the sidebar reserves a marker column of exactly this
    /// width and the feed lines under a terminal row indent past it; the one
    /// place in the app where the glyph's size is load-bearing for alignment
    /// gets to pick the number — and §03's smallest diameter is the same 8,
    /// so the number this app picked and the number the spec picked agree and
    /// only one of them is written down.
    static let inline: CGFloat = GlanceMarkSize.ribbon.diameter

    /// Where this mark is drawn on §03's ladder, or beneath it.
    ///
    /// Two cases and not one, mirroring `GlanceMarkView`'s own two
    /// initialisers. The spec's six diameters cover a mark that stands for a
    /// terminal; the roll-up dots on a collapsed workspace, a hidden section
    /// and the window's attention badge are 5 and 6 points, which is below
    /// anything §03 names, and those numbers are the ones this app already
    /// laid out around. `GlanceMarkView(_:inAppDiameter:)` is the sanctioned
    /// way to say that — it takes the 8pt stroke collapse and drops the core,
    /// which is exactly right for a dot that stands for a group rather than
    /// for one agent.
    private enum Geometry {
        case spec(GlanceMarkSize)
        case inApp(CGFloat)

        var diameter: CGFloat {
            switch self {
            case .spec(let size): size.diameter
            case .inApp(let d): d
            }
        }
    }

    /// Resolved here rather than by a dynamic colour provider, for the reason
    /// `GlanceInk` gives: the provider APIs are `NSColor`/`UIColor` and the
    /// watch has neither, so `@Environment(\.colorScheme)` is the one
    /// mechanism the whole palette shares. Only `outcome` reads it — the mark
    /// itself resolves its own.
    @Environment(\.colorScheme) private var scheme

    let status: Status
    private let geometry: Geometry

    /// One of §03's six diameters. `.ribbon` (8pt) in a row of text; `.lone`
    /// (15pt) for a dot standing in for a whole pane, centered with nothing
    /// else in it — a pane whose terminal is gone, and a tile waiting for one
    /// to start. Each of those shows this mark, a label and at most two
    /// buttons, and at row size that reads as a speck of dust rather than as
    /// the subject of the screen.
    init(status: Status, size: GlanceMarkSize = .ribbon) {
        self.status = status
        self.geometry = .spec(size)
    }

    /// A roll-up dot, smaller than §03 goes. See `Geometry`.
    init(status: Status, inAppDiameter: CGFloat) {
        self.status = status
        self.geometry = .inApp(inAppDiameter)
    }

    var body: some View {
        Group {
            if status.animates {
                mark.modifier(Breathing())
            } else {
                mark
            }
        }
        .help(status.label)
        // The Mac's own word, not `GlanceMark.phrase`, and the mark below is
        // therefore drawn `decorative` so it does not say a second one. Every
        // state §03 has a slot for maps onto a phrase that is broader than the
        // one this app already knew — "Nothing wanted" for a shell that
        // exited — and the states it has NO slot for are precisely the ones
        // whose whole content is the word: a screen reader that heard
        // "Nothing wanted" for a build that died overnight would be told the
        // opposite of the truth.
        .accessibilityLabel(status.label)
    }

    private var mark: some View {
        // The column is reserved whether or not anything occupies it, so names
        // align down the list rather than stepping in and out.
        ZStack {
            if let glance = status.glanceMark {
                switch geometry {
                case .spec(let size): GlanceMarkView(glance, size: size, decorative: true)
                case .inApp(let d): GlanceMarkView(glance, inAppDiameter: d)
                }
            } else {
                outcome
            }
        }
        .frame(width: geometry.diameter, height: geometry.diameter)
    }

    /// What this app draws for a state §03 has no slot for.
    ///
    /// See `Status.glanceMark` for the argument. These four keep exactly the
    /// drawing they had before the mark arrived — red for a turn that died or
    /// a terminal that is gone — because the alternative on offer was the
    /// quiet hairline, and a hairline is the mark's way of saying "nothing is
    /// wanted from you", which about an overnight build that failed is not a
    /// missing answer but a wrong one.
    ///
    /// **A finished turn is no longer one of them.** `done` draws the review
    /// ring now, so it never reaches here; `glanceMark` says why.
    @ViewBuilder
    private var outcome: some View {
        switch status {
        case .lost, .failed:
            // Hollow: something is missing, and the shape says so before the
            // color does.
            Circle().strokeBorder(status.tint(scheme), lineWidth: 1.5)
        default:
            Circle().fill(status.tint(scheme))
        }
    }
}

/// A slow breath on a mark that is still changing.
///
/// This used to be `.symbolEffect(.pulse)`, applied to a `ZStack` of `Circle`s.
/// `symbolEffect` animates the image content of an SF Symbol and nothing else,
/// so on a shape it is inert — it was the only `symbolEffect` in the app, and
/// `working` and `starting` had been documented as moving while standing
/// perfectly still. The static `opacity(0.85)` sitting beside it was the
/// stand-in for the motion that never arrived, and it survives as what a
/// reader who has asked for less motion sees.
///
/// One `repeatForever` on a real property rather than a clock: this runs on the
/// render server and rebuilds no views, where `Ticking`'s once-a-second wake-up
/// would be a blink rather than a breath and would cost a view update per row
/// per second. The modifier is applied inside an `if`, so leaving the animating
/// states tears the animation down with the branch instead of leaving a
/// `repeatForever` oscillating on a dot that has settled.
private struct Breathing: ViewModifier {
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var dim = false

    func body(content: Content) -> some View {
        if reduceMotion {
            // The value that was on screen before this existed. Motion is the
            // part being asked for less of, not the dimming.
            content.opacity(0.85)
        } else {
            content
                .opacity(dim ? 0.45 : 1)
                .animation(.easeInOut(duration: 1.1).repeatForever(autoreverses: true), value: dim)
                .onAppear { dim = true }
        }
    }
}

extension Status {
    /// This status as the one mark the whole product draws, or nil where §03
    /// has no slot for it.
    ///
    /// **Nil is a refusal, not an omission.** The glance vocabulary is three
    /// axes: a ring that says whether YOUR attention is wanted and what for, a
    /// core that says whether the AGENT is producing, and a dash that says the
    /// channel is broken. Eight of this app's twelve statuses are one of those
    /// combinations exactly, and they are mapped below. Four are not, and the
    /// nearest mark for every one of them is the quiet hairline — which is the
    /// mark for "nothing is wanted from you", spoken by VoiceOver as exactly
    /// that. Drawing it on a build that died overnight would make this app
    /// state the opposite of what it knows in a vocabulary that is now trusted
    /// on three platforms. So those four keep the drawing they already had,
    /// `StatusGlyph.outcome` draws them, and the gap is a question for the
    /// design document rather than one for this file to answer quietly.
    ///
    /// **`done` used to be a fifth, and it is not any more.** It asked whether
    /// `GlanceMark.Attention.toReview` — then "Never an agent's state" — could
    /// be narrowed to the diff counts it was actually written about, and the
    /// answer came back yes. The ban's stated reason was that a per-agent
    /// review tier would have to be INVENTED, because `reviewsWaiting` is a
    /// fleet scalar and `unreadDiff` is per workspace; that reason never
    /// reached `done`, which the daemon sends per terminal
    /// (`Terminal.status`, `AgentActivity.done`), and which this codebase
    /// already treats as latched rather than passing — `confidence(in:at:)`
    /// vouches for `blocked` and `done` at any age and stops vouching only for
    /// `working`. So `done` maps onto the review ring below, the green dot it
    /// wore here is gone, and this app, the phone and Android all put a
    /// finished turn in the review tier — Android in the hue only, where it is
    /// still drawn by `agentTint` rather than by a mark. The ban still stands
    /// for `reviewsWaiting` and `unreadDiff`, which still have no per-agent
    /// version and must not be given an invented one.
    ///
    /// The four that remain, and what the mark would have to grow to take
    /// them:
    ///
    ///   - `failedRun`, `failedTurn` — a definite bad outcome. Red exists
    ///     precisely because "a turn that died and a turn that worked were the
    ///     same dot until this existed"; folding them into the hairline would
    ///     put that bug back with the vocabulary's name on it.
    ///   - `lost`, `failed` — the terminal is gone, or never started. Tempting
    ///     to draw as a broken LINK, and wrong: a dashed ring is "the channel
    ///     is not carrying anything right now", which is a claim about
    ///     reachability that resolves on its own. These do not.
    ///
    /// **The blocked/needs-you mapping is the load-bearing one** and it is
    /// identical to the phone's, `GlanceMark(agent:)` and
    /// `GlanceMark(_ shell:)` both: a heavy amber ring with NO core, because
    /// §03 gives the ring to the person's side and the core to the agent's,
    /// and a blocked agent is stopped at a prompt.
    var glanceMark: GlanceMark? {
        switch self {
        case .blocked: GlanceMark(attention: .needsYou, core: .atAPrompt)
        // Producing. `starting` joins `working` because it already drew
        // identically to it — a filled secondary dot, breathing — and because
        // a pane coming up is a pane with something happening in it.
        case .working, .starting: GlanceMark(attention: .quiet, core: .producing)
        // At a prompt, and nothing wanted. The three states this app used to
        // draw as nothing at all.
        case .idle, .running, .exited: GlanceMark(attention: .quiet, core: .atAPrompt)
        // "The runner did not answer, so this row is not saying anything" —
        // `Status.unreadable`'s own words, and `GlanceMark.Link.broken`'s
        // ("Unreachable, or a claim about the present that has gone stale")
        // are the same sentence. The core is `nil` rather than `.atAPrompt`
        // for the same reason: nil is a surface DECLINING to state that axis,
        // which is exactly what "could not look" means, and it is the
        // difference §03 makes audible even though it draws the same.
        case .unreadable: GlanceMark(attention: .quiet, core: nil, link: .broken)
        // The turn ended and nobody has looked at it. The same mapping the
        // phone makes from the wire's own word, `GlanceMark(agent:)`'s
        // `case "done"`, and the reason it is allowed is in the comment above.
        case .done: GlanceMark(attention: .toReview, core: .atAPrompt)
        case .failed, .failedRun, .failedTurn, .lost: nil
        }
    }

    /// Warm for "you", the low-chroma review ink for "finished and unread",
    /// red for "missing", and nothing else. Four colors in an application is
    /// already generous, and this is now three.
    ///
    /// On `Status` rather than private to `StatusGlyph`, because the glyph was
    /// not the only thing painting a status. A collapsed worktree, a hidden
    /// section and a switcher tile each drew "something in here wants you" as
    /// solid orange while the glyph for the very same terminal drew red or
    /// green — so a failed agent was orange collapsed and red expanded, and the
    /// switcher showed both at once, a red dot inside an orange tile. One rule,
    /// one place, and there is nowhere left to disagree from.
    ///
    /// **A function of the appearance, because the glance inks are.** §01 is
    /// explicit that light mode is "Not a filter flip" — amber darkens to hold
    /// its contrast on a pale backdrop, and review darkens with it — so the
    /// two colours in here that belong to the glance system have to be
    /// resolved against a `ColorScheme` rather than being constants. Red does
    /// not vary, and it does not come from `GlancePalette` because §01 has no
    /// figure for it: it is this app's own ink for the four states §03 does
    /// not cover, and `glanceMark` is where that is argued.
    ///
    /// **Green is gone.** `done` used to be painted `.green` here and drawn as
    /// a filled green dot by `StatusGlyph.outcome`. It is a review-tier state
    /// now, so the glyph draws it through `glanceMark` and never reaches this
    /// function at all — but a collapsed worktree's roll-up and the palette
    /// tile's wash still ask, and they must not go on saying green about a
    /// terminal whose dot no longer is. Hence `GlancePalette.review`, which is
    /// the exact ink `GlanceMarkView` strokes that ring in.
    func tint(_ scheme: ColorScheme) -> Color {
        switch self {
        // The one saturated hue, from the one place it is written down.
        case .blocked: return GlancePalette.amber(scheme)
        // The other glance ink, and the same value `GlanceMarkView` strokes
        // the review ring in — so a `done` terminal's glyph and the tile
        // washed behind it cannot come out two different colours.
        case .done: return GlancePalette.review(scheme)
        // `failedRun` and `failedTurn` are filled rather than hollow: both are
        // a definite outcome, not a missing answer. Left out of the shape
        // switch in `StatusGlyph` so they fall to the filled-circle default
        // there. Red rather than the review ink is the whole point — a turn
        // that died and a turn that worked were the same dot until this
        // existed, and they must not become the same dot again now that the
        // one that worked has a mark.
        case .lost, .failed, .failedRun, .failedTurn: return .red
        // Not red. Red is reserved for something that has gone wrong and wants
        // a decision; a runner that has not answered yet is neither, and
        // painting the whole fleet red every time tmux is busy is how a colour
        // stops meaning anything.
        case .unreadable: return GlancePalette.ink2(scheme)
        case .working, .starting: return GlancePalette.ink2(scheme)
        default: return .clear
        }
    }

    /// The one status a roll-up should wear, out of everything waiting inside
    /// it.
    ///
    /// A collapsed worktree cannot show six dots, so it shows the one that
    /// decides the color — and the ranking is by how much a human is being
    /// waited on right now, which is the question the mark answers. `blocked`
    /// first: an agent stalled mid-turn is the state this application exists
    /// for, and it is the only one where the work resumes the moment you look.
    /// A failure is next, because something stopped; `done` last, because
    /// nothing is waiting on anything — it is finished and merely unread.
    ///
    /// Returns nil when nothing in the group wants attention, which is also
    /// when nothing should be drawn.
    static func mostUrgent(in statuses: some Sequence<Status>) -> Status? {
        statuses.filter(\.wantsAttention).min { rank($0) < rank($1) }
    }

    private static func rank(_ status: Status) -> Int {
        switch status {
        case .blocked: return 0
        case .lost, .failed, .failedRun, .failedTurn: return 1
        default: return 2
        }
    }
}

/// The count of everything waiting on you.
///
/// Deliberately not a filled capsule. A solid block of color in the header
/// competes with the one place color is supposed to matter — the row that
/// actually needs you — and a badge that shouts louder than the thing it points
/// at is pointing at itself.
///
/// Given the statuses rather than a count, so the dot can be the color the row
/// it points at will turn out to be — see `Status.mostUrgent(in:)`. A header
/// that says orange and a row that says red are a header pointing somewhere
/// else.
struct AttentionBadge: View {
    let waiting: [Status]

    var body: some View {
        if let status = Status.mostUrgent(in: waiting) {
            HStack(spacing: 4) {
                // The same mark the row it points at will draw, at the size
                // this badge already reserved. `StatusGlyph`'s in-app
                // initialiser rather than a bare `Circle`, so a header that
                // says amber and a row that says amber are one drawing and not
                // two that happen to agree today.
                StatusGlyph(status: status, inAppDiameter: 6)
                Text("\(waiting.count)")
                    .font(.system(size: 11, weight: .medium))
                    .foregroundStyle(.secondary)
                    .monospacedDigit()
            }
        }
    }
}

/// Text that recomputes once a second, and nothing else on the screen with it.
///
/// The clock a row shows — `Working 12m` — is a function of the wall clock,
/// which is the one input SwiftUI cannot observe. Nothing about a working row
/// changes from one second to the next, so the view is never invalidated, and
/// the duration froze at whatever it said when some unrelated event last
/// forced a redraw. Clicking a different pane was that event, which is exactly
/// how the complaint was phrased. A `TimelineView` keeps a clock of its own
/// and hands the time in, so the string is a function of an input that moves.
///
/// Scoped to ONE label, deliberately. A timeline wrapped around the sidebar
/// would rebuild every row of every workspace once a second to move a handful
/// of characters. `WorkingRow` in `AgentRows` already draws the same
/// distinction, for the same reason.
///
/// One second, because that is as often as the string can change: a redraw at
/// the display's refresh rate would be sixty of them to move a number sixty
/// times less often.
///
/// `paused` is for a row whose clock is not running at all — an idle or
/// finished terminal has no duration to show, and scheduling a wake-up per
/// second per resting row would be the sidebar paying for a number that is not
/// there.
struct Ticking<Content: View>: View {
    var paused: Bool = false
    @ViewBuilder let content: (Date) -> Content

    var body: some View {
        if paused {
            content(.now)
        } else {
            TimelineView(.periodic(from: .now, by: 1)) { context in content(context.date) }
        }
    }
}
