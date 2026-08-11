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

/// Making a picture small enough to ride WITH a prompt.
///
/// Two paths carry an image to a host and they have opposite constraints.
/// `terminal.paste_file` streams in chunks — that is what `crates/client` says
/// it exists for — so a phone photo goes over it whole, at any size. A prompt's
/// image is a content block inside ONE control envelope, and the protocol caps
/// an envelope at `MAX_CONTROL_ENVELOPE_BYTES` (1 MiB, in
/// crates/protocol/src/lib.rs). `framing::encode` refuses to encode a larger
/// one, so an untouched 4032x3024 photo — three to eight megabytes — could
/// never be sent as a prompt attachment and failed every time, reported as
/// though the machine were unreachable.
///
/// Shrinking rather than raising the cap: the cap is a deliberate guard on the
/// control channel, and full resolution buys nothing here anyway. Models
/// downscale on receipt, so the pixels past their limit are spent on the wire
/// and then thrown away.
enum PromptImageBudget {
    /// Comfortably inside the 1 MiB envelope with the prompt, the terminal id
    /// and protobuf framing alongside it. Deliberately not 1 MiB minus a few
    /// bytes: several images can ride one prompt.
    static let maxBytes = 500 * 1024

    /// The long edge to fit inside. Past this a model downsamples anyway, so
    /// the extra pixels cost upload time and buy no accuracy.
    static let maxDimension: CGFloat = 1568

    /// The bytes to actually send, and the type they are.
    ///
    /// An image already inside the budget is sent UNTOUCHED — a screenshot is
    /// usually small, usually PNG, and usually full of small text that a
    /// re-encode would smear. Only what cannot fit is resized, and then as
    /// JPEG, because a photograph is what "too big" almost always means.
    static func fit(_ image: UIImage, original: Data, mime: String) -> (Data, String)? {
        if original.count <= maxBytes { return (original, mime) }

        var candidate = image
        let longEdge = max(image.size.width, image.size.height)
        if longEdge > maxDimension {
            candidate = resize(image, scale: maxDimension / longEdge)
        }

        // Step the quality down rather than guessing one that works: the same
        // pixel count compresses to wildly different sizes depending on what is
        // in the picture, so a fixed quality either wastes budget on a flat
        // screenshot or overshoots on a detailed photograph.
        for quality in [0.8, 0.6, 0.45, 0.3] as [CGFloat] {
            if let data = candidate.jpegData(compressionQuality: quality),
                data.count <= maxBytes
            {
                return (data, "image/jpeg")
            }
        }

        // Still too big at the lowest quality worth sending: halve the edge once
        // more and take whatever that gives. A picture this stubborn is
        // enormous, and something legible beats nothing.
        let smaller = resize(candidate, scale: 0.5)
        return smaller.jpegData(compressionQuality: 0.5).map { ($0, "image/jpeg") }
    }

    private static func resize(_ image: UIImage, scale: CGFloat) -> UIImage {
        let size = CGSize(width: image.size.width * scale, height: image.size.height * scale)
        // Opaque, and at scale 1: a UIImage carries the screen's scale factor,
        // so rendering at the device's 3x would silently produce three times the
        // pixels asked for — which is the bug this whole type exists to avoid.
        let format = UIGraphicsImageRendererFormat.default()
        format.scale = 1
        format.opaque = true
        return UIGraphicsImageRenderer(size: size, format: format).image { _ in
            image.draw(in: CGRect(origin: .zero, size: size))
        }
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
