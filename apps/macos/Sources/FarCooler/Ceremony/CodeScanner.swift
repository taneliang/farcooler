// AVFoundation predates Sendable. The capture session is confined to the
// serial queue below for every blocking lifecycle call.
@preconcurrency import AVFoundation
import AppKit
import SwiftUI
import Vision

/// One lane for every blocking capture-session lifecycle call.
///
/// `AVCaptureSession` does not tolerate `startRunning` and `stopRunning` at the
/// same time. Keeping the queue in a small type makes that guarantee testable
/// without asking CI for a camera.
final class CaptureSessionQueue: @unchecked Sendable {
    private let queue: DispatchQueue

    init(label: String = "com.farcooler.scanner") {
        queue = DispatchQueue(label: label)
    }

    func perform(_ operation: @escaping @Sendable () -> Void) async {
        await withCheckedContinuation { continuation in
            queue.async {
                operation()
                continuation.resume()
            }
        }
    }

    func enqueue(_ operation: @escaping @Sendable () -> Void) {
        queue.async(execute: operation)
    }
}

/// The first QR payload out of a stream of camera frames.
///
/// **The Mac decodes its own frames; the phone does not.** It is tempting to
/// mirror the iOS scanner, which hands `AVCaptureMetadataOutput` a
/// `metadataObjectTypes` of `[.qr]` and is done. That output reads
/// machine-readable codes on iOS ONLY. On macOS the very same class offers
/// `face`, `humanBody`, `humanHead`, `humanHand`, `catHead`, `dogBody` and
/// `salientObject` — the whole list, every one of them a body, and not one code
/// symbology anywhere in it.
///
/// Asking it for `.qr` therefore raises `NSInvalidArgumentException`, and that
/// is worse than it sounds: an Objective-C exception is not a Swift error, no
/// `catch` here can stop it, it skips every `defer` on the way out — leaving the
/// session wedged mid-configuration with the camera never started — and AppKit
/// swallows it at the run loop, so the app dies seconds later somewhere
/// unrelated with a backtrace pointing at innocent SwiftUI layout. It shipped
/// once. The fix is not to guard the assignment; it is to not have an API here
/// that cannot do this job.
///
/// Vision reads codes on both platforms, so this end reads its own frames.
final class FrameReader: NSObject, @unchecked Sendable {
    /// Called once, on the frame queue, with the payload and the instant it was
    /// read — the instant belongs to the frame rather than to whenever the main
    /// actor gets round to it, because every ceremony call is scored on
    /// `held_ms` by this device's own clock.
    private let read: @Sendable (String, Date) -> Void
    private let lock = NSLock()
    private var claimed = false

    init(read: @escaping @Sendable (String, Date) -> Void) {
        self.read = read
        super.init()
    }

    /// Take the right to report the one code, if it is still going.
    ///
    /// Frames arrive faster than the sheet reacts to them, so the decision has
    /// to be made here, on the frame queue, and not by a `scanned == nil` check
    /// on the main actor that three frames can pass at once.
    func claim() -> Bool {
        lock.withLock {
            if claimed { return false }
            claimed = true
            return true
        }
    }

    /// Let this reader report one more code. "Try Again" on a refusal.
    ///
    /// Without this a retry re-enters a still-claimed reader, and the camera
    /// runs forever without ever reporting the code being held up to it.
    func resume() {
        lock.withLock { claimed = false }
    }

    /// The payload of the one QR code in an image, if there is one.
    ///
    /// Free of the camera on purpose, so a test can hand it a code this app
    /// generated and check that this app reads it back.
    static func payload(in image: CGImage) -> String? {
        decode(VNImageRequestHandler(cgImage: image, options: [:]))
    }

    static func payload(in buffer: CVPixelBuffer) -> String? {
        decode(VNImageRequestHandler(cvPixelBuffer: buffer, options: [:]))
    }

    private static func decode(_ handler: VNImageRequestHandler) -> String? {
        let request = VNDetectBarcodesRequest()
        request.symbologies = [.qr]
        try? handler.perform([request])
        return request.results?.compactMap(\.payloadStringValue).first
    }
}

extension FrameReader: AVCaptureVideoDataOutputSampleBufferDelegate {
    func captureOutput(
        _ output: AVCaptureOutput,
        didOutput sampleBuffer: CMSampleBuffer,
        from connection: AVCaptureConnection
    ) {
        guard let buffer = CMSampleBufferGetImageBuffer(sampleBuffer),
            let payload = Self.payload(in: buffer),
            claim()
        else { return }
        read(payload, Date())
    }
}

/// The camera, pointed at one code.
///
/// Stops after the first payload it reads. A scanner that keeps firing is a
/// scanner that reads the second code in the frame — and during onboarding
/// there are frequently two, because the other device is showing one back.
///
/// `@MainActor` throughout, with the frame reader hopping onto it: the
/// published value drives a sheet, and a sheet driven from a capture queue is a
/// crash waiting for a slow frame.
@MainActor
final class CodeScanner: ObservableObject {
    /// The payload of the one code this scanner read, and when it read it.
    ///
    /// The instant travels WITH the payload because every ceremony call takes
    /// `held_ms` — how long this device has held the code, by its own clock,
    /// which is the only clock that counts. Keeping the two together is what
    /// stops a later caller from measuring from the wrong moment.
    @Published private(set) var scanned: (payload: String, at: Date)?

