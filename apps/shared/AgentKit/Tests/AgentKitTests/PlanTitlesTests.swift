import Foundation
import Testing

@testable import AgentKit

/// Lanes and trains named by what they do (ov-462), and a train's own
/// integrating agent (ov-461), as the apps read them from the plan.
struct PlanTitlesTests {
    static func lane(_ name: String, title: String? = nil, state: String = "building") -> [String: Any] {
        var lane = PlanModelTests.lane(name, state, cards: [])
        if let title { lane["title"] = title }
        return lane
    }

    @Test("a lane's title comes first and its slug second; with none, the name stands alone")
    func aLaneHeadsWithItsTitle() throws {
        let plan = try PlanModel.decode(
            JSONSerialization.data(
                withJSONObject: [
                    "now_ms": PlanModelTests.now, "themes": [], "order": [], "cards": [],
                    "lanes": [Self.lane("agent-msg", title: "Agents message the orchestrator"), Self.lane("attach"), Self.lane("same", title: "")],
                ] as [String: Any]))
        let (titled, bare, empty) = (plan.lanes[0], plan.lanes[1], plan.lanes[2])
        #expect((titled.heading, titled.slug) == ("Agents message the orchestrator", "agent-msg"))
        #expect((bare.heading, bare.slug) == ("attach", nil))
        #expect((empty.heading, empty.slug) == ("same", nil), "an empty title is none")
    }

    @Test("the fixture: titles on the lanes, and Train 9 with its summary, agent and card")
    func theFixtureCarriesTitles() throws {
        let plan = try PlanModelTests.fixture()
        #expect(plan.lanes.map(\.heading) == ["Mac follow-ups", "Mac interface polish", "fix-ac84"])
        #expect(plan.lanes[1].slug == "mac-ux" && plan.lanes[2].slug == nil)
        let train = try #require(plan.trains.first)
        #expect((train.heading, train.slug, train.carries) == ("Train 9", "integ-9", "Mac interface polish"))
        #expect(train.card?.key == "ov-2")
        let agent = try #require(train.agent)
        #expect(agent.isWorking && agent.agentId == "i1" && agent.spend.totalTokens == 120_000)
        #expect(PlanWords.trainAgent(agent).hasPrefix("Agent working · 120K tokens"))
        #expect(PlanWords.train(train, ci: plan.ci(of: train)).contains(" · Agent working · 120K tokens"))
        #expect(plan.laneIsTrain.isEmpty)
    }

    @Test("a train reads Train N from integ-N or train-N, its own title when it has one, and its name otherwise")
    func trainNumbers() throws {
        #expect(PlanWords.trainNumber("integ-72") == 72 && PlanWords.trainNumber("Train-5") == 5)
        #expect(PlanWords.trainNumber("integ-x") == nil && PlanWords.trainNumber("rc-1") == nil && PlanWords.trainNumber("train-") == nil)
        let plan = try PlanTrainsTests.plan(
            trains: [
                PlanTrainsTests.train("integ-72", "gating", lanes: []),
                PlanTrainsTests.train("spring", "gating", lanes: []),
                PlanTrainsTests.train("train-3", "gating", lanes: []).merging(["title": "Phones catch up"]) { $1 },
            ], lanes: [])
        #expect(plan.trains.map(\.heading) == ["Train 72", "spring", "Phones catch up"], "an older runner's answer still reads")
        #expect(plan.trains.map(\.slug) == ["integ-72", nil, "train-3"])
        #expect(plan.trains.allSatisfy { $0.carries == nil && $0.agent == nil })
    }

    @Test("a lane named like a live train is drawn by the train once, and listed under Worth a look")
    func theShadowLane() throws {
        var object: [String: Any] = [
            "now_ms": PlanModelTests.now, "themes": [], "order": [], "cards": [],
            "lanes": [Self.lane("integ-2"), Self.lane("mlx-ram")],
            "trains": [PlanTrainsTests.train("integ-2", "integrating", lanes: ["mlx-ram"])],
            "lane_is_train": [["lane": "lane-integ-2", "name": "integ-2"]],
        ]
        var plan = try PlanModel.decode(JSONSerialization.data(withJSONObject: object))
        #expect(plan.nowGroups.map { $0.train?.name } == ["integ-2"])
        #expect(plan.nowGroups[0].lanes.map(\.name) == ["mlx-ram"], "integ-2 is the train, not also a lane")
        let outside = plan.outsideThemes(statuses: [:])
        #expect(outside.shadows.map(\.name) == ["integ-2"] && !outside.isEmpty)
        #expect(PlanWords.tidy(outside) == "1 lane to tidy")
        #expect(PlanWords.shadow(outside.shadows[0]) == "integ-2 is a lane and a train")
        // Without the runner's flag nothing is hidden.
        object["lane_is_train"] = nil
        plan = try PlanModel.decode(JSONSerialization.data(withJSONObject: object))
        #expect(plan.nowGroups.flatMap(\.lanes).map(\.name).contains("integ-2"))
    }

    @Test("tidy words count cards and lanes")
    func tidyWords() {
        let card = PlanFlaggedCard(task: "t", key: "ov-1", status: "in_review")
        let lane = PlanFlaggedLane(lane: "l", name: "integ-2")
        #expect(PlanWords.tidy(PlanOutside(lanes: [], openCards: 0, tidy: [card, card], shadows: [])) == "2 cards to tidy")
        #expect(PlanWords.tidy(PlanOutside(lanes: [], openCards: 0, tidy: [card], shadows: [lane])) == "1 card and 1 lane to tidy")
        #expect(PlanWords.tidy(PlanOutside(lanes: [], openCards: 0, tidy: [], shadows: [lane, lane])) == "2 lanes to tidy")
    }

    @Test("a task's line uses the lane's title; the strip, a status line of slugs, keeps the names")
    func titlesReachTheLineButNotTheStrip() throws {
        let plan = try PlanModelTests.fixture()
        let line = try #require(plan.taskLine("00000000-0000-0000-0000-000000001001"))
        #expect(line.laneWords == "In lane Mac interface polish")
        #expect(PlanStrip(plan: plan, needsYou: 0, orchestrator: .working).now.map(\.name) == ["mac-ux"])
    }
}
