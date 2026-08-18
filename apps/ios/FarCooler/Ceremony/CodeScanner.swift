// `@preconcurrency` because AVFoundation predates Sendable and an
// `AVCaptureSession` is documented as safe to use from any one thread at a
// time — which is exactly what the serial queue below gives it. Without this
// every hop onto that queue is a warning about a promise the framework never
// made.
@preconcurrency import AVFoundation
import SwiftUI

/// The camera, pointed at one QR code.
///
/// It publishes the first payload it reads and stops the session in the same
/// breath: a scanner that keeps firing is a scanner that scans the second code
/// in the frame, and the second code in the frame is not the one anybody aimed
/// at. Whether that payload means anything is not this file's question — every
/// rule about that is behind the ceremony FFI.
@MainActor
final class CodeScanner: NSObject, ObservableObject {
    /// The first code read, or nil until there is one. Set exactly once per
    /// `start()`.
    @Published private(set) var scanned: String?
    /// Camera access was refused, or this build cannot have it. Distinguished
    /// from "no camera" because the two have different answers: one is Settings,
    /// the other is the manual path.
    @Published private(set) var denied = false
    /// There is no camera to use — a simulator, mostly.
    @Published private(set) var unavailable = false

    let session = AVCaptureSession()

    /// Configuration and `startRunning` both block, and blocking the main
    /// thread while a camera warms up is a visible stall on the screen that
    /// just appeared.
    private let queue = DispatchQueue(label: "com.farcooler.scanner")
    private var configured = false

    /// Ask for the camera, then run it.
    ///
    /// The permission prompt carries `NSCameraUsageDescription` from
    /// `Info.plist` — "Far Cooler uses the camera to scan the code on a device
    /// you're adding" — which is why the ask happens here, on the screen that
    /// needs it, rather than at launch.
    func start() {
        scanned = nil
        switch AVCaptureDevice.authorizationStatus(for: .video) {
        case .authorized:
            run()
        case .notDetermined:
            AVCaptureDevice.requestAccess(for: .video) { [weak self] granted in
                Task { @MainActor in
                    guard let self else { return }
                    if granted { self.run() } else { self.denied = true }
                }
            }
        default:
            denied = true
        }
    }

    func stop() {
        queue.async { [session] in
            if session.isRunning { session.stopRunning() }
        }
    }

    private func run() {
        denied = false
        guard configure() else {
            unavailable = true
            return
        }
        queue.async { [session] in
            if !session.isRunning { session.startRunning() }
        }
    }

    /// Wire the camera to a metadata output that reads QR codes and nothing
    /// else. Once: a second `addInput` on a running session throws.
    private func configure() -> Bool {
        if configured { return true }
        guard
            let device = AVCaptureDevice.default(for: .video),
            let input = try? AVCaptureDeviceInput(device: device),
            session.canAddInput(input)
        else { return false }

        let output = AVCaptureMetadataOutput()
        guard session.canAddOutput(output) else { return false }

        session.beginConfiguration()
        session.addInput(input)
        session.addOutput(output)
        output.setMetadataObjectsDelegate(self, queue: DispatchQueue.main)
        // Set AFTER the output joins the session: the list of types an output
        // will accept is empty until it has a session to report what the
        // hardware can do, and assigning `.qr` before that traps.
        output.metadataObjectTypes = [.qr]
        session.commitConfiguration()

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
        // Delivered on the main queue, which is what this delegate was
        // registered with, so hopping through `assumeIsolated` costs nothing
        // and keeps the published properties where SwiftUI expects them.
        MainActor.assumeIsolated {
            guard scanned == nil else { return }
            guard
                let code = objects.first as? AVMetadataMachineReadableCodeObject,
                code.type == .qr,
                let payload = code.stringValue
            else { return }

            // Stopped before the payload is published, not after: whatever the
            // publisher triggers, this session has already read its one code.
            stop()
            scanned = payload
        }
    }
}

/// What the camera is seeing, as a view.
///
/// A layer-backed `UIView` rather than a `UIViewRepresentable` that adds a
/// sublayer: a preview layer that is the view's own layer resizes with the
/// view, and one added beside it does not — which shows up as a picture frozen
/// at its first size the moment the phone rotates.
struct CameraPreview: UIViewRepresentable {
    let session: AVCaptureSession

    func makeUIView(context: Context) -> PreviewView {
        let view = PreviewView()
        view.previewLayer.session = session
        view.previewLayer.videoGravity = .resizeAspectFill
        return view
    }

    func updateUIView(_ view: PreviewView, context: Context) {
        view.previewLayer.session = session
    }

    final class PreviewView: UIView {
        override class var layerClass: AnyClass { AVCaptureVideoPreviewLayer.self }
        // swiftlint:disable:next force_cast
        var previewLayer: AVCaptureVideoPreviewLayer { layer as! AVCaptureVideoPreviewLayer }
    }
}

/// The scanning screen: a viewfinder, one sentence, and a way out.
///
/// Shared by both sides of the ceremony, because both are the same act — point
/// the camera at the code the other device is showing.
struct ScanScreen: View {
    @ObservedObject var scanner: CodeScanner
    let instruction: String
    let onCancel: () -> Void

    var body: some View {
        VStack(spacing: 20) {
            if scanner.denied {
                message(
                    "Camera access is off",
                    "Allow Far Cooler to use the camera in Settings, or add this device with "
                        + "its public key."
                )
            } else if scanner.unavailable {
                message(
                    "Camera not available",
                    "Add the device with its public key instead."
                )
            } else {
                CameraPreview(session: scanner.session)
                    .clipShape(RoundedRectangle(cornerRadius: 16))
                    .frame(maxWidth: .infinity)
                    .frame(height: 360)

                Text(instruction)
                    .font(.callout)
                    .foregroundStyle(.secondary)
                    .multilineTextAlignment(.center)
            }

            Button("Cancel", action: onCancel)
                .padding(.top, 4)
        }
        .padding()
        .onAppear { scanner.start() }
        .onDisappear { scanner.stop() }
    }

    private func message(_ title: String, _ body: String) -> some View {
        VStack(spacing: 10) {
            Text(title).font(.headline)
            Text(body)
                .font(.callout)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
        }
        .padding(.vertical, 40)
    }
}
