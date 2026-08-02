import SwiftUI

/// The app's motion, in one place.
///
/// Springs rather than timed curves, and short ones. A disclosure that takes
/// a third of a second to open reads as the app thinking about it; the whole
/// point of a fold is that it costs nothing to look.
enum Motion {
    /// Opening, closing, and anything else that should feel instant.
    static let snap = Animation.spring(response: 0.22, dampingFraction: 0.82)
    /// A little overshoot, for something arriving rather than resizing.
    static let arrive = Animation.spring(response: 0.26, dampingFraction: 0.7)
}
