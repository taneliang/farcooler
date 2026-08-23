import SwiftUI

/// The status indicator.
///
/// Three rules, and everything here follows from them.
///
/// **Color means one thing.** Selection is structural — where am I — and
/// status is semantic — what is happening. Encoding both in color put an
/// orange label on an accent-blue row and made a list of four terminals look
/// like a warning panel. Selection is now neutral, and color belongs entirely
/// to status.
///
/// **Silence is the default.** An idle terminal shows no indicator at all.
/// Most terminals are idle most of the time, and an icon on every row is an
/// icon on none: the eye has nowhere to land. Marking only what has something
/// to say is what makes the one that does unmissable.
///
/// **One shape, one size.** A filled dot. Not a moon for idle and a gearwheel
/// for working — literal imagery reads as a sticker sheet, and a column of
/// different silhouettes will not align no matter how carefully it is spaced.
///
/// "One size" was the rule and it had stopped being the practice: eleven call
/// sites passed 6, 7, 8, 10 and 14 between them, and the declared default of 9
/// was used by nobody, so the app drew six diameters of a glyph whose whole
/// argument is that it is always the same mark. There are two situations, not
/// six, and they are named below.
struct StatusGlyph: View {
    /// A dot in a row of text — a sidebar row, a palette result, a tool call,
    /// a pane's header strip. Every one of these sits beside 11–13pt text.
    ///
    /// Eight, because the sidebar reserves a marker column of exactly this
    /// width and the feed lines under a terminal row indent past it; the one
    /// place in the app where the glyph's size is load-bearing for alignment
    /// gets to pick the number.
    static let inline: CGFloat = 8

    /// A dot standing in for a whole pane, centered with nothing else in it.
    ///
    /// Two places, and they are the same situation: a pane whose terminal is
    /// gone, and a tile waiting for one to start. Each shows this mark, a
    /// label and at most two buttons — at row size that reads as a speck of
    /// dust rather than as the subject of the screen.
    static let hero: CGFloat = 14

    let status: Status
    var size: CGFloat = StatusGlyph.inline

    var body: some View {
        Group {
            if status.animates {
                mark.modifier(Breathing())
            } else {
                mark
            }
        }
        .help(status.label)
        .accessibilityLabel(status.label)
    }

    private var mark: some View {
        // The column is reserved whether or not anything occupies it, so names
        // align down the list rather than stepping in and out.
        ZStack {
            switch status {
            case .idle, .running, .exited:
                // Nothing to say.
                Color.clear
            case .lost, .failed, .unreadable:
                // Hollow: something is missing, and the shape says so before
                // the color does. `unreadable` shares the shape because what is
                // missing is the same thing — an answer — and differs in color,
                // because one of them is a problem to act on and the other is a
                // runner that has not replied yet.
                Circle()
                    .strokeBorder(status.tint, lineWidth: 1.5)
            default:
                Circle().fill(status.tint)
            }
        }
        .frame(width: size, height: size)
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
    /// Warm for "you", green for "finished", red for "missing", and nothing
    /// else. Four colors in an application is already generous.
    ///
    /// On `Status` rather than private to `StatusGlyph`, because the glyph was
    /// not the only thing painting a status. A collapsed worktree, a hidden
    /// section and a switcher tile each drew "something in here wants you" as
    /// solid orange while the glyph for the very same terminal drew red or
    /// green — so a failed agent was orange collapsed and red expanded, and the
    /// switcher showed both at once, a red dot inside an orange tile. One rule,
    /// one place, and there is nowhere left to disagree from.
    var tint: Color {
        switch self {
        case .blocked: return .orange
        case .done: return .green
        // `failedRun` and `failedTurn` are filled rather than hollow, like
        // `done`: both are a definite outcome, not a missing answer. Left out
        // of the shape switch in `StatusGlyph` so they fall to the
        // filled-circle default there. Red rather than green is the whole point
        // — a turn that died and a turn that worked were the same dot until
        // this existed.
        case .lost, .failed, .failedRun, .failedTurn: return .red
        // Not red. Red is reserved for something that has gone wrong and wants
        // a decision; a runner that has not answered yet is neither, and
        // painting the whole fleet red every time tmux is busy is how a colour
        // stops meaning anything.
        case .unreadable: return .secondary
        case .working, .starting: return .secondary
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
                Circle().fill(status.tint).frame(width: 6, height: 6)
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
