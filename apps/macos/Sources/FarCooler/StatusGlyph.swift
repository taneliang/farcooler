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
struct StatusGlyph: View {
    let status: Status
    var size: CGFloat = 9

    var body: some View {
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
                // machine that has not replied yet.
                Circle()
                    .strokeBorder(color, lineWidth: 1.5)
            default:
                Circle().fill(color)
            }
        }
        .frame(width: size, height: size)
        .opacity(status == .working ? 0.85 : 1)
        .symbolEffect(.pulse, options: .repeating, isActive: status.animates)
        .help(status.label)
        .accessibilityLabel(status.label)
    }

    /// Warm for "you", green for "finished", red for "missing", and nothing
    /// else. Four colors in an application is already generous.
    private var color: Color {
        switch status {
        case .blocked: return .orange
        case .done: return .green
        case .lost, .failed: return .red
        // Not red. Red is reserved for something that has gone wrong and wants
        // a decision; a machine that has not answered yet is neither, and
        // painting the whole fleet red every time tmux is busy is how a colour
        // stops meaning anything.
        case .unreadable: return .secondary
        case .working, .starting: return .secondary
        default: return .clear
        }
    }
}

/// The count of everything waiting on you.
///
/// Deliberately not a filled capsule. A solid block of color in the header
/// competes with the one place color is supposed to matter — the row that
/// actually needs you — and a badge that shouts louder than the thing it points
/// at is pointing at itself.
struct AttentionBadge: View {
    let count: Int

    var body: some View {
        if count > 0 {
            HStack(spacing: 4) {
                Circle().fill(Color.orange).frame(width: 6, height: 6)
                Text("\(count)")
                    .font(.system(size: 11, weight: .medium))
                    .foregroundStyle(.secondary)
                    .monospacedDigit()
            }
        }
    }
}
