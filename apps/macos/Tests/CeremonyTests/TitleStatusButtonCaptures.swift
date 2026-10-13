import AppKit
import Testing

@testable import Far_Cooler

/// Every status button at rest, hovered and pressed, at a narrow and a wide
/// window, with counts of 1, 8 and 120, light and dark (ov-511), as PNGs to
/// read. Opt-in, never in CI: `FARCOOLER_STATUS_BUTTONS_OUT`, a directory.
@MainActor
@Suite(.serialized)
struct TitleStatusButtonCaptures {
    @Test(.enabled(if: ProcessInfo.processInfo.environment["FARCOOLER_STATUS_BUTTONS_OUT"] != nil))
    func capture() async throws {
        let out = URL(fileURLWithPath: try #require(ProcessInfo.processInfo.environment["FARCOOLER_STATUS_BUTTONS_OUT"]))
        try FileManager.default.createDirectory(at: out, withIntermediateDirectories: true)
        for (look, appearance) in [("light", NSAppearance.Name.aqua), ("dark", .darkAqua)] {
            for (name, width, ids) in [
                ("narrow", CGFloat(900), Array(TitleStatusButtonKit.ids.dropFirst())), ("wide", 1400, TitleStatusButtonKit.ids),
            ] {
                for count in [1, 8, 120] {
                    var states: [(String, (id: String, plate: TitleStatusPlate)?)] = [("rest", nil)]
                    for id in ids {
                        states.append(("\(id.dropFirst("title-status-".count))-hover", (id, .hover)))
                        states.append(("\(id.dropFirst("title-status-".count))-pressed", (id, .pressed)))
                    }
                    for (state, forced) in states {
                        let shot = try await TitleStatusButtonKit.shot(
                            width: width, count: count, appearance: appearance, forced: forced)
                        let png = try #require(shot.rep.representation(using: .png, properties: [:]))
                        try png.write(to: out.appendingPathComponent("\(name)-\(count)-\(look)-\(state).png"))
                    }
                }
            }
        }
    }
}
