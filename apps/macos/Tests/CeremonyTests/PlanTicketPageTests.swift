import AgentKit
import AppKit
import SwiftUI
import TestWindows
import Testing

@testable import Far_Cooler

/// A ticket opened from the plan is one scrolling page (ov-873): the header
/// with its title whole, who is working it and its branch, then Overview's
/// intent, acceptance and record. Hiding the tab strip must not hide these.
@MainActor
@Suite(.serialized)
struct PlanTicketPageTests {
    static let title = "The Android app no longer crashes on a cold start after the Play update"

    static func row() -> TaskRow {
        TaskRow(
            id: "ov-880", key: "ov-880", title: title, status: .inProgress, statusSince: Date(),
            intent: "A cold start after the Play update crashes before the first frame.",
            acceptance: [TaskAcceptanceLine(id: "a1", text: "A cold start reaches the home screen", met: false)])
    }

    static func page(_ work: PlanTicketWork, card: Bool = true) -> some View {
        PlanTicketPage(
            row: row(), agent: nil, work: work, line: { EmptyView() },
            card: {
                if card {
                    TaskCard(row: row(), detail: .empty, question: nil, canAnswer: true, onAnswer: { _ in true })
                }
            })
    }

    static let work = PlanTicketWork(
        agent: "Builder 1", doing: "Running the Android tests", branch: "builder-1/android-cold-start",
        openChat: {}, openTerminal: {}, openChanges: {})

    @Test("A plan-opened ticket shows its intent and acceptance, under its agent and branch")
    func showsItsContent() async throws {
        let drawn = await PlanThemePageTests.frames(Self.page(Self.work).frame(width: 640, height: 1400), width: 640, height: 1400)
        let intent = try #require(drawn["task-intent-section"], "intent drawn: \(drawn.keys.sorted())")
        let acceptance = try #require(drawn["task-acceptance-section"], "acceptance drawn: \(drawn.keys.sorted())")
        let agent = try #require(drawn["plan-ticket-agent"])
        let branch = try #require(drawn["plan-ticket-branch"])
        #expect(intent.height > 0 && acceptance.height > 0)
        #expect(agent.maxY <= branch.minY + 0.5 && branch.maxY <= intent.minY + 0.5, "agent, branch, then the Overview")
        #expect(intent.maxY <= acceptance.minY + 0.5)
    }

    @Test("The page is red when the Overview is hidden")
    func redWhenHidden() async {
        let drawn = await PlanThemePageTests.frames(
            Self.page(Self.work, card: false).frame(width: 640, height: 1400), width: 640, height: 1400)
        #expect(drawn["task-intent-section"] == nil && drawn["task-acceptance-section"] == nil)
    }

    @Test("A long title wraps whole rather than truncating")
    func titleWraps() async throws {
        let one = await PlanThemePageTests.frames(
            TaskViewHeader(row: Self.row(), wrapsTitle: false).probed("h").frame(width: 420), width: 420, height: 400)
        let wrapped = await PlanThemePageTests.frames(
            TaskViewHeader(row: Self.row(), wrapsTitle: true).probed("h").frame(width: 420), width: 420, height: 400)
        let truncated = try? #require(one["h"]), whole = try? #require(wrapped["h"])
        #expect((whole?.height ?? 0) > (truncated?.height ?? 1000) + 10, "\(String(describing: whole)) vs \(String(describing: truncated))")
    }

    @Test("Who is working it: the card's own words when opened under it, else the pane's")
    func who() {
        let origin = PlanStack.Origin(task: "ov-880", agent: "Builder 1", doing: "Running the tests", target: nil)
        let under = PlanTicketWork.who(origin: origin, task: "ov-880", pane: nil)
        #expect(under?.name == "Builder 1" && under?.doing == "Running the tests")
        #expect(PlanTicketWork.who(origin: origin, task: "ov-881", pane: nil) == nil, "another ticket's agent isn't this one's")
    }
}
