import AgentKit
import AppKit
import ImageIO
import QuickLook
import SwiftUI
import UniformTypeIdentifiers

/// Where a prompt's images come from (ov-454): the runner, which reads each
/// back from the agent's transcript (`agent.image`), so an image pasted in
/// the terminal shows as one sent from the composer does.
protocol PromptImageSource: Sendable {
    /// Image `index` of the prompt on turn row `row`, its bytes whole.
    func promptImage(terminal: String, row: String, index: Int) async throws -> Data
}

extension RunnerCore: PromptImageSource {
    func promptImage(terminal: String, row: String, index: Int) async throws -> Data {
        let data = try await call("agent.image", ["terminal": terminal, "row": row, "index": index])
        let object = (try? JSONSerialization.jsonObject(with: data) as? [String: Any]) ?? [:]
        guard let text = object["base64"] as? String, let bytes = Data(base64Encoded: text) else {
            throw Failure.refused("The image couldn’t be read.", word: nil, what: "image")
        }
        return bytes
    }
}

/// One pane's prompt images: each fetched once, kept as a small picture for
/// its thumbnail and as a file for Quick Look to open at full size.
@MainActor
final class PromptImageStore: ObservableObject {
    let terminal: String
    /// The runner, where it serves `agent.image`; nil, and no thumbnails
    /// are drawn, where it doesn't.
    @Published var source: (any PromptImageSource)?

    /// What a thumbnail shows.
    enum State: Equatable {
        case loading
        case shown(NSImage, URL)
        case failed
    }

    @Published private(set) var states: [String: State] = [:]
    private var asked: Set<String> = []

    /// Where this pane's images are written for Quick Look: a folder of the
    /// app's own under the temporary directory, one per pane.
    let folder: URL

    init(terminal: String, source: (any PromptImageSource)?, folder: URL? = nil) {
        self.terminal = terminal
        self.source = source
        self.folder = folder
            ?? FileManager.default.temporaryDirectory.appendingPathComponent("FarCooler Prompt Images", isDirectory: true)
            .appendingPathComponent(terminal, isDirectory: true)
    }

    static func key(row: String, index: Int) -> String { "\(row)#\(index)" }

    func state(row: String, index: Int) -> State {
        states[Self.key(row: row, index: index)] ?? .loading
    }

    /// Fetch image `index` of `row` once; again only after it failed and the
    /// row is shown anew.
    func load(row: String, index: Int, mime: String) async {
        let key = Self.key(row: row, index: index)
        guard let source, !asked.contains(key) else { return }
        asked.insert(key)
        states[key] = .loading
        do {
            let data = try await source.promptImage(terminal: terminal, row: row, index: index)
            let file = try write(data, key: key, mime: mime)
            guard let thumbnail = Self.thumbnail(data) else { throw CocoaError(.fileReadCorruptFile) }
            states[key] = .shown(thumbnail, file)
        } catch {
            states[key] = .failed
            asked.remove(key)
        }
    }

    /// The bytes as a file Quick Look can open, named for the image.
    private func write(_ data: Data, key: String, mime: String) throws -> URL {
        try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        let ext = UTType(mimeType: mime)?.preferredFilenameExtension ?? "png"
        let safe = key.map { $0.isLetter || $0.isNumber || $0 == "-" ? $0 : "-" }
        let url = folder.appendingPathComponent(String(safe)).appendingPathExtension(ext)
        try data.write(to: url, options: .atomic)
        return url
    }

    /// The thumbnail's picture: drawn small once, never decoded at full size
    /// on a redraw.
    static func thumbnail(_ data: Data, side: CGFloat = 160) -> NSImage? {
        guard let source = CGImageSourceCreateWithData(data as CFData, nil) else { return nil }
        let options: [CFString: Any] = [
            kCGImageSourceCreateThumbnailFromImageAlways: true, kCGImageSourceThumbnailMaxPixelSize: side,
            kCGImageSourceCreateThumbnailWithTransform: true,
        ]
        guard let cg = CGImageSourceCreateThumbnailAtIndex(source, 0, options as CFDictionary) else { return nil }
        return NSImage(cgImage: cg, size: NSSize(width: cg.width, height: cg.height))
    }
}

extension EnvironmentValues {
    /// The pane's prompt images, where its runner serves them.
    @Entry var promptImages: PromptImageStore?
}

/// A turn's images above its message, as thumbnails that open at full size
/// in Quick Look (ov-454). Nothing where the prompt had none, or the runner
/// doesn't serve them.
struct PromptImageStrip: View {
    let row: String
    let images: [AgentRow.PromptImage]
    @Environment(\.promptImages) private var store

    /// The side of a thumbnail, in points.
    static let side: CGFloat = 72

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

        var body: some View {
            if store.source != nil { strip }
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
            .quickLookPreview($preview, in: files)
            .identified("native-prompt-images")
        }

        @ViewBuilder
        private func thumbnail(_ i: Int) -> some View {
            let label = images.count == 1 ? "Image" : "Image \(i + 1) of \(images.count)"
            switch store.state(row: row, index: i) {
            case .shown(let picture, let url):
                Button {
                    preview = url
                } label: {
                    Image(nsImage: picture)
                        .resizable()
                        .aspectRatio(contentMode: .fill)
                        .frame(width: PromptImageStrip.side, height: PromptImageStrip.side)
                        .clipShape(.control)
                        .contentShape(.control)
                }
                .buttonStyle(.plain)
                .help("Open Image")
                .accessibilityLabel(label)
                .accessibilityHint("Opens the image at full size.")
                .identified("native-prompt-image")
            case .loading:
                placeholder { ProgressView().controlSize(.small) }
                    .accessibilityLabel("\(label), loading")
            case .failed:
                placeholder { Image(systemName: "photo").foregroundStyle(.secondary) }
                    .help("This image couldn’t be loaded.")
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
/// another row where the pane is too narrow for them all, each row flush
/// with the message's trailing edge (ov-454 review 3).
struct TrailingFlow: Layout {
    var spacing: CGFloat

    /// Each row's subview indices and its width and height, for `width`.
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
