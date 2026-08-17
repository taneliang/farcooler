import AppKit
import SwiftUI
import UniformTypeIdentifiers

/// Pasting or dropping a file into a terminal.
///
/// A terminal takes bytes and an agent running in one opens a path, so anything
/// handed to a pane has to become a file on the runner that pane is on before
/// it can become anything the agent can see. The daemon does that and types the
/// path; this decides what to hand it, and says so while it is happening.
///
/// Any file type. What an agent does with a `.parquet` is its business, and
/// refusing to carry one would be this app deciding on its behalf.

/// A failure with something worth showing a person.
///
/// Its own type only because `Result` needs an `Error`, and the whole payload
/// is the sentence the chip shows.
struct PasteFailure: Error {
    let message: String
}

/// What the pasteboard, or a drop, actually gave us.
enum PastedImage {
    /// A file that already exists on this Mac.
    case file(URL)
    /// Bytes with no path — a screenshot, a copy out of Preview.
    case data(Data, mime: String)
}

/// One image on its way into a pane.
@MainActor
final class ImagePasteJob: ObservableObject, Identifiable {
    let id = UUID()
    let thumbnail: NSImage?
    @Published var sent: Int64 = 0
    @Published var total: Int64 = 0
    @Published var failure: String?

    /// Whether this job should be drawn while it is working.
    ///
    /// False against a local daemon, where the transfer is a unix socket and a
    /// ring that flashes for one frame is noise. A failure is still shown: that
    /// is worth interrupting for however fast it happened.
    let quiet: Bool

    /// Run again, for the Retry button.
    var retry: (() -> Void)?

    init(thumbnail: NSImage?, quiet: Bool) {
        self.thumbnail = thumbnail
        self.quiet = quiet
    }

    var fraction: Double {
        guard total > 0 else { return 0 }
        return min(1, Double(sent) / Double(total))
    }

    var visible: Bool { failure != nil || !quiet }
}

/// The jobs for one pane.
@MainActor
final class ImagePasteQueue: ObservableObject {
    @Published private(set) var jobs: [ImagePasteJob] = []

    /// Set by `TerminalSurface` once the view exists. Only the local-file case
    /// uses it — every other path lets the daemon do the typing, which is what
    /// keeps the bracketing decided where the pane is.
    var type: ((String) -> Void)?

    func start(
        _ pasted: PastedImage,
        terminal: String,
        binary: String?,
        environment: [String: String],
        hostArguments: [String]
    ) {
        let isLocal = hostArguments.isEmpty

        // A file already on the runner the agent runs on needs nothing done to
        // it. Copying would put the same image in two places with two
        // lifetimes, and the one the daemon owns would expire while the user's
        // own copy sat there forever.
        if isLocal, case .file(let url) = pasted {
            type?(quoteForPaste(url.path) + " ")
            return
        }

        let job = ImagePasteJob(thumbnail: thumbnail(of: pasted), quiet: isLocal)
        let run = { [weak self] in
            guard let self else { return }
            job.failure = nil
            job.sent = 0
            Task { await self.send(pasted, job: job, terminal: terminal, binary: binary,
                                   environment: environment, hostArguments: hostArguments) }
        }
        job.retry = run
        jobs.append(job)
        run()
    }

    func dismiss(_ job: ImagePasteJob) {
        jobs.removeAll { $0.id == job.id }
    }

    private func send(
        _ pasted: PastedImage,
        job: ImagePasteJob,
        terminal: String,
        binary: String?,
        environment: [String: String],
        hostArguments: [String]
    ) async {
        guard let binary else {
            job.failure = "Far Cooler couldn't find its command line tool."
            return
        }

        let file: URL
        switch stage(pasted) {
        case .success(let url): file = url
        case .failure(let reason):
            job.failure = reason.message
            return
        }
        // Only what we wrote ourselves. A file the user already had is theirs.
        let staged = file.path.hasPrefix(NSTemporaryDirectory())
        defer { if staged { try? FileManager.default.removeItem(at: file) } }

        let attributes = try? FileManager.default.attributesOfItem(atPath: file.path)
        let size = (attributes?[.size] as? NSNumber)?.int64Value ?? 0
        job.total = size
        // Refused here rather than after the upload: on a phone-speed link the
        // difference is a minute of waiting to be told no.
        if size > 16 * 1024 * 1024 {
            job.failure = "That file is too large to send. Files up to 16 MB work."
            return
        }

        let arguments = hostArguments + ["terminal", "paste-file", terminal, file.path]
        let outcome = await ImagePasteProcess.run(
            binary: binary, arguments: arguments, environment: environment,
            onProgress: { sent, total in
                Task { @MainActor in
                    job.sent = sent
                    if total > 0 { job.total = total }
                }
            })

        switch outcome {
        case .success:
            dismiss(job)
        case .failure(let reason):
            job.failure = reason.message
        }
    }

