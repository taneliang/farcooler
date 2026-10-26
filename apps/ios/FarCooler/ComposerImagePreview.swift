import ImageIO
import SwiftUI
import UIKit

/// A photo waiting in the composer, held (ov-792): the context menu's
/// preview, as Messages shows a photo in its field. Drawn from the bytes the
/// message will carry (`OutgoingImage.data`, after any conversion), never the
/// chip's thumbnail, so what's seen is what's sent.
///
/// Decoded once per photo, at the window's size, off the main thread when the
/// chip appears (`PreviewImages.prepare`), and taken from that cache here: a
/// 48 MP photo decoded in `body` held up the menu as it opened, and again on
/// every pass that re-evaluated it.
struct OutgoingImagePreview: View {
    let image: OutgoingImage

    /// The most the preview takes of the window, so the menu's Remove still
    /// fits under it.
    static let windowShare: CGFloat = 0.6

    /// The box the preview fits in: the window's width less a margin either
    /// side, and a share of its height. The window's, not the screen's: an
    /// iPad in Split View or Slide Over has a narrower one.
    @MainActor static var most: CGSize {
        let window = WindowBounds.current
        return CGSize(width: window.width - 2 * Spacing.section, height: window.height * windowShare)
    }

    var body: some View {
        let most = Self.most
        Group {
            if let drawn = PreviewImages.drawn(image, longest: PreviewImages.longest(for: most)) {
                let size = Self.fit(drawn.size, in: most)
                Image(uiImage: drawn)
                    .resizable()
                    .frame(width: size.width, height: size.height)
            } else {
                Image(systemName: "photo")
                    .font(.largeTitle)
                    .foregroundStyle(.quietInk)
                    .frame(width: Self.chipFallback, height: Self.chipFallback)
            }
        }
        .accessibilityElement()
        .accessibilityLabel("Photo Preview")
        // The chip's value: which photo this is (ov-805).
        .accessibilityValue(image.id.uuidString)
        .accessibilityIdentifier("native-image-preview")
    }

    private static let chipFallback: CGFloat = 200

    /// `size` scaled to fit `box`, its shape kept.
    static func fit(_ size: CGSize, in box: CGSize) -> CGSize {
        guard size.width > 0, size.height > 0 else { return box }
        let scale = min(box.width / size.width, box.height / size.height)
        return CGSize(width: (size.width * scale).rounded(), height: (size.height * scale).rounded())
    }
}

/// The held previews' decoded photos, by photo: each one's bytes never change
/// under its id, so a decode is good for as long as the photo waits.
enum PreviewImages {
    nonisolated(unsafe) private static let cache: NSCache<NSUUID, UIImage> = {
        let cache = NSCache<NSUUID, UIImage>()
        cache.countLimit = AgentConversation.mostImages
        return cache
    }()

    /// Pixels on the long side for a preview that fits `box`.
    @MainActor static func longest(for box: CGSize) -> CGFloat {
        max(box.width, box.height) * WindowBounds.scale
    }

    /// `image` decoded for a preview, from the cache, or decoded now and kept.
    static func drawn(_ image: OutgoingImage, longest: CGFloat) -> UIImage? {
        if let kept = cache.object(forKey: image.id as NSUUID) { return kept }
        guard let drawn = decode(image.data, longest: longest) else { return nil }
        cache.setObject(drawn, forKey: image.id as NSUUID)
        return drawn
    }

    /// Decode `image` off the main thread, ahead of a hold, so the menu opens
    /// on a picture already made.
    static func prepare(_ image: OutgoingImage, longest: CGFloat) async {
        if cache.object(forKey: image.id as NSUUID) != nil { return }
        await Task.detached(priority: .utility) { _ = drawn(image, longest: longest) }.value
    }

    /// `data` decoded no larger than `longest` pixels on its long side, the
    /// right way up.
    static func decode(_ data: Data, longest: CGFloat) -> UIImage? {
        guard let source = CGImageSourceCreateWithData(data as CFData, nil) else { return nil }
        let options: [CFString: Any] = [
            kCGImageSourceCreateThumbnailFromImageAlways: true,
            kCGImageSourceCreateThumbnailWithTransform: true,
            kCGImageSourceThumbnailMaxPixelSize: Int(longest),
        ]
        guard let cg = CGImageSourceCreateThumbnailAtIndex(source, 0, options as CFDictionary) else { return nil }
        return UIImage(cgImage: cg)
    }
}

/// The app's foreground window's size and scale: what a preview that isn't
/// laid out inside a container (a context menu's) sizes itself to. The
/// window's, not `UIScreen.main`'s, which on an iPad in Split View or Slide
/// Over is the whole screen.
@MainActor
enum WindowBounds {
    private static var window: UIWindow? {
        let scenes = UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }
        let scene = scenes.first { $0.activationState == .foregroundActive } ?? scenes.first
        return scene?.keyWindow ?? scene?.windows.first
    }

    /// An iPhone's, for a moment with no window yet.
    private static let fallback = CGSize(width: 390, height: 844)

    static var current: CGSize { window?.bounds.size ?? fallback }
    static var scale: CGFloat { window?.traitCollection.displayScale ?? 3 }
}
