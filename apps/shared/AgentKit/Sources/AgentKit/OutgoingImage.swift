import CoreGraphics
import Foundation
import ImageIO
import UniformTypeIdentifiers

/// An image waiting in a phone's conversation composer (ov-404): its bytes as
/// the runner takes them (PNG, JPEG, GIF or WebP) and what its chip draws.
///
/// The runner sniffs the bytes and refuses anything else, so an image in
/// another format (a HEIC photo, a TIFF from the pasteboard) is converted as
/// it's added: a JPEG at 0.9 when it's opaque, as a photo is, a PNG when it
/// has transparency; either at most `longestEdge` pixels on its long side. A
/// kept format past `largestKept` is converted the same way. The Mac's
/// `ComposeImage` follows the same rules, in the same words.
public struct OutgoingImage: Identifiable, Equatable, Sendable {
    public let id = UUID()
    public let mime: String
    public let data: Data

    public static func == (a: OutgoingImage, b: OutgoingImage) -> Bool { a.id == b.id }

    /// The formats the runner reads as they are, by type.
    private static let kept: [(UTType, String)] = [
        (.png, "image/png"), (.jpeg, "image/jpeg"), (.gif, "image/gif"), (.webP, "image/webp"),
    ]

    /// The long side, in pixels, of an image this converts: what claude reads
    /// an image at, and far more than a screenshot of a phone needs.
    public static let longestEdge = 2576

    /// The largest file the runner takes a piece of (`MAX_PASTE_FILE_BYTES`).
    public static let largestKept = 16 * 1024 * 1024

    /// `data` as the runner takes it: as it is when it's a format the runner
    /// reads and fits, converted when it isn't. Nil when it isn't an image.
    public static func make(_ data: Data) -> OutgoingImage? {
        guard let source = CGImageSourceCreateWithData(data as CFData, nil) else { return nil }
        let type = (CGImageSourceGetType(source) as String?).flatMap(UTType.init)
        if let type, data.count <= largestKept, let mime = kept.first(where: { type.conforms(to: $0.0) })?.1 {
            return OutgoingImage(mime: mime, data: data)
        }
        return converted(source, type: type)
    }

    /// `source` decoded, scaled down to `longestEdge`, and written as a JPEG
    /// at 0.9 when opaque or a PNG when not.
    private static func converted(_ source: CGImageSource, type decoded: UTType?) -> OutgoingImage? {
        guard let properties = CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any],
            let width = properties[kCGImagePropertyPixelWidth] as? Int,
            let height = properties[kCGImagePropertyPixelHeight] as? Int
        else { return nil }
        let options: [CFString: Any] = [
            kCGImageSourceCreateThumbnailFromImageAlways: true,
            kCGImageSourceThumbnailMaxPixelSize: min(max(width, height), longestEdge),
            kCGImageSourceCreateThumbnailWithTransform: true,
        ]
        guard let image = CGImageSourceCreateThumbnailAtIndex(source, 0, options as CFDictionary) else { return nil }
        // The file's own word first. A HEIC says nothing, and decodes with an
        // alpha channel it doesn't use: its format has no transparency a
        // camera writes, so it's a photo.
        let heif = [UTType.heic, .heif].contains { decoded?.conforms(to: $0) == true }
        let opaque = !(properties[kCGImagePropertyHasAlpha] as? Bool
            ?? (!heif && ![.none, .noneSkipFirst, .noneSkipLast].contains(image.alphaInfo)))
        let (type, mime) = opaque ? (UTType.jpeg, "image/jpeg") : (UTType.png, "image/png")
        let out = NSMutableData()
        guard let destination = CGImageDestinationCreateWithData(out, type.identifier as CFString, 1, nil) else { return nil }
        let quality: [CFString: Any] = [kCGImageDestinationLossyCompressionQuality: 0.9]
        CGImageDestinationAddImage(destination, image, (opaque ? quality : [:]) as CFDictionary)
        guard CGImageDestinationFinalize(destination) else { return nil }
        return OutgoingImage(mime: mime, data: out as Data)
    }

    /// The chip's picture: decoded small, once, not at full size on every
    /// redraw.
    public func thumbnail(side: Int = 96) -> CGImage? {
        guard let source = CGImageSourceCreateWithData(data as CFData, nil) else { return nil }
        let options: [CFString: Any] = [
            kCGImageSourceCreateThumbnailFromImageAlways: true, kCGImageSourceThumbnailMaxPixelSize: side,
            kCGImageSourceCreateThumbnailWithTransform: true,
        ]
        return CGImageSourceCreateThumbnailAtIndex(source, 0, options as CFDictionary)
    }
}
