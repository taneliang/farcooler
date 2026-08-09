import SwiftUI
import UIKit
import UniformTypeIdentifiers

/// Pasting an image into a terminal.
///
/// A terminal takes bytes and an agent running in one opens a path, so an image
/// from a phone has to become a file on the machine the pane is on before the
/// agent can see it. The daemon writes it and types the path; this carries the
/// bytes there and says how far along it is.
///
/// Always a transfer here, unlike the Mac: a phone is never the machine the
/// pane is on.

/// One image on its way into a pane.
@MainActor
final class ImagePasteJob: ObservableObject, Identifiable {
    let id = UUID()
    let thumbnail: UIImage?
    @Published var sent: Int64 = 0
    @Published var total: Int64
    @Published var failure: String?

    var retry: (() -> Void)?

    init(thumbnail: UIImage?, total: Int64) {
        self.thumbnail = thumbnail
        self.total = total
    }

    var fraction: Double {
        guard total > 0 else { return 0 }
        return min(1, Double(sent) / Double(total))
    }
}

@MainActor
final class ImagePasteQueue: ObservableObject {
    @Published private(set) var jobs: [ImagePasteJob] = []

    /// The largest image the daemon will accept. Checked here too, so a phone
    /// on a slow link is told immediately rather than after the upload.
    static let limit: Int64 = 16 * 1024 * 1024

    func send(_ image: UIImage, name: String = "", terminal: String, core: ClientCore) {
        guard let (data, mime) = encode(image) else {
            let job = ImagePasteJob(thumbnail: image, total: 0)
            job.failure = "Far Cooler couldn't prepare that image to send."
            jobs.append(job)
            return
        }
        // Named from the encoding when the picker gave us nothing: a phone's
        // photo library has no filename worth carrying, and `photo.png` in a
        // prompt still reads better than a bare timestamp.
        let named = name.isEmpty ? (mime == "image/jpeg" ? "photo.jpg" : "photo.png") : name
        send(data: data, name: named, mime: mime, thumbnail: image, terminal: terminal, core: core)
    }

    func send(
        data: Data, name: String, mime: String, thumbnail: UIImage?, terminal: String,
        core: ClientCore
    ) {
        let job = ImagePasteJob(thumbnail: thumbnail, total: Int64(data.count))
        guard Int64(data.count) <= Self.limit else {
            job.failure = "That file is too large to send. Files up to 16 MB work."
            jobs.append(job)
            return
        }

        let run = { [weak self, weak job] in
            guard let self, let job else { return }
            job.failure = nil
            job.sent = 0
            Task {
                do {
                    _ = try await core.pasteFile(
                        terminal, name: name, mime: mime, data: data,
                        onProgress: { sent, total in
                            Task { @MainActor in
                                job.sent = sent
                                if total > 0 { job.total = total }
                            }
                        })
                    self.dismiss(job)
                } catch {
                    job.failure = Self.message(for: error)
                }
            }
        }
        job.retry = run
        jobs.append(job)
        run()
    }

    func dismiss(_ job: ImagePasteJob) {
        jobs.removeAll { $0.id == job.id }
    }

    /// Turn the core's answer into something worth showing someone.
    ///
    /// The core's own text is developer-facing, and none of it belongs on a
    /// phone screen over a terminal.
    static func message(for error: Error) -> String {
        let text = error.localizedDescription.lowercased()
        // Two causes, indistinguishable on the wire: a daemon too old
        // to know the method, or a pane that closed before the path
        // could be typed. Named together rather than guessed.
        if text.contains("predates this") {
            return "That terminal may have closed, or this machine's Far Cooler may be too old."
        }
        if text.contains("file size") {
            return "That file is too large to send. Files up to 16 MB work."
        }
        if text.contains("not found") {
            return "That terminal isn't running anymore."
        }
        return "Couldn't reach this machine."
    }

    /// PNG for anything with sharp edges, JPEG for a photograph.
    ///
    /// HEIC is never sent: it is what an iPhone photo actually is, and both
    /// Claude Code and Codex refuse it — an untouched photo would land as a
    /// file that exists, has a path, and cannot be read. Screenshots stay PNG
    /// because their whole point is usually small text, which JPEG smears.
    private func encode(_ image: UIImage) -> (Data, String)? {
        let pixels = image.size.width * image.size.height * image.scale * image.scale
        // A screenshot is small enough that lossless costs little; a camera
        // photo as PNG is tens of megabytes and would fail the size check for
        // no gain in what an agent can read from it.
        if pixels <= 4_000_000, let png = image.pngData() {
            return (png, "image/png")
        }
        if let jpeg = image.jpegData(compressionQuality: 0.9) {
            return (jpeg, "image/jpeg")
        }
        return image.pngData().map { ($0, "image/png") }
    }
}

/// The chips for one pane, stacked over the bottom of it.
struct ImagePasteChips: View {
    @ObservedObject var queue: ImagePasteQueue

    var body: some View {
        VStack(spacing: 6) {
            ForEach(queue.jobs) { job in
                ImagePasteChip(job: job) { queue.dismiss(job) }
            }
        }
        .padding(.bottom, 12)
        .animation(.easeOut(duration: 0.15), value: queue.jobs.count)
    }
}

private struct ImagePasteChip: View {
    @ObservedObject var job: ImagePasteJob
    let onDismiss: () -> Void

    var body: some View {
        HStack(spacing: 8) {
            if let thumbnail = job.thumbnail {
                Image(uiImage: thumbnail)
                    .resizable()
                    .aspectRatio(contentMode: .fill)
                    .frame(width: 26, height: 26)
                    .clipShape(RoundedRectangle(cornerRadius: 5))
            }

            if let failure = job.failure {
                Text(failure).font(.footnote).lineLimit(2)
                Button("Retry") { job.retry?() }.font(.footnote.weight(.semibold))
                Button("Cancel", action: onDismiss).font(.footnote)
            } else {
                Text("Sending\u{2026}").font(.footnote)
                ProgressView(value: job.fraction)
                    .progressViewStyle(.circular)
                    .controlSize(.small)
            }
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 8)
        .modifier(GlassSurface(radius: 16))
        .padding(.horizontal, 12)
    }
}
