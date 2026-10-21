import Foundation

/// The photos in a composer's unsent message, kept on disk per terminal so an
/// app quit or a jetsam doesn't lose them (ov-622). `NativeDraftStore` keeps
/// the words; the images can't live in that string.
///
/// A photo is written when it is attached and removed when it is taken out,
/// sent, or the pane goes away. Past 64 MB for a pane or 256 MB in all a photo
/// is just not kept (the composer still holds it until the app ends), and
/// whatever was kept stays. Anything untouched for 30 days goes, so a pane
/// that was closed while the app wasn't running doesn't leave its photos
/// behind for good.
///
/// Every change goes through one queue, in order, so a photo taken out right
/// after it went in can't have its write land afterwards.
public final class DraftImages: @unchecked Sendable {
    public static let shared = DraftImages(
        files: PaneImageFiles(
            root: PaneImageFiles.appFolder(.applicationSupportDirectory, "draft-images"),
            paneCap: 64 * 1024 * 1024, totalCap: 256 * 1024 * 1024, whenFull: .refuse, maxAge: 30 * 24 * 3600))

    private let files: PaneImageFiles
    private let queue = DispatchQueue(label: "draft-images", qos: .userInitiated)
    private let clock = NSLock()
    private var lastMillis = 0

    /// The time in milliseconds, never the same twice, so photos attached
    /// within one millisecond still list in the order they came.
    private func nextMillis(_ time: Date) -> Int {
        clock.withLock {
            lastMillis = max(Int(time.timeIntervalSince1970 * 1000), lastMillis + 1)
            return lastMillis
        }
    }

    public init(files: PaneImageFiles) { self.files = files }

    private static let types = [("image/png", "png"), ("image/jpeg", "jpg"), ("image/gif", "gif"), ("image/webp", "webp")]

    /// `<milliseconds>_<id>.<extension>`: files list in the order they were attached.
    private static func name(_ image: OutgoingImage, millis: Int) -> String {
        let ext = types.first { $0.0 == image.mime }?.1 ?? "png"
        return String(format: "%013d_", millis) + "\(image.id.uuidString).\(ext)"
    }

    /// Keep `image` for `terminal`'s draft.
    public func keep(_ image: OutgoingImage, terminal: String) {
        let name = Self.name(image, millis: nextMillis(Date()))
        queue.async { [files] in files.write(image.data, named: name, pane: terminal) }
    }

    /// `image` came out of the draft, or was sent.
    public func drop(_ id: UUID, terminal: String) {
        queue.async { [files] in
            for name in files.names(pane: terminal) where name.contains(id.uuidString) {
                files.remove(named: name, pane: terminal)
            }
        }
    }

    /// The draft is gone: sent whole, cleared, or its pane is.
    public func dropAll(terminal: String) {
        queue.async { [files] in files.removePane(terminal) }
    }

    /// Files past their age, and panes not in `keeping`.
    public func prune(keeping panes: Set<String>? = nil) {
        queue.async { [files] in files.prune(keeping: panes) }
    }

    /// What was kept for `terminal`, in the order it was attached.
    public func restored(terminal: String) async -> [OutgoingImage] {
        await withCheckedContinuation { continuation in
            queue.async { [files] in
                let found = files.names(pane: terminal).sorted().compactMap { name -> OutgoingImage? in
                    let stem = name.split(separator: ".").first.map(String.init) ?? ""
                    let parts = stem.split(separator: "_", maxSplits: 1)
                    guard parts.count == 2, let id = UUID(uuidString: String(parts[1])),
                        let ext = name.split(separator: ".").last.map(String.init),
                        let mime = Self.types.first(where: { $0.1 == ext })?.0,
                        let data = files.read(named: name, pane: terminal)
                    else { return nil }
                    return OutgoingImage(id: id, mime: mime, data: data)
                }
                continuation.resume(returning: found)
            }
        }
    }

    /// Wait for the changes so far. For tests.
    public func flush() { queue.sync {} }
}
