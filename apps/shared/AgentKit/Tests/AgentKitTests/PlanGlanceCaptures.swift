import Foundation
import Testing

@testable import AgentKit

// Captures of the plan on the glance (ov-310), opt-in like every capture: set
// `FARCOOLER_CAPTURE_OUT` to a directory and the PNGs land there, in light
// and dark. Off in CI and in a normal run.
//
// What this draws is the shipped `PlanGlanceView`, the one view the phone's
// Plan widget, the Live Activity's lock screen card, the watch app and its
// complication all put the glance in, at each surface's size, and the Live
// Activity's rows card composed as `LockScreenCard` composes it, from the
// relay's own card fixture. What it can't draw is the system's chrome around
// them: the widget container, the lock screen's vibrancy and the watch face's
// tinting are WidgetKit's, and only a device shows those.
#if os(macOS)
    import AppKit
    import SwiftUI

    @MainActor
    struct PlanGlanceCaptures {
        nonisolated static let out = ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_OUT"]

        /// A surface the glance is drawn on: its size, its ground and how the
        /// glance sits in it.
        struct Surface {
            let name: String
            let size: CGSize
            let padding: CGFloat
            let style: PlanGlanceView.Style
            let ground: (ColorScheme) -> Color
        }

        static let surfaces = [
            // A small home screen widget, on the container's tertiary fill.
            Surface(name: "widget-small", size: CGSize(width: 170, height: 170), padding: 16, style: .rows) {
                $0 == .dark ? Color(white: 0.11) : Color(white: 0.95)
            },
            Surface(name: "widget-medium", size: CGSize(width: 364, height: 170), padding: 16, style: .rows) {
                $0 == .dark ? Color(white: 0.11) : Color(white: 0.95)
            },
            // The lock screen's rectangular accessory.
            Surface(name: "lock-rectangular", size: CGSize(width: 172, height: 76), padding: 0, style: .lines) {
                $0 == .dark ? Color(white: 0.05) : Color(white: 0.85)
            },
            // The watch's Smart Stack slot, 45 mm.
            Surface(name: "watch-rectangular", size: CGSize(width: 184, height: 89), padding: 8, style: .lines) {
                $0 == .dark ? Color.black : Color(white: 0.2)
            },
            // The watch app's Plan section row.
            Surface(name: "watch-app-row", size: CGSize(width: 198, height: 110), padding: 10, style: .rows) {
                $0 == .dark ? Color(white: 0.14) : Color(white: 0.22)
            },
        ]

        @Test(
            "Captures of the plan on the glance, light and dark",
            .enabled(if: out != nil, "capture-only; set FARCOOLER_CAPTURE_OUT"))
        func capture() throws {
            let dir = URL(fileURLWithPath: try #require(Self.out))
            try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
            let glance = PlanGlanceTests.main
            for scheme in [ColorScheme.light, .dark] {
                let suffix = scheme == .dark ? "dark" : "light"
                for surface in Self.surfaces {
                    // The watch draws dark whatever the phone's appearance.
                    let drawn: ColorScheme = surface.name.hasPrefix("watch") ? .dark : scheme
                    let view = PlanGlanceView(glance, style: surface.style)
                        .padding(surface.padding)
                        .frame(width: surface.size.width, height: surface.size.height, alignment: .topLeading)
                        .background(surface.ground(scheme))
                    try write(view, scheme: drawn, to: dir.appendingPathComponent("\(surface.name)-\(suffix).png"))
                }
                try write(liveActivity(scheme), scheme: scheme, to: dir.appendingPathComponent("live-activity-\(suffix).png"))
            }
        }

        /// The Live Activity's rows card with the plan under it, as
        /// `LockScreenCard` stacks them, from the relay's card fixture.
        private func liveActivity(_ scheme: ColorScheme) throws -> some View {
            let update = try #require(try Contracts.object("live-activity/running/plan.json")["aps"] as? [String: Any])
            let state = try JSONSerialization.data(withJSONObject: try #require(update["content-state"]))
            let card = try JSONDecoder().decode(AgentCardState.self, from: state)
            let now = Date(timeIntervalSince1970: 1_791_019_830)
            let layout = try #require(AgentCardLayout(state: card, now: now, stale: false))
            let plan = try #require(card.plan)
            return VStack(alignment: .leading, spacing: 0) {
                GlanceCardView(layout: layout)
                PlanGlanceView(plan, style: .lines)
                    .padding(.horizontal, 14)
                    .padding(.bottom, 12)
            }
            .frame(width: 364, height: 200, alignment: .top)
            .background(scheme == .dark ? GlancePalette.card.dark.color : Color(white: 0.73))
        }

        private func write<V: View>(_ view: V, scheme: ColorScheme, to url: URL) throws {
            let renderer = ImageRenderer(content: view.environment(\.colorScheme, scheme))
            renderer.scale = 2
            let image = try #require(renderer.cgImage)
            let rep = NSBitmapImageRep(cgImage: image)
            try #require(rep.representation(using: .png, properties: [:])).write(to: url)
        }
    }
#endif
