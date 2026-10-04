import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// What the notification list holds for one pane that blocked and then
/// finished (ov-163): the two banners it used to stack (two identifiers), and
/// the one it leaves now (one identifier, so the second replaced the first).
/// The cards are a stand-in for the system's, around the real words and the
/// real identifiers. Both appearances, written where `FARCOOLER_GLANCE_OUT`
/// says.
@MainActor
struct PaneBannerSpecimenTests {
    private static func terminal(_ activity: String) throws -> Terminal {
        try JSONDecoder().decode(
            Terminal.self,
            from: Data(
                """
                {"id":"t-1","short":"s","title":"claude","preset":"claude","state":"running",
                "activity":"\(activity)","line":"","feed":["Added the retry."],"said":"Added the retry.","epoch":0}
                """.utf8))
    }

    private struct Card: View {
        let title: String
        let body_: String
        var body: some View {
            VStack(alignment: .leading, spacing: 2) {
                Text(title).font(.system(size: 13, weight: .semibold))
                Text(body_).font(.system(size: 12)).foregroundStyle(.secondary)
            }
            .padding(10)
            .frame(width: 300, alignment: .leading)
            .background(.quaternary.opacity(0.5))  // style-exempt: a stand-in for the system's banner
            .clipShape(RoundedRectangle(cornerRadius: 10))  // style-exempt: a stand-in for the system's banner
        }
    }

    @Test("Write the pane banner sheets")
    func writeSheets() throws {
        let blocked = try #require(Notifier.words(for: Self.terminal("blocked"), place: "Billing · fc-3-webhooks"))
        let done = try #require(Notifier.words(for: Self.terminal("done"), place: "Billing · fc-3-webhooks"))
        let directory = URL(
            fileURLWithPath: ProcessInfo.processInfo.environment["FARCOOLER_GLANCE_OUT"]
                ?? FileManager.default.currentDirectoryPath + "/.build/glance")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        for dark in [false, true] {
            let specimen = HStack(alignment: .top, spacing: 28) {
                VStack(alignment: .leading, spacing: 8) {
                    Text("Before: two identifiers, two banners").font(.caption).foregroundStyle(.secondary)
                    Card(title: done.title, body_: done.body)
                    Card(title: blocked.title, body_: blocked.body)
                }
                VStack(alignment: .leading, spacing: 8) {
                    Text("Now: one identifier, so the later replaces the earlier")
                        .font(.caption).foregroundStyle(.secondary)
                    Card(title: done.title, body_: done.body)
                }
            }
            .padding(20)
            .environment(\.colorScheme, dark ? .dark : .light)
            let host = NSHostingView(
                rootView: specimen.background(dark ? Color(white: 0.12) : Color(white: 0.96)))
            host.appearance = NSAppearance(named: dark ? .darkAqua : .aqua)
            host.frame = CGRect(origin: .zero, size: host.fittingSize)
            host.layoutSubtreeIfNeeded()
            let rep = try #require(host.bitmapImageRepForCachingDisplay(in: host.bounds))
            host.cacheDisplay(in: host.bounds, to: rep)
            let png = try #require(rep.representation(using: .png, properties: [:]))
            try png.write(to: directory.appendingPathComponent("pane-banners-\(dark ? "dark" : "light").png"))
        }
    }
}
