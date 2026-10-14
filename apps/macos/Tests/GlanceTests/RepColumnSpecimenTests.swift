import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// A member's Talk on the Mac (ov-526): the rep's column in each state, and
/// the navigator's "Your Rep" row, drawn offscreen at a narrow and a wide
/// column, in both appearances, with a long account and a long refusal so the
/// wrapping can be looked at. A rendering, not an assertion beyond "it drew":
/// written where `FARCOOLER_GLANCE_OUT` says.
@MainActor
struct RepColumnSpecimenTests {
    private static let long = RunnerPeople(
        members: [], claudeAccount: "platform-team-shared-account@example-corporation.com", you: "bob")
    private static let short = RunnerPeople(members: [], claudeAccount: "team@example.com", you: "bob")

    private struct Specimen {
        let name: String
        let view: AnyView
    }

    private static func column(_ state: RepColumn.State, people: RunnerPeople, refusal: String? = nil, canAct: Bool = true) -> AnyView {
        AnyView(
            RepPlaceholder(
                state: state, intro: people.repIntro, refusal: refusal, canAct: canAct, onStart: {}))
    }

    private static func specimens() -> [Specimen] {
        let refused = RunnerPeople.repNotStarted
        return [
            Specimen(name: "column-none", view: column(.none, people: short)),
            Specimen(name: "column-none-long-account", view: column(.none, people: long)),
            Specimen(name: "column-starting", view: column(.starting, people: long)),
            Specimen(name: "column-stopped", view: column(.stopped, people: short)),
            Specimen(name: "column-refused", view: column(.none, people: long, refusal: refused)),
            Specimen(name: "column-looking", view: column(.none, people: short, canAct: false)),
            Specimen(name: "column-unknown", view: column(.unknown, people: short)),
            Specimen(name: "column-neither", view: column(.neither, people: short)),
            Specimen(
                name: "row-none",
                view: AnyView(
                    OrchestratorRowView(
                        model: NavigatorOrchestrator(
                            state: .none, title: RunnerPeople.repTitle, noneTitle: RunnerPeople.repTitle, onStartRep: {}),
                        inProgress: 0, selected: false, keyed: false
                    ).padding(.horizontal, NavigatorGrid.edge))),
            Specimen(
                name: "row-running-long",
                view: AnyView(
                    OrchestratorRowView(
                        model: NavigatorOrchestrator(
                            state: .needsYou, agent: "claude", status: .blocked,
                            nowDoing: "Waiting on you: should the Enterprise Platform Team’s billing and collections export keep its legacy columns?",
                            title: RunnerPeople.repTitle, noneTitle: RunnerPeople.repTitle),
                        inProgress: 3, selected: true, keyed: true
                    ).padding(.horizontal, NavigatorGrid.edge))),
            Specimen(
                name: "row-running",
                view: AnyView(
                    OrchestratorRowView(
                        model: NavigatorOrchestrator(
                            state: .working, agent: "claude", status: .working, nowDoing: "Reading the board",
                            title: RunnerPeople.repTitle, noneTitle: RunnerPeople.repTitle),
                        inProgress: 0, selected: true, keyed: false
                    ).padding(.horizontal, NavigatorGrid.edge))),
        ]
    }

    @Test("Write the rep column sheets")
    func writeSheets() throws {
        let directory = URL(
            fileURLWithPath: ProcessInfo.processInfo.environment["FARCOOLER_GLANCE_OUT"]
                ?? FileManager.default.currentDirectoryPath + "/.build/glance")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        for specimen in Self.specimens() {
            let isRow = specimen.name.hasPrefix("row")
            // The navigator is 240 pt at its narrowest; the conversation column
            // 340 narrow, and wide where it is the main area.
            for (label, width, height): (String, CGFloat, CGFloat) in isRow
                ? [("narrow", 240, 60), ("wide", 420, 60)] : [("narrow", 340, 460), ("wide", 720, 460)]
            {
                for dark in [false, true] {
                    let view = specimen.view
                        .frame(width: width, height: height)
                        .background(dark ? Color(white: 0.12) : Color.white)
                    let host = NSHostingView(rootView: view)
                    host.appearance = NSAppearance(named: dark ? .darkAqua : .aqua)
                    host.frame = CGRect(origin: .zero, size: CGSize(width: width, height: height))
                    host.layoutSubtreeIfNeeded()
                    let rep = try #require(host.bitmapImageRepForCachingDisplay(in: host.bounds))
                    host.cacheDisplay(in: host.bounds, to: rep)
                    let png = try #require(rep.representation(using: .png, properties: [:]))
                    try png.write(
                        to: directory.appendingPathComponent("rep-\(specimen.name)-\(label)-\(dark ? "dark" : "light").png"))
                }
            }
        }
    }
}
