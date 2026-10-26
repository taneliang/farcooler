import ImageIO
import QuickLook
import SwiftUI
import UIKit
import UniformTypeIdentifiers

// A prompt's images in the conversation view (ov-458), as the Mac's
// `PromptImages.swift` draws them: thumbnails above the message that open at
// full size, the message without the `[Image #N]` placeholders they stand for.

/// Where a prompt's images come from: the runner, which reads each back from
/// the agent's transcript (`agent.image`), so an image pasted in the terminal
/// shows as one sent from the composer does.
protocol PromptImageSource: Sendable {
    /// Image `index` of the prompt on turn row `row`, its bytes whole.
    func promptImage(terminal: String, row: String, index: Int) async throws -> Data
}

/// The runner's `agent.image`, over this phone's client core.
struct CorePromptImages: PromptImageSource {
    let core: ClientCore

    func promptImage(terminal: String, row: String, index: Int) async throws -> Data {
        let data = try await core.call("agent.image", ["terminal": terminal, "row": row, "index": index])
        let object = (try? JSONSerialization.jsonObject(with: data) as? [String: Any]) ?? [:]
        guard let text = object["base64"] as? String, let bytes = Data(base64Encoded: text) else {
            throw CocoaError(.fileReadCorruptFile)
        }
        return bytes
    }
}

/// One pane's prompt images: each fetched once, kept as a small picture for
/// its thumbnail and as a file for Quick Look to open at full size.
///
/// Both are also kept on disk, in the app's caches directory (ov-622), so a
/// sent message's photos are there after a restart and offline, not drawn as
/// placeholders until the runner is asked again (`PromptImageDisk`: capped,
/// the least recently shown go first, a pane's go with it). All of that disk
/// work runs off the main thread; what the views ask in a redraw is answered
/// from `keptFiles`, the names on disk, read once as the store is made.
@MainActor
final class PromptImageStore: ObservableObject {
    let terminal: String
    /// The runner, where it serves `agent.image`; nil, and nothing is
    /// fetched where it doesn't (what is on disk still shows).
    @Published var source: (any PromptImageSource)?

    enum State: Equatable {
        case loading
        case shown(UIImage, URL)
        case failed
    }

    @Published private(set) var states: [String: State] = [:]
    private var asked: Set<String> = []
    /// The names of this pane's files on disk.
    @Published private var keptFiles: Set<String> = []
    private let disk: PromptImageDisk

    nonisolated static let disk = PromptImageDisk.shared

    init(terminal: String, source: (any PromptImageSource)?, disk: PromptImageDisk = PromptImageStore.disk) {
        self.terminal = terminal
        self.source = source
        self.disk = disk
        Task { [terminal] in
            let names = await Task.detached(priority: .userInitiated) { disk.kept(pane: terminal) }.value
            keptFiles.formUnion(names)
        }
    }

    static func key(row: String, index: Int) -> String { "\(row)#\(index)" }

    func state(row: String, index: Int) -> State {
        states[Self.key(row: row, index: index)] ?? .loading
    }

    /// Whether image `index` of `row` is on disk, so it can be drawn with no
    /// runner to ask. Answered from memory.
    func isKept(row: String, index: Int, mime: String) -> Bool {
        guard PromptImageDisk.cacheable(row) else { return false }
        let name = disk.names(row: row, index: index, mime: mime)
        return keptFiles.contains(name.original) && keptFiles.contains(name.thumbnail)
    }

    /// Show image `index` of `row`: from disk where it is kept, else fetched
    /// once; again only after it failed and the row is shown anew.
    func load(row: String, index: Int, mime: String) async {
        let key = Self.key(row: row, index: index)
        guard !asked.contains(key) else { return }
        let (disk, terminal) = (self.disk, self.terminal)
        if isKept(row: row, index: index, mime: mime) {
            let held = await Task.detached(priority: .userInitiated) {
                disk.held(pane: terminal, row: row, index: index, mime: mime)
            }.value
            if let held, let picture = UIImage(data: held.thumbnail) {
                asked.insert(key)
                states[key] = .shown(picture, held.original)
                return
            }
        }
        guard let source else { return }
        asked.insert(key)
        states[key] = .loading
        do {
            let data = try await source.promptImage(terminal: terminal, row: row, index: index)
            guard let thumbnail = Self.thumbnail(data) else { throw CocoaError(.fileReadCorruptFile) }
            let file = try await keep(data, thumbnail: thumbnail, row: row, index: index, mime: mime)
            states[key] = .shown(thumbnail, file)
        } catch {
            states[key] = .failed
            asked.remove(key)
        }
    }

