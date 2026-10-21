import Foundation
import UniformTypeIdentifiers

/// The sent messages' images kept on disk (ov-622), for the phone's and the
/// Mac's `PromptImageStore`: each image's original, which Quick Look opens at
/// full size, and its thumbnail, a few KB, beside it. Both or neither: a pair
/// that lost one file is a miss.
///
/// Everything here reads or writes files, so a store calls it off the main
/// thread. The files go in `PaneImageFiles` under a cap (the least recently
/// shown go first), and a pane's go with it.
public struct PromptImageDisk: Sendable {
    public let files: PaneImageFiles

    public init(files: PaneImageFiles) { self.files = files }

    /// Where the app keeps them: its caches directory, in a folder of its own
    /// bundle's (a stable, a preview and a canary on one Mac don't share one),
    /// 64 MB for a pane and 200 MB in all, nothing older than 60 days.
    public static let shared = PromptImageDisk(
        files: PaneImageFiles(
            root: PaneImageFiles.appFolder(.cachesDirectory, "prompt-images"),
            paneCap: 64 * 1024 * 1024, totalCap: 200 * 1024 * 1024, whenFull: .evictOldest, maxAge: 60 * 24 * 3600))

    /// Whether the images of turn `row` are kept at all. A resumed session's
    /// turns are numbered from 0 again (`turn:resumed:{seq}`), so the same id
    /// names a different turn in another session and a cached image would
    /// show under the wrong message.
    public static func cacheable(_ row: String) -> Bool { !row.hasPrefix("turn:resumed:") }

    /// The file names image `index` of `row` is kept under: the original, with
    /// its type's extension, and its thumbnail.
    public func names(row: String, index: Int, mime: String) -> (original: String, thumbnail: String) {
        let key = "\(row)#\(index)"
        let ext = UTType(mimeType: mime)?.preferredFilenameExtension ?? "png"
        let safe = String(key.map { $0.isLetter || $0.isNumber || $0 == "-" ? $0 : "-" })
        let tag = String(key.utf8.reduce(UInt32(2_166_136_261)) { ($0 ^ UInt32($1)) &* 16_777_619 }, radix: 16)
        return ("\(safe)-\(tag).\(ext)", "\(safe)-\(tag).thumb.png")
    }

    /// The names on disk for `pane`, to know what is kept without asking the
    /// disk for each image.
    public func kept(pane: String) -> Set<String> { Set(files.names(pane: pane)) }

    /// The original's file, and the thumbnail's bytes, where both are kept.
    public func held(pane: String, row: String, index: Int, mime: String) -> (original: URL, thumbnail: Data)? {
        guard Self.cacheable(row) else { return nil }
        let name = names(row: row, index: index, mime: mime)
        guard let original = files.url(named: name.original, pane: pane), let thumbnail = files.read(named: name.thumbnail, pane: pane) else {
            return nil
        }
        return (original, thumbnail)
    }

    /// Keep an image, and the file Quick Look opens; nil where it wasn't kept
    /// (not cacheable, over a cap, or the disk won't take it).
    public func keep(pane: String, row: String, index: Int, mime: String, original: Data, thumbnail: Data) -> URL? {
        guard Self.cacheable(row) else { return nil }
        let name = names(row: row, index: index, mime: mime)
        guard files.write(original, named: name.original, pane: pane), files.write(thumbnail, named: name.thumbnail, pane: pane) else {
            files.remove(named: name.original, pane: pane)
            files.remove(named: name.thumbnail, pane: pane)
            return nil
        }
        return files.url(named: name.original, pane: pane)
    }
}
