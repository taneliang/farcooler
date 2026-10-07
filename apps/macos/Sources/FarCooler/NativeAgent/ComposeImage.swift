import AppKit
import Foundation
import UniformTypeIdentifiers

/// An image waiting in the native composer (ov-400): its bytes as the runner
/// takes them, PNG, JPEG, GIF or WebP, and a small picture for its chip.
///
/// The runner sniffs the bytes and refuses anything else, so an image in
/// another format (a TIFF from the pasteboard, a HEIC photo) is converted
/// here, as it's added: a JPEG when it's opaque, as a photo is, a PNG when
/// it has transparency; either at most `longestEdge` pixels on its long
/// side. A kept format past a paste's 16 MB is converted the same way.
struct ComposeImage: Identifiable, Equatable, Sendable {
    let id = UUID()
    let mime: String
    let data: Data

    static func == (a: ComposeImage, b: ComposeImage) -> Bool { a.id == b.id }

    /// The formats the runner reads as they are, by type.
    private static let kept: [(UTType, String)] = [(.png, "image/png"), (.jpeg, "image/jpeg"), (.gif, "image/gif"), (.webP, "image/webp")]

    /// The long side, in pixels, of an image this converts: what claude
    /// reads an image at, and far more than a screenshot of a window needs.
    static let longestEdge = 2576

    /// The largest file a paste takes (`MAX_PASTE_FILE_BYTES`).
    static let largestKept = 16 * 1024 * 1024

    /// `data` of `type` as the runner takes it: as it is, or converted.
    static func make(_ data: Data, type: UTType?) -> ComposeImage? {
        if let type, data.count <= largestKept, let mime = kept.first(where: { type.conforms(to: $0.0) })?.1 {
            return ComposeImage(mime: mime, data: data)
        }
        return converted(data)
    }

    /// `data` decoded, scaled down to `longestEdge`, and written as a JPEG
    /// at 0.9 when opaque or a PNG when not.
    private static func converted(_ data: Data) -> ComposeImage? {
        guard let source = CGImageSourceCreateWithData(data as CFData, nil),
            let properties = CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any],
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
        let decoded = (CGImageSourceGetType(source) as String?).flatMap(UTType.init)
        let heif = [UTType.heic, .heif].contains { decoded?.conforms(to: $0) == true }
        let opaque = !(properties[kCGImagePropertyHasAlpha] as? Bool
            ?? (!heif && ![.none, .noneSkipFirst, .noneSkipLast].contains(image.alphaInfo)))
        let (type, mime) = opaque ? (UTType.jpeg, "image/jpeg") : (UTType.png, "image/png")
        let out = NSMutableData()
        guard let destination = CGImageDestinationCreateWithData(out, type.identifier as CFString, 1, nil) else { return nil }
        let quality: [CFString: Any] = [kCGImageDestinationLossyCompressionQuality: 0.9]
        CGImageDestinationAddImage(destination, image, (opaque ? quality : [:]) as CFDictionary)
        guard CGImageDestinationFinalize(destination) else { return nil }
        return ComposeImage(mime: mime, data: out as Data)
    }

    /// The image files at `urls`, each read and kept or converted; anything
    /// that isn't an image is left out.
    static func from(urls: [URL]) -> [ComposeImage] {
        urls.compactMap { url in
            let type = UTType(filenameExtension: url.pathExtension)
            guard type?.conforms(to: .image) == true, let data = try? Data(contentsOf: url) else { return nil }
            return make(data, type: type)
        }
    }

    /// The images on `pasteboard`, or none when it holds text to paste
    /// instead: image files first (a Finder copy names them and their names
    /// as text); then any text, so a cell copied with a picture of itself
    /// pastes as words; then image data (a screenshot copied to the
    /// clipboard).
    static func from(_ pasteboard: NSPasteboard) -> [ComposeImage] {
        let options: [NSPasteboard.ReadingOptionKey: Any] = [
            .urlReadingFileURLsOnly: true, .urlReadingContentsConformToTypes: [UTType.image.identifier],
        ]
        if let urls = pasteboard.readObjects(forClasses: [NSURL.self], options: options) as? [URL], !urls.isEmpty {
            return from(urls: urls)
        }
        if holdsWords(pasteboard) { return [] }
        for (type, uti) in [(NSPasteboard.PasteboardType.png, UTType.png), (.tiff, .tiff)] {
            if let data = pasteboard.data(forType: type), let image = make(data, type: uti) { return [image] }
        }
        return []
    }

    /// Whether `pasteboard` holds images `from` would read, without reading
    /// them: for a drag passing over, and for enabling Paste.
    static func offered(on pasteboard: NSPasteboard) -> Bool {
        let options: [NSPasteboard.ReadingOptionKey: Any] = [
            .urlReadingFileURLsOnly: true, .urlReadingContentsConformToTypes: [UTType.image.identifier],
        ]
        if pasteboard.canReadObject(forClasses: [NSURL.self], options: options) { return true }
        return !holdsWords(pasteboard) && pasteboard.availableType(from: [.png, .tiff]) != nil
    }

    /// Whether `pasteboard` holds text to paste rather than its picture:
    /// any text but a lone web address, which a browser's Copy Image may
    /// put beside the image it copied.
    static func holdsWords(_ pasteboard: NSPasteboard) -> Bool {
        guard let string = pasteboard.string(forType: .string) else { return false }
        let trimmed = string.trimmingCharacters(in: .whitespacesAndNewlines)
        let lone = !trimmed.contains(where: \.isWhitespace) && ["http", "https"].contains(URL(string: trimmed)?.scheme ?? "")
        return !(lone && pasteboard.availableType(from: [.png, .tiff]) != nil)
    }

    /// The chip's picture: drawn small once, not decoded at full size on
    /// every redraw.
    func thumbnail(side: CGFloat = 96) -> NSImage? {
        guard let source = CGImageSourceCreateWithData(data as CFData, nil) else { return nil }
        let options: [CFString: Any] = [
            kCGImageSourceCreateThumbnailFromImageAlways: true, kCGImageSourceThumbnailMaxPixelSize: side,
            kCGImageSourceCreateThumbnailWithTransform: true,
        ]
        guard let cg = CGImageSourceCreateThumbnailAtIndex(source, 0, options as CFDictionary) else { return nil }
        return NSImage(cgImage: cg, size: NSSize(width: cg.width, height: cg.height))
    }
}
