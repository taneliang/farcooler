import Foundation
import SwiftUI

extension GlancePalette {
    /// The tint for one rung of `FleetSnapshot.Glance`.
    ///
    /// **The single replacement for the two `glanceTint` functions** that used
    /// to live in `FleetWidget.swift` and `WatchFleetWidget.swift`, each of
    /// which carried a comment explaining that it had to be a copy because a
    /// color is a SwiftUI type. It does not have to be a copy any more; it has
    /// to be in a SwiftUI file, and this is one.
    ///
    /// `AnyShapeStyle` because the three tints are not all colors: the working
    /// rung stays a hierarchical style, which is what lets the all-clear case
    /// recede against whatever wallpaper or watch face is behind it rather than
    /// sitting at a fixed gray that vanishes on one and shouts on the other.
    /// That reasoning survives the palette — §01 has no color for "getting on
    /// with it", and it should not, because the answer is "whatever is behind
    /// this, slightly".
    public static func tint(_ glance: FleetSnapshot.Glance, _ scheme: ColorScheme) -> AnyShapeStyle
    {
        switch glance {
        case .blocked: AnyShapeStyle(amber(scheme))
        case .failed: AnyShapeStyle(failed)
        case .review: AnyShapeStyle(review(scheme))
        case .working: AnyShapeStyle(HierarchicalShapeStyle.tertiary)
        }
    }
}