    /// Give the CLI a file to send.
    ///
    /// A file already on disk is sent exactly as it is — no re-encode, because
    /// the bytes ARE the thing being sent: a round trip through `NSImage` would
    /// destroy a PDF outright and soften a screenshot's text for nothing. Only
    /// raw pasteboard bytes, which have no file yet, get written to one.
    private func stage(_ pasted: PastedImage) -> Result<URL, PasteFailure> {
        switch pasted {
        case .file(let url):
            return .success(url)
        case .data(let data, let mime):
            let ext = mime == "image/jpeg" ? "jpg" : "png"
            return write(data, extension: ext)
        }
    }

    private func write(_ data: Data, extension ext: String) -> Result<URL, PasteFailure> {
        let url = URL(fileURLWithPath: NSTemporaryDirectory())
            .appendingPathComponent("farcooler-paste-\(UUID().uuidString).\(ext)")
        do {
            try data.write(to: url)
            return .success(url)
        } catch {
            return .failure(PasteFailure(message: "Far Cooler couldn't prepare that file to send."))
        }
    }

    private func thumbnail(of pasted: PastedImage) -> NSImage? {
        switch pasted {
        case .file(let url): return NSImage(contentsOf: url)
        case .data(let data, _): return NSImage(data: data)
        }
    }
}

/// The path as it should be typed into a terminal.
///
/// The same rule as the daemon's `quote_for_paste`, and duplicated for the one
/// case the daemon never sees: a file already on this Mac, pasted into a pane
/// on this Mac, where nothing crosses the wire to be quoted on the far side.
func quoteForPaste(_ path: String) -> String {
    let needs = path.contains { $0.isWhitespace || "\"'\\$`".contains($0) }
    guard needs else { return path }
    var out = "\""
    for c in path {
        if c == "\"" || c == "\\" || c == "$" || c == "`" { out.append("\\") }
        out.append(c)
    }
    out.append("\"")
    return out
}

/// Run the CLI, reading progress off stderr as it goes.
///
/// `DaemonClient.runRaw` reads stderr to the end, which is right for the
/// thirty commands that only report a failure — but a transfer that reports
/// nothing until it finishes has no progress to draw.
enum ImagePasteProcess {
    static func run(
        binary: String,
        arguments: [String],
        environment: [String: String],
        onProgress: @escaping @Sendable (Int64, Int64) -> Void
    ) async -> Result<String, PasteFailure> {
        await withCheckedContinuation { continuation in
            DispatchQueue.global(qos: .userInitiated).async {
                let process = Process()
                process.executableURL = URL(fileURLWithPath: binary)
                process.arguments = arguments
                process.environment = environment

                let out = Pipe()
                let err = Pipe()
                process.standardOutput = out
                process.standardError = err

                do {
                    try process.run()
                } catch {
                    continuation.resume(
                        returning: .failure(PasteFailure(message: error.localizedDescription)))
                    return
                }

                // Read stderr incrementally so the ring moves. stdout is small
                // — one path — and is drained after the process ends.
                //
                // Every line that is NOT progress is kept, because one of them
                // is the reason this failed. Splitting the stream and holding
                // only the trailing partial line threw the error away: it ends
                // with a newline, so it was consumed as a complete line, found
                // not to be progress, and dropped — leaving the failure path
                // nothing to read and every failure reported as the catch-all,
                // "Couldn't reach this runner", whatever had actually gone
                // wrong. That cost a debugging session on a message that was
                // being produced correctly the whole time.
                var diagnostic = ""
                var trailing = ""
                let handle = err.fileHandleForReading
                while true {
                    let chunk = handle.availableData
                    if chunk.isEmpty { break }
                    trailing += String(data: chunk, encoding: .utf8) ?? ""
                    var lines = trailing.components(separatedBy: "\n")
                    trailing = lines.removeLast()
                    for line in lines {
                        if let (sent, total) = progress(in: line) {
                            onProgress(sent, total)
                        } else {
                            diagnostic += line + "\n"
                        }
                    }
                }
                diagnostic += trailing

                let stdout = out.fileHandleForReading.readDataToEndOfFile()
                process.waitUntilExit()

                if process.terminationStatus != 0 {
                    // Logged as well as mapped. The chip shows a sentence a
                    // person can act on; the reason it was chosen has to be
                    // recoverable afterwards, or the next unmapped failure is
                    // another debugging session.
                    let raw = diagnostic.trimmingCharacters(in: .whitespacesAndNewlines)
                    NSLog("farcooler: paste-file failed: %@", raw)
                    continuation.resume(
                        returning: .failure(PasteFailure(message: message(from: raw))))
                    return
                }
                let path = String(data: stdout, encoding: .utf8)?
                    .trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
                continuation.resume(returning: .success(path))
            }
        }
    }

