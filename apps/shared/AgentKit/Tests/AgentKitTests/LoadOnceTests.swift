import Testing

@testable import AgentKit

/// The devices screen's loads (ov-828): a `.task` on its `Group` landed on each
/// section, so every section the load swapped in started another load and the
/// owner's phone sent 128 cancelled `POST /v1/account`s in two seconds.
@MainActor
struct LoadOnceTests {
    @Test func everySectionAppearingStartsOneLoad() async {
        let loads = LoadOnce()
        var ran = 0
        for _ in 0..<5 { loads.start(for: true) { ran += 1 } }
        await loads.current?.value
        #expect(ran == 1)
    }

    @Test func signingInOrOutLoadsOnceMore() async {
        let loads = LoadOnce()
        var ran: [Bool] = []
        loads.start(for: true) { ran.append(true) }
        await loads.current?.value
        for _ in 0..<4 { loads.start(for: false) { ran.append(false) } }
        await loads.current?.value
        for _ in 0..<4 { loads.start(for: true) { ran.append(true) } }
        await loads.current?.value
        #expect(ran == [true, false, true])
    }

    /// A section swapped out mid-load doesn't cancel it: the load is its own task.
    @Test func aLoadOutlivesTheSectionThatAsked() async {
        let loads = LoadOnce()
        var finished = false
        let section = Task { @MainActor in
            loads.start(for: true) {
                try? await Task.sleep(for: .milliseconds(50))
                finished = !Task.isCancelled
            }
        }
        await section.value
        section.cancel()
        await loads.current?.value
        #expect(finished)
    }
}
