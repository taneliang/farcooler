import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// Pictures of the Mac's Tailcat route screens for a person to look at
/// (ov-485 fix 1, C5): the route question's alert, the Settings > Runners
/// route row, and Add Device's Tailcat note, light and dark, short and long.
/// Opt-in: runs only with `FARCOOLER_CAPTURE_OUT` set, and never in CI.
@MainActor
struct RouteCaptureTests {
    private var out: URL? {
        ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_OUT"].map { URL(fileURLWithPath: $0) }
    }

    /// The view over the window's own background, in the view's appearance.
    /// An alert's content view draws no background of its own, so without
    /// this a dark capture is white text on transparency, which reads blank.
    private func write(_ view: NSView, _ name: String) {
        guard let out, let rep = view.bitmapImageRepForCachingDisplay(in: view.bounds) else { return }
        view.cacheDisplay(in: view.bounds, to: rep)
        let image = NSImage(size: view.bounds.size)
        image.addRepresentation(rep)
        guard
            let flat = NSBitmapImageRep(
                bitmapDataPlanes: nil, pixelsWide: rep.pixelsWide, pixelsHigh: rep.pixelsHigh,
                bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false,
                colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)
        else { return }
        flat.size = view.bounds.size
        NSGraphicsContext.saveGraphicsState()
        NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: flat)
        view.effectiveAppearance.performAsCurrentDrawingAppearance {
            NSColor.windowBackgroundColor.setFill()
            NSRect(origin: .zero, size: view.bounds.size).fill()
        }
        image.draw(in: NSRect(origin: .zero, size: view.bounds.size))
        NSGraphicsContext.restoreGraphicsState()
        try? flat.representation(using: .png, properties: [:])?.write(to: out.appendingPathComponent("\(name).png"))
    }

    private func render<V: View>(_ view: V, width: CGFloat, name: String) {
        for dark in [false, true] {
            let host = NSHostingView(rootView: view.frame(width: width).fixedSize(horizontal: false, vertical: true))
            let size = host.fittingSize
            host.frame = NSRect(x: 0, y: 0, width: width, height: max(size.height, 20))
            let window = NSWindow(contentRect: host.frame, styleMask: [.titled], backing: .buffered, defer: false)
            window.appearance = NSAppearance(named: dark ? .darkAqua : .aqua)
            window.contentView = host
            host.layoutSubtreeIfNeeded()
            host.display()
            write(host, "\(name)-\(dark ? "dark" : "light")")
        }
    }

    @Test func captures() {
        guard out != nil else { return }
        let one = RouteQuestion(runners: [.init(target: "fc-cosmo", address: "192.168.1.180", name: "cosmo")])
        let several = RouteQuestion(runners: [
            .init(target: "fc-cosmo", address: "192.168.1.180", name: "cosmo"),
            .init(target: "fc-build", address: "fe80::1c2b:3aff:fe4d:5e6f", name: "Build Machine in the Basement"),
            .init(target: "fc-mini", address: "mini.local:2222", name: "Mac mini"),
        ])
        for (question, name) in [(one, "mac-route-alert"), (several, "mac-route-alert-several")] {
            for dark in [false, true] {
                let alert = RoutePrompt.makeAlert(question)
                alert.window.appearance = NSAppearance(named: dark ? .darkAqua : .aqua)
                alert.layout()
                if let content = alert.window.contentView {
                    content.layoutSubtreeIfNeeded()
                    content.display()
                    write(content, "\(name)-\(dark ? "dark" : "light")")
                }
            }
        }
        for (width, label) in [(CGFloat(360), "narrow"), (CGFloat(640), "wide")] {
            render(
                Form {
                    VStack(alignment: .leading, spacing: 6) {
                        Text("fc-cosmo")
                        RunnerRouteRow(route: .tunnel) {}
                    }
                    VStack(alignment: .leading, spacing: 6) {
                        Text("fc-build-machine-in-the-basement-with-a-long-name")
                        RunnerRouteRow(route: .direct(address: "fe80::1c2b:3aff:fe4d:5e6f%en0-long-host.example.com")) {}
                    }
                }.formStyle(.grouped),
                width: width, name: "mac-runners-route-row-\(label)")
        }
        render(
            VStack(alignment: .leading) {
                TailcatPairingNote(name: "E-Liang’s iPhone 17 Pro Max")
            }.padding(20),
            width: 460, name: "mac-add-device-tailcat-note")
    }
}