    /// `sent 262144/1048576 bytes`.
    static func progress(in line: String) -> (Int64, Int64)? {
        guard line.hasPrefix("sent ") else { return nil }
        let body = line.dropFirst("sent ".count)
        guard let space = body.firstIndex(of: " ") else { return nil }
        let pair = body[body.startIndex..<space].split(separator: "/")
        guard pair.count == 2, let sent = Int64(pair[0]), let total = Int64(pair[1]) else {
            return nil
        }
        return (sent, total)
    }

    /// Turn whatever the CLI said into something worth showing someone.
    ///
    /// The CLI's own failures are developer-facing — a Rust error chain, a
    /// method name — and none of them belong in a terminal pane.
    static func message(from stderr: String) -> String {
        let text = stderr.lowercased()
        // Two causes, indistinguishable on the wire: a daemon too old
        // to know the method, or a pane that closed before the path
        // could be typed. Named together rather than guessed.
        if text.contains("predates this") {
            return "That terminal may have closed, or this runner's Far Cooler may be too old."
        }
        if text.contains("file size") {
            return "That file is too large to send. Files up to 16 MB work."
        }
        if text.contains("not found") {
            return "That terminal isn't running anymore."
        }
        return "Couldn't reach this runner."
    }
}

/// The chips for one pane, stacked at the bottom of it.
struct ImagePasteChips: View {
    @ObservedObject var queue: ImagePasteQueue

    var body: some View {
        VStack(spacing: 6) {
            ForEach(queue.jobs) { job in
                ImagePasteChip(job: job) { queue.dismiss(job) }
            }
        }
        .padding(.bottom, 10)
        .animation(.easeOut(duration: 0.15), value: queue.jobs.count)
    }
}

private struct ImagePasteChip: View {
    @ObservedObject var job: ImagePasteJob
    let onDismiss: () -> Void

    var body: some View {
        if job.visible {
            HStack(spacing: 8) {
                if let thumbnail = job.thumbnail {
                    Image(nsImage: thumbnail)
                        .resizable()
                        .aspectRatio(contentMode: .fill)
                        .frame(width: 24, height: 24)
                        .clipShape(RoundedRectangle(cornerRadius: 4))
                }

                if let failure = job.failure {
                    Text(failure).font(.callout)
                    Button("Retry") { job.retry?() }.buttonStyle(.link)
                    Button("Cancel", action: onDismiss).buttonStyle(.link)
                } else {
                    Text("Sending\u{2026}").font(.callout)
                    ProgressView(value: job.fraction)
                        .progressViewStyle(.circular)
                        .controlSize(.small)
                }
            }
            .padding(.horizontal, 10)
            .padding(.vertical, 7)
            .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 8))
            .overlay(RoundedRectangle(cornerRadius: 8).strokeBorder(.separator))
            .shadow(radius: 6, y: 2)
            .transition(.opacity)
        }
    }
}