    /// Empty until the camera says no. Not a Rust error and not an
    /// `AVFoundation` code: the two things that go wrong here are "you have not
    /// allowed the camera" and "there is no camera", and both have a sentence.
    @Published private(set) var problem: String?

    let session = AVCaptureSession()
    private let lifecycle = CaptureSessionQueue()
    /// Decoding is slower than the frame rate and has no business on the main
    /// thread or on the lane that `startRunning` blocks.
    private let frames = DispatchQueue(label: "com.farcooler.scanner.frames")
    /// Held because `setSampleBufferDelegate` does not retain its delegate, and
    /// because "Try Again" has to re-arm this exact reader.
    private var reader: FrameReader?
    private var configured = false

    /// Ask for the camera, and start if it is granted.
    ///
    /// Asked when the scan screen opens, never at launch. A permission prompt
    /// that arrives with no explanation on screen behind it is a prompt people
    /// deny, and then the feature is broken for good in System Settings.
    func start() async {
        guard await granted() else {
            problem = "Camera access is off. Allow Far Cooler to use the camera in "
                + "System Settings › Privacy & Security › Camera."
            return
        }
        guard configure() else { return }
        // `startRunning` blocks until the camera is up, which on a Mac is long
        // enough to drop frames from the window server if it happens on the
        // main thread. Start and stop share one serial queue: using unrelated
        // detached tasks let a retry start the session while the previous scan
        // was still stopping it, which corrupts AVFoundation's session state.
        await lifecycle.perform { [session] in
            if !session.isRunning { session.startRunning() }
        }
    }

    func stop() {
        lifecycle.enqueue { [session] in
            if session.isRunning { session.stopRunning() }
        }
    }

    /// Forget what was read, so the same sheet can scan again.
    ///
    /// Used by "Try Again" on a refusal. The scan is discarded rather than
    /// re-decoded: a refused code is refused, and the way back is a fresh look
    /// at a fresh code.
    func rescan() {
        scanned = nil
        reader?.resume()
        Task { await start() }
    }

    private func granted() async -> Bool {
        switch AVCaptureDevice.authorizationStatus(for: .video) {
        case .authorized: return true
        case .notDetermined: return await AVCaptureDevice.requestAccess(for: .video)
        default: return false
        }
    }

    private func configure() -> Bool {
        if configured { return true }
        guard let camera = AVCaptureDevice.default(for: .video),
            let input = try? AVCaptureDeviceInput(device: camera)
        else {
            problem = "No camera is available."
            return false
        }

        let output = AVCaptureVideoDataOutput()
        // Decoding a frame takes longer than the next one takes to arrive. A
        // backlog would have this reading a code off a frame the camera stopped
        // seeing seconds ago, so late frames are dropped rather than queued.
        output.alwaysDiscardsLateVideoFrames = true

        let reader = FrameReader { [weak self] payload, at in
            Task { @MainActor in
                guard let self, self.scanned == nil else { return }
                self.scanned = (payload, at)
                self.stop()
            }
        }
        output.setSampleBufferDelegate(reader, queue: frames)
        self.reader = reader

        if let failure = attach(input, output) {
            problem = failure
            return false
        }

        configured = true
        return true
    }

    /// Put the camera and the frame reader on the session, in one transaction.
    ///
    /// Returns the sentence to show if either could not be added, and nil if
    /// both went on.
    private func attach(
        _ input: AVCaptureDeviceInput, _ output: AVCaptureVideoDataOutput
    ) -> String? {
        session.beginConfiguration()
        defer { session.commitConfiguration() }

        guard session.canAddInput(input) else { return "The camera is in use by another app." }
        session.addInput(input)
        guard session.canAddOutput(output) else { return "The camera can’t scan codes." }
        session.addOutput(output)

        // Said out loud rather than left to the default, because a mirrored QR
        // code does not decode at all — it is not a rotation, it is a different
        // symbol. The preview layer may still mirror for the person on this
        // side of the camera; these frames must not.
        if let connection = output.connection(with: .video),
            connection.isVideoMirroringSupported
        {
            connection.automaticallyAdjustsVideoMirroring = false
            connection.isVideoMirrored = false
        }
        return nil
    }
}

/// What the camera sees, in a SwiftUI view.
struct CameraPreview: NSViewRepresentable {
    let session: AVCaptureSession

    func makeNSView(context: Context) -> NSView {
        let view = NSView()
        view.wantsLayer = true
        let preview = AVCaptureVideoPreviewLayer(session: session)
        preview.videoGravity = .resizeAspectFill
        preview.frame = view.bounds
        preview.autoresizingMask = [.layerWidthSizable, .layerHeightSizable]
        view.layer = preview
        return view
    }

    func updateNSView(_ view: NSView, context: Context) {}
}
