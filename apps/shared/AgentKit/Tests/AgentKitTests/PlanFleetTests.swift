import Foundation
import Testing

@testable import AgentKit

// What the runner says the agents spent (ov-518). The Agents rows and their
// header are the Rust core's (`PlanView.Agents`, ov-1016), pinned by its tests.
@Suite("The Plan's agents list")
struct PlanFleetTests {
    @Test("agents_spend decodes, and a runner without it leaves nil")
    func decodesTheTotal() throws {
        let base = """
            {"now_ms":1,"themes":[],"lines":[],"cards":[]
            """
        let with = base + #","agents_spend":{"input_tokens":7,"cost_micros":9}}"#
        #expect(try PlanModel.decode(Data(with.utf8)).agentsSpend?.totalTokens == 7)
        #expect(try PlanModel.decode(Data((base + "}").utf8)).agentsSpend == nil)
    }
}
