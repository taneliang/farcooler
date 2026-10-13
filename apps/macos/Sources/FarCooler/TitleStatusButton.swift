import AgentKit
import SwiftUI

// One look for every button in the title bar's status area (ov-511): Activity,
// Needs You, Failed, Running and In Review. They had two: Needs You was a
// `.borderless` button, which draws nothing under the pointer, and the others
// were the default style, whose hover plate is sized to the symbol, so a
// count stuck out past it. Here the plate is a capsule round the whole label,
// icon and count, with the same padding each side, and it is the hit target.

/// What a status button's plate is showing.
enum TitleStatusPlate: Equatable {
    case rest, hover, pressed

    /// The plate's fill: nothing at rest.
    func fill(_ contrast: ColorSchemeContrast) -> Color {
        switch self {
        case .rest: .clear
        case .hover: Fill.controlHover(contrast)
        case .pressed: Fill.controlPressed(contrast)
        }
    }
}

/// Measures every status button shares.
enum TitleStatusButtonMetrics {
    /// The plate's height: the regular toolbar's 28 pt control, inside the
    /// 36 pt band.
    static let height: CGFloat = 28
    /// Room between the label's ends and the plate's.
    static let padding: CGFloat = 8
    /// The icon's box, so a wide symbol (the eye) and a narrow one (the
    /// bubble) take the same room and sit on the same center.
    static let icon: CGFloat = 16
    /// Between the icon and its count.
    static let spacing: CGFloat = 4
    static let iconFont = Font.system(size: 13, weight: .medium)
}

/// A capture holds one button's plate down without a pointer (the toolbar
/// reads no environment from the view around it, so this is a global, set
/// before the window is made): the button's identifier and the plate to show.
@MainActor
enum TitleStatusForcing {
    static var plate: (id: String, plate: TitleStatusPlate)?
}

/// The status buttons' style: the label padded evenly, a capsule plate under
/// the pointer and a deeper one while pressed.
struct TitleStatusButtonStyle: ButtonStyle {
    /// The button's identifier, for a forced plate.
    let id: String

    func makeBody(configuration: Configuration) -> some View {
        Plate(id: id, label: configuration.label, pressed: configuration.isPressed)
    }

    private struct Plate: View {
        let id: String
        let label: ButtonStyleConfiguration.Label
        let pressed: Bool
        @State private var hovering = false
        @Environment(\.colorSchemeContrast) private var contrast
        @Environment(\.isEnabled) private var enabled

        var plate: TitleStatusPlate {
            if let forced = TitleStatusForcing.plate, forced.id == id { return forced.plate }
            if pressed { return .pressed }
            return hovering && enabled ? .hover : .rest
        }

        var body: some View {
            label
                .padding(.horizontal, TitleStatusButtonMetrics.padding)
                .frame(height: TitleStatusButtonMetrics.height)
                .background(Capsule().fill(plate.fill(contrast)))
                .contentShape(Capsule())
                .onHover { hovering = $0 }
        }
    }
}

/// A status button's label: an icon in its box, and a count when it has one.
struct TitleStatusLabel: View {
    let symbol: String
    var count: Int? = nil
    let tint: Color

    var body: some View {
        HStack(spacing: TitleStatusButtonMetrics.spacing) {
            Image(systemName: symbol)
                .font(TitleStatusButtonMetrics.iconFont)
                .frame(width: TitleStatusButtonMetrics.icon, height: TitleStatusButtonMetrics.icon)
            if let count { Text(TitleStatus.number(count)).monospacedDigit() }
        }
        .foregroundStyle(tint)
        .lineLimit(1)
        .fixedSize()
    }
}
