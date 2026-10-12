import Testing

@testable import AgentKit

struct PlainLineTests {
    /// The orchestrator's real reply from the hero capture, as the runner sends
    /// it: one line, cut at 120 characters, opening with a list (ov-495).
    @Test func aReplyThatOpensWithAListLosesItsMarkup() {
        let said = "Four asks, in order: - **PDF export** (ha-12): queued behind ha-10. - **Charge retries**: building now in `charge-retry`."
        #expect(
            PlainLine.of(said)
                == "Four asks, in order: PDF export (ha-12): queued behind ha-10. Charge retries: building now in charge-retry.")
    }

    @Test func linesKeepOnlyTheFirstWords() {
        #expect(PlainLine.of("Four asks, in order:\n\n- **PDF export** (ha-12)\n- Charge retries") == "Four asks, in order:")
        #expect(PlainLine.of("\n- **PDF export** (ha-12)\n- more") == "PDF export (ha-12)")
        #expect(PlainLine.of("## Plan\nthen") == "Plan")
        #expect(PlainLine.of("1. Ship [the docs](https://x.example/a_b) _now_") == "Ship the docs now")
    }

    @Test func prosePunctuationSurvives() {
        #expect(PlainLine.of("Done - tests pass, 2 * 3 is 6, snake_case_name stays.") == "Done - tests pass, 2 * 3 is 6, snake_case_name stays.")
        #expect(PlainLine.of("") == "")
    }
}
