import AppKit
import Foundation
import UniformTypeIdentifiers

/// An image waiting in the native composer (ov-400): its bytes as the runner
/// takes them, PNG, JPEG, GIF or WebP, and a small picture for its chip.
///
/// The runner sniffs the bytes and refuses anything else, so an image in
/// another format (a TIFF from the pasteboard, a HEIC photo) is made a PNG
/// here, as it's added.
struct ComposeImage: Identifiable, Equatable, Sendable {
    let id = UUID()
    let mime: String
    let data: Data

    static func == (a: ComposeImage, b: ComposeImage) -> Bool { a.id == b.id }

    /// The formats the runner reads as they are, by type.
    private static let kept: [(UTType, String)] = [(.png, "image/png"), (.jpeg, "image/jpeg"), (.gif, "image/gif"), (.webP, "image/webp")]

    /// `data` of `type` as the runner takes it: as it is, or made a PNG.
    static func make(_ data: Data, type: UTType?) -> ComposeImage? {
        if let type, let mime = kept.first(where: { type.conforms(to: $0.0) })?.1 {
            return ComposeImage(mime: mime, data: data)
        }
        guard let rep = NSBitmapImageRep(data: data) ?? NSImage(data: data).flatMap(Self.bitmap),
            let png = rep.representation(using: .png, properties: [:])
        else { return nil }
        return ComposeImage(mime: "image/png", data: png)
    }

    private static func bitmap(_ image: NSImage) -> NSBitmapImageRep? {
        image.tiffRepresentation.flatMap(NSBitmapImageRep.init(data:))
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
        if pasteboard.availableType(from: [.string]) != nil { return [] }
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
        return pasteboard.availableType(from: [.string]) == nil && pasteboard.availableType(from: [.png, .tiff]) != nil
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
