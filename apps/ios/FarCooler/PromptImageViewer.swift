import QuickLook
import SwiftUI
import UIKit

/// A prompt's images open at full size in Quick Look, grown out of the
/// thumbnail tapped, and a swipe down puts one back (ov-806), as Photos does:
/// the image follows the finger, the screen behind shows through as it goes,
/// and a release far enough down or a flick closes it, else it springs back.
/// Zoomed in, a drag pans. Pinch, markup and share are Quick Look's own.
///
/// Quick Look offers that swipe only to a preview that knows where it came
/// from: its delegate's `frameFor:inSourceView:`. SwiftUI's
/// `.quickLookPreview` presents with no source, so the only way out was a tap
/// on the image for the bar and then its X. This presents the controller
/// itself, with each thumbnail's frame as the source.
struct PromptImageViewer: UIViewRepresentable {
    @Binding var preview: URL?
    /// The images it pages through, in order.
    let files: [URL]
    /// Where each thumbnail is, to grow out of and swipe back into.
    let places: Places

    /// The thumbnails' frames in their window, and their pictures, by file.
    /// A class, so a scroll that moves them doesn't redraw the strip.
    @MainActor
    final class Places {
        struct Place {
            var frame: CGRect
            var picture: UIImage
        }

        var byFile: [URL: Place] = [:]
    }

    func makeCoordinator() -> Coordinator { Coordinator() }

    func makeUIView(context: Context) -> UIView {
        let view = UIView()
        view.isUserInteractionEnabled = false
        view.isHidden = true
        return view
    }

    func updateUIView(_ view: UIView, context: Context) {
        let viewer = context.coordinator
        viewer.places = places
        viewer.closed = { preview = nil }
        if let url = preview, viewer.shown == nil {
            viewer.open(url, in: files, from: view)
        } else if preview == nil, let shown = viewer.shown {
            viewer.shown = nil
            shown.dismiss(animated: true)
        }
    }

    @MainActor
    final class Coordinator: NSObject, QLPreviewControllerDataSource, QLPreviewControllerDelegate {
        var places = Places()
        var closed: () -> Void = {}
        weak var shown: QLPreviewController?
        private weak var window: UIWindow?
        /// The files as they were when it opened: a thumbnail that finishes
        /// loading meanwhile mustn't move the pages under the reader.
        private var files: [URL] = []

        func open(_ url: URL, in files: [URL], from view: UIView) {
            guard let window = view.window, var top = window.rootViewController else { return }
            while let above = top.presentedViewController, !above.isBeingDismissed { top = above }
            self.files = files.contains(url) ? files : [url]
            self.window = window
            let viewer = QLPreviewController()
            viewer.dataSource = self
            viewer.delegate = self
            viewer.currentPreviewItemIndex = self.files.firstIndex(of: url) ?? 0
            shown = viewer
            top.present(viewer, animated: true)
        }

        nonisolated func numberOfPreviewItems(in controller: QLPreviewController) -> Int {
            MainActor.assumeIsolated { files.count }
        }

        nonisolated func previewController(_ controller: QLPreviewController, previewItemAt index: Int) -> any QLPreviewItem {
            MainActor.assumeIsolated { files[index] as NSURL }
        }

        /// The thumbnail's frame: Quick Look grows the image out of it, and
        /// with it offers the swipe down that shrinks it back.
        nonisolated func previewController(
            _ controller: QLPreviewController, frameFor item: any QLPreviewItem,
            inSourceView view: AutoreleasingUnsafeMutablePointer<UIView?>
        ) -> CGRect {
            MainActor.assumeIsolated {
                guard let url = item.previewItemURL, let place = places.byFile[url], let window else { return .zero }
                view.pointee = window
                return place.frame
            }
        }

        /// The thumbnail as the strip shows it, cropped to its square, to
        /// draw as the image grows out of it and shrinks back. Without one
        /// Quick Look took two seconds to open and its animations never
        /// settled; with the whole picture and its square as `contentRect`,
        /// the image shrank to a square over twice the thumbnail's size from
        /// its top left corner, and hung there before it went.
        nonisolated func previewController(
            _ controller: QLPreviewController, transitionImageFor item: any QLPreviewItem,
            contentRect: UnsafeMutablePointer<CGRect>
        ) -> UIImage? {
            MainActor.assumeIsolated {
                guard let url = item.previewItemURL, let place = places.byFile[url],
                    let square = Self.square(place.picture, side: place.frame.width)
                else { return nil }
                contentRect.pointee = CGRect(origin: .zero, size: square.size)
                return square
            }
        }

        /// `picture`'s middle square, as `.fill` shows it in a square, `side`
        /// points on a side: Quick Look draws the image at its own size from
        /// the frame's corner, whatever the frame's size.
        static func square(_ picture: UIImage, side: CGFloat) -> UIImage? {
            guard let cg = picture.cgImage, side > 0 else { return nil }
            let pixels = min(cg.width, cg.height)
            let crop = CGRect(x: (cg.width - pixels) / 2, y: (cg.height - pixels) / 2, width: pixels, height: pixels)
            guard let cut = cg.cropping(to: crop) else { return nil }
            return UIImage(cgImage: cut, scale: CGFloat(pixels) / side, orientation: picture.imageOrientation)
        }

        nonisolated func previewControllerDidDismiss(_ controller: QLPreviewController) {
            MainActor.assumeIsolated {
                if shown === controller || shown == nil {
                    shown = nil
                    closed()
                }
            }
        }
    }
}
