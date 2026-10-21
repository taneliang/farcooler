import AgentKit
import Foundation
import Testing

@testable import Far_Cooler

/// The conversation composer's draft survives the pane's model going away
/// (ov-369 F4, R-38): app quit, jetsam and process death all end as "a new
/// model for the same terminal". It is cleared when the runner confirms the
/// send, not when Send is pressed.
@MainActor
@Suite(.serialized)
struct NativeDraftSurvivalTests {
    typealias Sink = NativeAgentTests.StandInSink

    private func defaults() -> UserDefaults {
        ScratchDefaults.suite("native-draft-survival-\(UUID().uuidString)")!
    }

    private func model(_ defaults: UserDefaults, sink: Sink? = nil) throws -> NativePaneModel {
        let terminal = try NativeAgentTests.terminal()
        NativePaneModel.remember(false, for: terminal.id)
        return NativePaneModel(
            terminal: terminal.id, store: AgentRowStore(key: "test-\(UUID())", cache: nil), sink: sink,
            draftKeeper: DraftKeeper(terminal: terminal.id, defaults: defaults, delay: .milliseconds(1)))
    }

    @Test("A draft survives the model being made again")
    func aDraftSurvivesTheModelBeingRecreated() async throws {
        let store = defaults()
        let first = try model(store)
        first.draft = "fix the login, then the tests"
        await first.draftKeeper?.settled()
        let second = try model(store)
        #expect(second.draft == "fix the login, then the tests")
    }

    @Test("A send the runner confirmed clears the saved draft")
    func aConfirmedSendClearsIt() async throws {
        let store = defaults()
        let sink = Sink()
        await sink.set(.success(false))
        let first = try model(store, sink: sink)
        first.draft = "ship it"
        await first.draftKeeper?.settled()
        await first.send()
        #expect(await sink.sent == ["ship it"])
        let second = try model(store)
        #expect(second.draft.isEmpty)
    }

    @Test("A send the runner didn't confirm keeps the saved draft")
    func anUnconfirmedSendKeepsIt() async throws {
        let store = defaults()
        let sink = Sink()
        await sink.set(.failure(.refused("no", word: "resource-conflict", what: "draft")))
        let first = try model(store, sink: sink)
        first.draft = "ship it"
        await first.draftKeeper?.settled()
        await first.send()
        #expect(first.issue != nil && first.draft == "ship it")
        let second = try model(store)
        #expect(second.draft == "ship it")
    }

    // MARK: Photos (ov-622)

    private func scratchImages() throws -> DraftImages {
        DraftImages(
            files: PaneImageFiles(
                root: try ScratchDirectory.make("fc-draft-images"), paneCap: 1_000_000, totalCap: 10_000_000, whenFull: .refuse, maxAge: 3600))
    }

    private func photoModel(_ defaults: UserDefaults, _ images: DraftImages, sink: Sink? = nil) throws -> NativePaneModel {
        let terminal = try NativeAgentTests.terminal()
        NativePaneModel.remember(false, for: terminal.id)
        let model = NativePaneModel(
            terminal: terminal.id, store: AgentRowStore(key: "test-\(UUID())", cache: nil), sink: sink,
            draftKeeper: DraftKeeper(terminal: terminal.id, defaults: defaults, delay: .milliseconds(1)), draftImages: images)
        model.rich = true
        return model
    }

    private func settle(_ model: NativePaneModel, images count: Int) async -> Bool {
        for _ in 0..<200 where model.images.count != count { try? await Task.sleep(for: .milliseconds(10)) }
        return model.images.count == count
    }

    @Test("A draft's photos survive the model being made again, and go with the message")
    func aDraftsPhotosSurvive() async throws {
        let store = defaults(), images = try scratchImages()
        let sink = Sink()
        await sink.set(.success(false))
        let first = try photoModel(store, images, sink: sink)
        let photo = ComposeImage(mime: "image/png", data: NativeComposerTests.png())
        first.attach([photo])
        first.draft = "what is this"
        await first.draftKeeper?.settled()
        images.flush()
        let second = try photoModel(store, images, sink: sink)
        #expect(await settle(second, images: 1), "the photo didn't come back")
        #expect(second.images.first?.id == photo.id && second.images.first?.data == photo.data)
        #expect(second.draft == "what is this")
        await second.send()
        #expect(await sink.sent == ["what is this"])
        images.flush()
        let third = try photoModel(store, images)
        #expect(!(await settle(third, images: 1)), "a sent photo came back")
    }

    @Test("A photo taken out of the draft doesn't come back")
    func aRemovedPhotoDoesNotComeBack() async throws {
        let store = defaults(), images = try scratchImages()
        let first = try photoModel(store, images)
        let photo = ComposeImage(mime: "image/png", data: NativeComposerTests.png())
        first.attach([photo])
        first.detach(photo.id)
        images.flush()
        #expect(!(await settle(try photoModel(store, images), images: 1)))
    }

    @Test("Without compose the photos wait on disk for a runner that takes them")
    func photosWaitForARunnerThatTakesThem() async throws {
        let store = defaults(), images = try scratchImages()
        let first = try photoModel(store, images)
        first.attach([ComposeImage(mime: "image/png", data: NativeComposerTests.png())])
        images.flush()
        first.rich = false
        #expect(first.images.isEmpty)
        first.rich = true
        #expect(await settle(first, images: 1), "the photo wasn't back once the runner took photos again")
    }

    @Test("Photos aren't restored into a composer that has sent since they were asked for")
    func photosAreNotRestoredAfterASend() async throws {
        let store = defaults(), images = try scratchImages()
        let first = try photoModel(store, images)
        first.attach([ComposeImage(mime: "image/png", data: NativeComposerTests.png())])
        images.flush()
        // The restore is asked for as the model takes photos (`rich`), and
        // reads on its own queue; a send lands before it is done.
        let second = try photoModel(store, images)
        second.sendCount += 1
        #expect(!(await settle(second, images: 1)), "a photo was restored into a composer that had sent")
    }
}
