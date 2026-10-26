import ImageIO
import SwiftUI
import UIKit

/// A photo waiting in the composer, held (ov-792): the context menu's
/// preview, as Messages shows a photo in its field. Drawn from the bytes the
/// message will carry (`OutgoingImage.data`, after any conversion), never the
/// chip's thumbnail, so what's seen is what's sent; read at the screen's size,
/// since a 48 MP photo decoded whole would hold up the menu as it opens.
struct OutgoingImagePreview: View {
    let image: OutgoingImage

    /// The most the preview takes of the screen, so the menu's Remove still
    /// fits under it.
    static let screenShare: CGFloat = 0.6

    var body: some View {
        let screen = UIScreen.main.bounds.size
        let most = CGSize(width: screen.width - 2 * Spacing.section, height: screen.height * Self.screenShare)
        Group {
            if let drawn = Self.decode(image.data, longest: max(most.width, most.height) * UIScreen.main.scale) {
                Image(uiImage: drawn)
                    .resizable()
                    .frame(width: Self.fit(drawn.size, in: most).width, height: Self.fit(drawn.size, in: most).height)
            } else {
                Image(systemName: "photo")
                    .font(.largeTitle)
                    .foregroundStyle(.quietInk)
                    .frame(width: Self.chipFallback, height: Self.chipFallback)
            }
        }
        .accessibilityElement()
        .accessibilityLabel("Photo")
        .accessibilityIdentifier("native-image-preview")
    }

    private static let chipFallback: CGFloat = 200

    /// `size` scaled to fit `box`, its shape kept.
    static func fit(_ size: CGSize, in box: CGSize) -> CGSize {
        guard size.width > 0, size.height > 0 else { return box }
        let scale = min(box.width / size.width, box.height / size.height)
        return CGSize(width: (size.width * scale).rounded(), height: (size.height * scale).rounded())
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
