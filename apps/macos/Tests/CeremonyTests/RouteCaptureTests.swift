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

    private func write(_ view: NSView, _ name: String) {
        guard let out, let rep = view.bitmapImageRepForCachingDisplay(in: view.bounds) else { return }
        view.cacheDisplay(in: view.bounds, to: rep)
        try? rep.representation(using: .png, properties: [:])?.write(to: out.appendingPathComponent("\(name).png"))
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
        let one = RouteQuestion(runners: [.init(target: "fc-cosmo", address: "192.168.1.180")])
        let several = RouteQuestion(runners: [
            .init(target: "fc-cosmo", address: "192.168.1.180"),
            .init(target: "fc-build-machine-in-the-basement", address: "fe80::1c2b:3aff:fe4d:5e6f"),
            .init(target: "fc-mini", address: "mini.local:2222"),
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
