import CoreGraphics
import Foundation
import ImageIO
import Testing
import UniformTypeIdentifiers

@testable import AgentKit

// ov-404: what a phone's composer decides by where the runner has `compose`:
// the images it sends, the words a refusal gets, and when Stop is offered.
// The views are held by the iOS UI tests (`NativeComposerTests`) and the
// Android ones.

@Suite struct ComposeRichTests {
    // MARK: Images

    /// An opaque picture of noise, written as `type`: noise doesn't compress,
    /// so the file's size follows from the pixels.
    private func picture(_ type: UTType, width: Int, height: Int, alpha: Bool = false, quality: Double = 0.95) throws -> Data {
        let info = alpha ? CGImageAlphaInfo.premultipliedLast : CGImageAlphaInfo.noneSkipLast
        let context = try #require(
            CGContext(
                data: nil, width: width, height: height, bitsPerComponent: 8, bytesPerRow: 0,
                space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: info.rawValue))
        let pixels = try #require(context.data).assumingMemoryBound(to: UInt8.self)
        var seed: UInt64 = 0x9E37_79B9_7F4A_7C15
        for i in 0..<(context.bytesPerRow * height) {
            seed = seed &* 6_364_136_223_846_793_005 &+ 1_442_695_040_888_963_407
            pixels[i] = UInt8(truncatingIfNeeded: seed >> 33)
        }
        let image = try #require(context.makeImage())
        let out = NSMutableData()
        let destination = try #require(CGImageDestinationCreateWithData(out, type.identifier as CFString, 1, nil))
        CGImageDestinationAddImage(
            destination, image, [kCGImageDestinationLossyCompressionQuality: quality] as CFDictionary)
        try #require(CGImageDestinationFinalize(destination))
        return out as Data
    }

    private func size(of data: Data) throws -> (width: Int, height: Int) {
        let source = try #require(CGImageSourceCreateWithData(data as CFData, nil))
        let properties = try #require(CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any])
        return (properties[kCGImagePropertyPixelWidth] as? Int ?? 0, properties[kCGImagePropertyPixelHeight] as? Int ?? 0)
    }

    /// ov-393's iPhone half: a 10 MB photo goes to the runner as it is. A
    /// JPEG the runner reads and that fits its 16 MB is never re-encoded.
    @Test func aTenMegabytePhotoIsSentAsItIs() throws {
        let photo = try picture(.jpeg, width: 3_500, height: 2_500, quality: 0.98)
        #expect(photo.count > 10 * 1024 * 1024, "the fixture is the 10 MB photo: \(photo.count)")
        #expect(photo.count < OutgoingImage.largestKept)
        let image = try #require(OutgoingImage.make(photo))
        #expect(image.mime == "image/jpeg")
        #expect(image.data == photo, "the bytes weren't touched")
    }

    @Test func aPhotoInAnotherFormatIsAJpegNoLongerThanTheLimit() throws {
        // A 12 MP HEIC, as a phone's camera writes. On a Mac without a HEVC
        // encoder the destination is nil; the TIFF below says the same thing.
        let heic = try picture(.heic, width: 4_032, height: 3_024)
        let image = try #require(OutgoingImage.make(heic))
        #expect(image.mime == "image/jpeg", "an opaque photo isn't a PNG, which would be several times larger")
        let edge = try size(of: image.data)
        #expect(max(edge.width, edge.height) == OutgoingImage.longestEdge)
        #expect(image.data.count < OutgoingImage.largestKept)
    }

    @Test func transparencyKeepsItsAlphaAsAPng() throws {
        let tiff = try picture(.tiff, width: 400, height: 300, alpha: true)
        let image = try #require(OutgoingImage.make(tiff))
        #expect(image.mime == "image/png")
    }

    @Test func aKeptFormatPastTheLimitIsConvertedNotRefused() throws {
        // A noisy PNG a little over 16 MB: the runner would refuse it as
        // `image_too_large`, so it's made smaller here, as a JPEG when opaque.
        let big = try picture(.png, width: 4_300, height: 1_300)
        try #require(big.count > OutgoingImage.largestKept, "the fixture is over the limit: \(big.count)")
        let image = try #require(OutgoingImage.make(big))
        #expect(image.mime == "image/jpeg")
        #expect(image.data.count < OutgoingImage.largestKept)
    }

    @Test func somethingThatIsNotAnImageIsNil() {
        #expect(OutgoingImage.make(Data("not a picture".utf8)) == nil)
        #expect(OutgoingImage.make(Data()) == nil)
    }

    @Test func aChipIsDrawnSmall() throws {
        let image = try #require(OutgoingImage.make(try picture(.png, width: 800, height: 600)))
        let thumbnail = try #require(image.thumbnail())
        #expect(max(thumbnail.width, thumbnail.height) <= 96)
    }

    // MARK: Words

    @Test func aRefusalOfAnImageSaysWhichLimitItHit() {
        #expect(AgentConversation.issue(for: .refused(what: "image_too_large")) == .said(AgentConversation.imageTooLarge))
        #expect(AgentConversation.issue(for: .refused(what: "images_too_large")) == .said(AgentConversation.imagesTooLarge))
        #expect(AgentConversation.issue(for: .refused(what: "images")) == .said(AgentConversation.tooManyImages))
        #expect(
            AgentConversation.issue(for: .refused(what: "images"), command: true)
                == .said(AgentConversation.commandWithImages), "a slash command can't carry any, however few")
        #expect(AgentConversation.imageTooLarge != AgentConversation.imagesTooLarge)
        #expect(AgentConversation.issue(for: .refused(what: "backslash")) == .said(AgentConversation.backslash))
        #expect(AgentConversation.issue(for: .refused(what: "handoff")) == .panel)
        #expect(AgentConversation.issue(for: .refused(what: "command")) == .said(AgentConversation.commandRefused))
        #expect(AgentConversation.issue(for: .refused(what: "unconfirmed")) == .said(AgentConversation.unconfirmed))
    }

    @Test func theLimitsFollowWhetherTheRunnerHasCompose() {
        #expect(AgentConversation.longest(rich: false) == 500)
        #expect(AgentConversation.longest(rich: true) == 100_000)
        #expect(AgentConversation.tooLong(rich: true).contains("100,000"))
        #expect(AgentConversation.tooLong(rich: false).contains("500"))
    }

    @Test func noCopyOnTheWayOutIsRawOrBritish() {
        let words = [
            AgentConversation.tooLongComposed, AgentConversation.commandRefused, AgentConversation.imagesTooLarge,
            AgentConversation.tooManyImages, AgentConversation.commandWithImages, AgentConversation.imageTooLarge,
            AgentConversation.backslash, AgentConversation.unconfirmed, AgentConversation.panel,
        ]
        for line in words {
            #expect(line.hasSuffix("."), "\(line)")
            #expect(!line.contains("Err"), "\(line)")
            #expect(!line.contains("colour") && !line.contains("authoris"), "\(line)")
        }
    }

    // MARK: A message with images, queued

    @Test func anEchoOfImagesSettlesAgainstTheRowClaudeWrote() {
        #expect(AgentConversation.echo("look", images: 2) == "[Image] [Image] look")
        #expect(AgentConversation.echo("  ", images: 1) == "[Image]")
        let row = { (text: String) in
            AgentRow(id: "q", ord: 1, rev: 1, kind: .queued(.init(text: text, state: "Waiting", atMs: nil)))
        }
        // Claude numbers its placeholders; the echo doesn't. Same count, same words: settled.
        #expect(AgentConversation.unsettled(["[Image] [Image] look"], newest: [row("[Image #1] [Image #2] look")]) == [])
        // One image isn't two.
        #expect(AgentConversation.unsettled(["[Image] [Image] look"], newest: [row("[Image #1] look")]).count == 1)
        // Words are words.
        #expect(AgentConversation.unsettled(["look"], newest: [row("look elsewhere")]).count == 1)
    }

    // MARK: Stop and Send Now

    private func turn(outcome: AgentRow.Turn.Outcome? = nil, activity: String?) -> AgentRow.Turn {
        AgentRow.Turn(
            prompt: "p", origin: "Typed", startedMs: nil, endedMs: nil, durationMs: nil, outcome: outcome,
            backgroundRunning: 0, activity: activity)
    }

    /// Stop is for a turn claude is working on; under a dialog its Esc would
    /// answer No, so it isn't offered (review of ov-368).
    @Test func stopIsOfferedOnlyWhileClaudeWorks() {
        #expect(AgentConversation.isWorking(newestTurn: turn(activity: "Busy")))
        #expect(!AgentConversation.isWorking(newestTurn: turn(activity: "Waiting")), "hidden under a dialog")
        #expect(!AgentConversation.isWorking(newestTurn: turn(activity: "Idle")))
        #expect(!AgentConversation.isWorking(newestTurn: turn(activity: "Shell")))
        #expect(!AgentConversation.isWorking(newestTurn: turn(outcome: .finished, activity: "Busy")), "a turn that ended")
        #expect(!AgentConversation.isWorking(newestTurn: nil))
    }

    @Test func theNewestTurnIsTheLastOne() {
        let rows = [
            AgentRow(id: "a", ord: 1, rev: 1, kind: .turn(turn(outcome: .finished, activity: nil))),
            AgentRow(id: "b", ord: 2, rev: 1, kind: .notice(.init(kind: "Compacted", text: "x"))),
            AgentRow(id: "c", ord: 3, rev: 1, kind: .turn(turn(activity: "Busy"))),
            AgentRow(id: "d", ord: 4, rev: 1, kind: .notice(.init(kind: "Compacted", text: "y"))),
        ]
        #expect(AgentConversation.newestTurn(in: rows)?.activity == "Busy")
        #expect(AgentConversation.newestTurn(in: []) == nil)
    }

    @Test func aKeyThatDidNothingNeededSaysNothing() {
        #expect(AgentConversation.keyIssue(for: .refused(what: "idle"), .stop) == nil)
        #expect(AgentConversation.keyIssue(for: .refused(what: "too_soon"), .sendNow) == nil)
        #expect(AgentConversation.keyIssue(for: .refused(what: "prompt"), .stop) == .handoff)
        #expect(AgentConversation.keyIssue(for: .refused(what: "draft"), .sendNow) == .draftInTerminal)
        #expect(
            AgentConversation.keyIssue(for: .refused(what: "nothing_queued"), .sendNow)
                == .said("Nothing is waiting in Claude’s queue."))
    }

    @Test func eachKeySaysItsOwnNameWhenClaudeIsSettling() {
        #expect(
            AgentConversation.keyIssue(for: .refused(what: "settling"), .stop)
                == .said("Claude is starting a step. Try Stop again in a moment."))
        #expect(
            AgentConversation.keyIssue(for: .refused(what: "settling"), .sendNow)
                == .said("Claude is starting a step. Try Send Now again in a moment."))
    }

    @Test func aKeyThatMayHaveLandedNeverSaysItDidnt() {
        let stopped = AgentConversation.keyIssue(for: .refused(what: "unconfirmed"), .stop)
        #expect(stopped == .said("Claude didn’t confirm it stopped. It may have stopped; check the terminal before pressing again."))
        #expect(AgentConversation.keyIssue(for: .timedOut, .stop) == .said("The runner didn’t answer in time. Check the terminal."))
        #expect(AgentConversation.keyIssue(for: .lost(notSent: false), .sendNow) == .said("The runner didn’t answer in time. Check the terminal."))
        #expect(AgentConversation.keyIssue(for: .lost(notSent: true), .stop) == .said("The runner isn’t connected. Use the terminal."))
        #expect(
            AgentConversation.keyIssue(for: .refused(what: "mystery"), .stop)
                == .said("Far Cooler can’t stop Claude safely from here. Use the terminal."))
        #expect(
            AgentConversation.keyIssue(for: .refused(what: "mystery"), .sendNow)
                == .said("Far Cooler can’t send the queue safely from here. Use the terminal."))
    }
}
