import Testing

@testable import AgentKit

private func runner(_ capabilities: Set<String>) -> DaemonBuild {
    DaemonBuild(version: "v", matches: true, platform: "linux", capabilities: capabilities)
}

struct QueueControlsTests {
    @Test func aRunnerWithoutTheCapabilityGetsTheSentence() {
        let gate = QueueControls.gate(runner(["agent", "tasks"]))
        #expect(gate == .unavailable(sentence: QueueControls.olderRunnerSentence))
        #expect(!gate.isAvailable)
    }

    @Test func aRunnerFromBeforeCapabilitiesIsGatedToo() {
        #expect(!QueueControls.gate(runner([])).isAvailable)
    }

    @Test func aRunnerWithTheCapabilityShowsTheControls() {
        #expect(QueueControls.gate(runner(["agent", "agent_queue"])).isAvailable)
    }

    @Test func aRunnerNotYetHeardFromIsNotGated() {
        #expect(QueueControls.gate(nil).isAvailable)
    }

    @Test func theSentenceReadsPlainly() {
        #expect(
            QueueControls.olderRunnerSentence
                == "This runner can’t change queued messages. Update it to edit or cancel them.")
    }

    @Test func aRefusalIsSaidAndKeepsItsStep() {
        let said = QueueControls.refusal(.cancel, word: "agent-stopped", message: "raw")
        #expect(said.hasPrefix("Couldn’t take that message back."))
        #expect(said.contains("The agent stopped."))
        #expect(!said.contains("raw"))
    }

    @Test func anUnreadableRefusalStillSaysTheStep() {
        #expect(
            QueueControls.refusal(.edit, word: "from-the-future", message: "raw")
                == "Couldn’t save that edit.")
    }
}
