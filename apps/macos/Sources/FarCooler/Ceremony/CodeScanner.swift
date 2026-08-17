import AVFoundation
import AppKit
import SwiftUI

/// The camera, pointed at one code.
///
/// Stops after the first payload it reads. A scanner that keeps firing is a
/// scanner that reads the second code in the frame — and during onboarding
/// there are frequently two, because the other device is showing one back.
///
/// `@MainActor` throughout, with the AVFoundation delegate hopping onto it: the
/// published value drives a sheet, and a sheet driven from a capture queue is a
/// crash waiting for a slow frame.
@MainActor
final class CodeScanner: NSObject, ObservableObject {
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
    private var configured = false

    /// Ask for the camera, and start if it is granted.
    ///
    /// Asked when the scan screen opens, never at launch. A permission prompt
    /// that arrives with no explanation on screen behind it is a prompt people
    /// deny, and then the feature is broken for good in System Settings.
    func start() async {
        guard await granted() else {
            problem = "Far Cooler needs camera access to scan the code. "
                + "Allow it in System Settings › Privacy & Security › Camera."
            return
        }
        guard configure() else { return }
        guard !session.isRunning else { return }
        // `startRunning` blocks until the camera is up, which on a Mac is long
        // enough to drop frames from the window server if it happens on the
        // main thread.
        let held = Held(session)
        await Task.detached { held.session.startRunning() }.value
    }

    func stop() {
        guard session.isRunning else { return }
        let held = Held(session)
        Task.detached { held.session.stopRunning() }
    }

    /// An `AVCaptureSession` carried off the main actor, on purpose.
    ///
    /// `AVCaptureSession` is not `Sendable` and never will be — most of it is
    /// only safe between `beginConfiguration` and `commitConfiguration`, which
    /// is exactly why the compiler refuses to let it cross. But `startRunning`
    /// and `stopRunning` are the two calls Apple's own sample code makes from a
    /// dedicated queue precisely BECAUSE they block, and configuration here
    /// happens once, on the main actor, before either is ever called.
    ///
    /// So the unchecked part is checked by the shape of this file: nothing
    /// touches the session off the main actor except those two calls.
    private struct Held: @unchecked Sendable {
        let session: AVCaptureSession
        init(_ session: AVCaptureSession) { self.session = session }
    }

    /// Forget what was read, so the same sheet can scan again.
    ///
    /// Used by "Try Again" on a refusal. The scan is discarded rather than
    /// re-decoded: a refused code is refused, and the way back is a fresh look
    /// at a fresh code.
    func rescan() {
        scanned = nil
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
            problem = "This Mac has no camera Far Cooler can use."
            return false
        }

        session.beginConfiguration()
        defer { session.commitConfiguration() }
        guard session.canAddInput(input) else {
            problem = "This Mac's camera is in use by something else."
            return false
        }
        session.addInput(input)

        let output = AVCaptureMetadataOutput()
        guard session.canAddOutput(output) else {
            problem = "This Mac's camera can't read codes."
            return false
        }
        session.addOutput(output)
        output.setMetadataObjectsDelegate(self, queue: .main)
        // Set AFTER the output is added. Before it, the list of types the
        // session supports is empty and assigning `.qr` throws.
        output.metadataObjectTypes = [.qr]

        configured = true
        return true
    }
}

extension CodeScanner: AVCaptureMetadataOutputObjectsDelegate {
    nonisolated func metadataOutput(
        _ output: AVCaptureMetadataOutput,
        didOutput objects: [AVMetadataObject],
        from connection: AVCaptureConnection
    ) {
        let payloads = objects
            .compactMap { $0 as? AVMetadataMachineReadableCodeObject }
            .compactMap(\.stringValue)
        guard let first = payloads.first else { return }
        Task { @MainActor in
            guard self.scanned == nil else { return }
            self.scanned = (first, Date())
            self.stop()
        }
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