    /// The original on disk for Quick Look, and the thumbnail beside it. Where
    /// the cache won't take them (a resumed session's turn, over a cap, or the
    /// disk is full) the original is a temporary file for as long as it is open.
    private func keep(_ data: Data, thumbnail: UIImage, row: String, index: Int, mime: String) async throws -> URL {
        let (disk, terminal) = (self.disk, self.terminal)
        let small = thumbnail.pngData()
        let name = disk.names(row: row, index: index, mime: mime)
        let kept = await Task.detached(priority: .utility) { () -> URL? in
            guard let small else { return nil }
            return disk.keep(pane: terminal, row: row, index: index, mime: mime, original: data, thumbnail: small)
        }.value
        if let kept {
            keptFiles.insert(name.original)
            keptFiles.insert(name.thumbnail)
            return kept
        }
        let folder = FileManager.default.temporaryDirectory.appendingPathComponent("FarCooler Prompt Images", isDirectory: true)
            .appendingPathComponent(terminal, isDirectory: true)
        let url = folder.appendingPathComponent(name.original)
        try await Task.detached(priority: .utility) {
            try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
            try data.write(to: url, options: [.atomic, .completeFileProtection])
        }.value
        return url
    }

    /// The thumbnail's picture: drawn small once, never decoded at full size
    /// on a redraw.
    static func thumbnail(_ data: Data, side: CGFloat = 240) -> UIImage? {
        guard let source = CGImageSourceCreateWithData(data as CFData, nil) else { return nil }
        let options: [CFString: Any] = [
            kCGImageSourceCreateThumbnailFromImageAlways: true, kCGImageSourceThumbnailMaxPixelSize: side,
            kCGImageSourceCreateThumbnailWithTransform: true,
        ]
        guard let cg = CGImageSourceCreateThumbnailAtIndex(source, 0, options as CFDictionary) else { return nil }
        return UIImage(cgImage: cg)
    }
}

extension EnvironmentValues {
    /// The pane's prompt images, where its runner serves them.
    @Entry var promptImages: PromptImageStore?
}

/// A turn's images above its message, as thumbnails that open at full size in
/// Quick Look, grown out of the thumbnail and swiped down to put away
/// (`PromptImageViewer`). Nothing where the prompt had none, or the runner
/// doesn't serve them.
struct PromptImageStrip: View {
    let row: String
    let images: [AgentRow.PromptImage]
    @Environment(\.promptImages) private var store

    /// The side of a thumbnail, in points.
    static let side: CGFloat = 80

    var body: some View {
        if let store, !images.isEmpty {
            Strip(row: row, images: images, store: store)
        }
    }

    private struct Strip: View {
        let row: String
        let images: [AgentRow.PromptImage]
        @ObservedObject var store: PromptImageStore
        @State private var preview: URL?
        /// Where each thumbnail is, for the viewer to grow out of.
        @State private var places = PromptImageViewer.Places()

        var body: some View {
            if store.source != nil || images.indices.contains(where: { store.isKept(row: row, index: $0, mime: images[$0].mime) }) {
                strip
            }
        }

        private var strip: some View {
            let files = images.indices.compactMap { i -> URL? in
                if case .shown(_, let url) = store.state(row: row, index: i) { return url }
                return nil
            }
            return TrailingFlow(spacing: Spacing.group) {
                ForEach(images.indices, id: \.self) { i in
                    thumbnail(i)
                        .task(id: PromptImageStore.key(row: row, index: i)) {
                            await store.load(row: row, index: i, mime: images[i].mime)
                        }
                }
            }
            .frame(maxWidth: .infinity, alignment: .trailing)
            .background { PromptImageViewer(preview: $preview, files: files, places: places) }
            .accessibilityElement(children: .contain)
            .accessibilityIdentifier("native-prompt-images")
        }

