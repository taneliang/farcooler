import AppKit
import WebKit

/// Web panes in a capture (ov-435). A `WKWebView` draws in WebKit's own
/// process, so `cacheDisplay` leaves its rectangle empty: each one's own
/// snapshot is drawn over the bitmap where it sits. Only the page itself;
/// everything around it is still the window's own drawing.
extension RealWindowCaptures {
    static func withWebPages(_ rep: NSBitmapImageRep, in root: NSView) async -> NSBitmapImageRep {
        let pages = webViews(in: root)
        guard !pages.isEmpty else { return rep }
        // An appearance change re-renders the page in its own process.
        try? await Task.sleep(for: .seconds(1))
        for page in pages {
            guard let image = try? await page.takeSnapshot(configuration: nil),
                let context = NSGraphicsContext(bitmapImageRep: rep)
            else { continue }
            var rect = page.convert(page.bounds, to: root)
            if root.isFlipped { rect.origin.y = root.bounds.height - rect.maxY }
            NSGraphicsContext.saveGraphicsState()
            NSGraphicsContext.current = context
            // The context may be in pixels or in points, depending on the rep.
            let scale = CGFloat(rep.pixelsWide) / rep.size.width
            if context.cgContext.userSpaceToDeviceSpaceTransform.a < scale - 0.01 {
                context.cgContext.scaleBy(x: scale, y: scale)
            }
            image.draw(in: rect)
            context.flushGraphics()
            NSGraphicsContext.restoreGraphicsState()
        }
        return rep
    }

    private static func webViews(in view: NSView) -> [WKWebView] {
        if let web = view as? WKWebView { return [web] }
        return view.subviews.flatMap(webViews)
    }
}
