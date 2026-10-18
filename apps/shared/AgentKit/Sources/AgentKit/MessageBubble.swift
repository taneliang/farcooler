import SwiftUI

// The person's own message as a blue bubble (ov-610), the marketing site's
// (`services/site/public/index.html`, `.fe-msg`: background #0a59c7, white
// text, corners 13px 13px 4px 13px, the small bottom-right corner reading as
// the tail). One named color, shared by the phone and the Mac, with the
// site's value as its light face. Agent and orchestrator messages are not
// bubbles.
//
// The site sets its bubble at 11.5 px type; scaled to the apps' 17 pt body
// its corners are 19 and 6. The apps draw it from the radius scale instead,
// `Radius.large` and `Radius.small` (16 and 6): the same shape within 3 pt,
// on the steps every other surface uses (ov-611).

public enum MessageBubble {
    /// The site's `.fe-msg` blue, 0x0A59C7: the light face.
    public static let siteBlue: UInt32 = 0x0A59C7
    /// Dark: a step lighter, so the bubble separates from a black page,
    /// with white text still above AA (5.0:1).
    public static let darkBlue: UInt32 = 0x1A6AE0
    /// Increase Contrast, light: a step darker (8.4:1).
    public static let contrastBlue: UInt32 = 0x0848A6

    /// The fill's packed RGB for an appearance. Increase Contrast in dark
    /// takes the site's own blue (6.4:1), which is darker than dark's.
    public static func packed(_ scheme: ColorScheme, contrast: ColorSchemeContrast) -> UInt32 {
        switch (scheme, contrast) {
        case (.dark, .increased): siteBlue
        case (.dark, _): darkBlue
        case (_, .increased): contrastBlue
        default: siteBlue
        }
    }

    public static func fill(_ scheme: ColorScheme, contrast: ColorSchemeContrast = .standard) -> Color {
        color(packed(scheme, contrast: contrast))
    }

    /// The text on it: white, in every appearance.
    public static let text = Color.white
    /// White, packed, for the contrast check.
    public static let textPacked: UInt32 = 0xFFFFFF

    public static func color(_ packed: UInt32) -> Color {
        Color(
            .sRGB, red: Double((packed >> 16) & 0xFF) / 255, green: Double((packed >> 8) & 0xFF) / 255,
            blue: Double(packed & 0xFF) / 255)
    }

    /// The bubble's corners: `Radius.large`, and `Radius.small` at the
    /// bottom trailing corner, the tail.
    public static let radius = Radius.large
    public static let tail = Radius.small
    public static var shape: UnevenRoundedRectangle {
        UnevenRoundedRectangle(
            topLeadingRadius: radius, bottomLeadingRadius: radius, bottomTrailingRadius: tail,
            topTrailingRadius: radius, style: .continuous)
    }

    /// The widest a bubble gets, as a share of its column: Messages' measure.
    public static let widest: CGFloat = 0.75

    // MARK: - Contrast (WCAG 2.x)

    /// The relative luminance of a packed sRGB color.
    public static func luminance(_ packed: UInt32) -> Double {
        func channel(_ value: UInt32) -> Double {
            let c = Double(value & 0xFF) / 255
            return c <= 0.040_45 ? c / 12.92 : pow((c + 0.055) / 1.055, 2.4)
        }
        return 0.2126 * channel(packed >> 16) + 0.7152 * channel(packed >> 8) + 0.0722 * channel(packed)
    }

    /// The contrast ratio between two packed sRGB colors, 1 to 21.
    public static func contrast(_ a: UInt32, _ b: UInt32) -> Double {
        let (x, y) = (luminance(a), luminance(b))
        return (max(x, y) + 0.05) / (min(x, y) + 0.05)
    }
}

/// A column that gives its one child at most `fraction` of its width and
/// sets it at the trailing edge: a bubble as wide as its words, up to
/// Messages' three quarters.
public struct TrailingMeasure: Layout {
    public var fraction: CGFloat

    public init(fraction: CGFloat = MessageBubble.widest) { self.fraction = fraction }

    public func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        guard let child = subviews.first else { return .zero }
        let width = proposal.width ?? .infinity
        let size = child.sizeThatFits(ProposedViewSize(width: width.isFinite ? width * fraction : nil, height: proposal.height))
        return CGSize(width: width.isFinite ? width : size.width, height: size.height)
    }

    public func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        guard let child = subviews.first else { return }
        let size = child.sizeThatFits(ProposedViewSize(width: bounds.width * fraction, height: proposal.height))
        child.place(
            at: CGPoint(x: bounds.maxX - size.width, y: bounds.minY), proposal: ProposedViewSize(size))
    }
}