        @ViewBuilder
        private func thumbnail(_ i: Int) -> some View {
            let label = images.count == 1 ? "Image" : "Image \(i + 1) of \(images.count)"
            switch store.state(row: row, index: i) {
            case .shown(let picture, let url):
                Button {
                    preview = url
                } label: {
                    Image(uiImage: picture)
                        .resizable()
                        .aspectRatio(contentMode: .fill)
                        .frame(width: PromptImageStrip.side, height: PromptImageStrip.side)
                        .clipShape(.control)
                        .contentShape(.control)
                }
                .buttonStyle(.plain)
                // Kept as it moves, without a redraw: the viewer reads it only
                // as it opens and closes.
                .onGeometryChange(for: CGRect.self) { $0.frame(in: .global) } action: { frame in
                    places.byFile[url] = .init(frame: frame, picture: picture)
                }
                .accessibilityLabel(label)
                .accessibilityHint("Opens the image at full size.")
                .accessibilityIdentifier("native-prompt-image")
            case .loading:
                placeholder { ProgressView() }
                    .accessibilityLabel("\(label), loading")
            case .failed:
                placeholder { Image(systemName: "photo").foregroundStyle(.quietInk) }
                    .accessibilityLabel("\(label), couldn’t be loaded")
            }
        }

        private func placeholder(_ content: () -> some View) -> some View {
            content()
                .frame(width: PromptImageStrip.side, height: PromptImageStrip.side)
                .surface(.inset, in: .control)
                .clipShape(.control)
        }
    }
}

/// Thumbnails right to left as a message's attachments sit, wrapping onto
/// another row where the screen is too narrow for them all, each row flush
/// with the message's trailing edge.
struct TrailingFlow: Layout {
    var spacing: CGFloat

    private func rows(_ subviews: Subviews, width: CGFloat) -> [(items: [Int], width: CGFloat, height: CGFloat)] {
        var rows: [(items: [Int], width: CGFloat, height: CGFloat)] = []
        for (i, view) in subviews.enumerated() {
            let size = view.sizeThatFits(.unspecified)
            if let last = rows.last, !last.items.isEmpty, last.width + spacing + size.width <= width {
                rows[rows.count - 1] = (last.items + [i], last.width + spacing + size.width, max(last.height, size.height))
            } else {
                rows.append(([i], size.width, size.height))
            }
        }
        return rows
    }

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        let width = proposal.width ?? .infinity
        let rows = rows(subviews, width: width)
        let height = rows.map(\.height).reduce(0, +) + spacing * CGFloat(max(0, rows.count - 1))
        let widest = rows.map(\.width).max() ?? 0
        return CGSize(width: proposal.width ?? widest, height: height)
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        var y = bounds.minY
        for row in rows(subviews, width: bounds.width) {
            var x = bounds.maxX - row.width
            for i in row.items {
                let size = subviews[i].sizeThatFits(.unspecified)
                subviews[i].place(at: CGPoint(x: x, y: y), proposal: ProposedViewSize(size))
                x += size.width + spacing
            }
            y += row.height + spacing
        }
    }
}

extension AgentConversation {
    /// `prompt` without its first `shown` `[Image #N]` placeholders, and the
    /// space before each; the rest, and every other word, as they are.
    static func withoutPlaceholders(_ prompt: String, shown: Int) -> String {
        guard shown > 0, let pattern = try? NSRegularExpression(pattern: #"[ \t]?\[Image #\d+\]"#) else { return prompt }
        let text = prompt as NSString
        let matches = pattern.matches(in: prompt, range: NSRange(location: 0, length: text.length)).prefix(shown)
        var out = prompt
        for match in matches.reversed() {
            guard let range = Range(match.range, in: out) else { continue }
            out.removeSubrange(range)
        }
        return out.trimmingCharacters(in: .whitespacesAndNewlines)
    }
}

/// The panes one runner's fleet has had, to see which have gone (ov-622): a
/// pane that is really gone (`MissingPanes`: missing from two reads at least
/// 30 s apart) takes its draft's photos and its sent messages' thumbnails
/// with it. Only that runner's own panes, since a phone can have several
/// runners. A pane that went while the app was closed, a runner that was
/// removed and a sign-out leave their photos for the age limit to clear.
struct GonePanes {
    private var missing = MissingPanes()

    @MainActor
    mutating func forget(missingFrom now: Set<String>) {
        let gone = missing.read(now)
        guard !gone.isEmpty else { return }
        let cache = PromptImageStore.disk.files
        for terminal in gone { DraftImages.shared.dropAll(terminal: terminal) }
        Task.detached(priority: .utility) { for terminal in gone { cache.removePane(terminal) } }
    }
}
