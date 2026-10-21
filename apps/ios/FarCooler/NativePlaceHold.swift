import SwiftUI

/// Reports a row's y in the scroll view while its place is being watched.
struct NativePlaceMarker: ViewModifier {
    let id: String
    let watching: Bool
    let moved: (String, CGFloat) -> Void
    let gone: (String) -> Void

    func body(content: Content) -> some View {
        content.overlay {
            if watching {
                Color.clear  // style-exempt: a measuring layer, nothing drawn
                    .allowsHitTesting(false)
                    .accessibilityHidden(true)
                    .onGeometryChange(for: CGFloat.self) { $0.frame(in: .scrollView).minY } action: { moved(id, $0) }
                    .onDisappear { gone(id) }
                    #if DEBUG
                    .onGeometryChange(for: CGFloat.self) { $0.size.height.rounded() } action: { was, now in
                        if abs(now - was) > 50 { NativeScrollProbe.note("H\(id):\(Int(was))>\(Int(now))") }
                    }
                    #endif
            }
        }
    }
}
