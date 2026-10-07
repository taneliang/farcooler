import AgentKit
import AppKit
import ImageIO
import SwiftUI
import Testing
import UniformTypeIdentifiers

@testable import Far_Cooler

/// The native composer's review-1 fixes (ov-400): photos converted small
/// enough to send, the capability that makes a box rich, the images a lost
/// capability drops, the Queued echo's image count, a browser's Copy Image,
/// and the sentences for one image too large and for images on a command.
@MainActor
@Suite(.serialized)
struct NativeComposerFixTests {
    /// `width`×`height` of noise, as a camera's photo compresses worst,
    /// written as `type`; `alpha` with a transparent channel.
    static func noise(_ width: Int, _ height: Int, as type: UTType, alpha: Bool = false) throws -> Data {
        var pixels = [UInt8](repeating: 0, count: width * height * 4)
        arc4random_buf(&pixels, pixels.count)
        let info = alpha ? CGImageAlphaInfo.premultipliedLast.rawValue : CGImageAlphaInfo.noneSkipLast.rawValue
        let context = try #require(
            CGContext(data: &pixels, width: width, height: height, bitsPerComponent: 8, bytesPerRow: width * 4,
                      space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: info))
        let image = try #require(context.makeImage())
        let out = NSMutableData()
        let destination = try #require(CGImageDestinationCreateWithData(out, type.identifier as CFString, 1, nil))
        CGImageDestinationAddImage(destination, image, nil)
        #expect(CGImageDestinationFinalize(destination))
        return out as Data
    }

    static func longEdge(_ data: Data) -> Int {
        guard let source = CGImageSourceCreateWithData(data as CFData, nil),
            let p = CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any]
        else { return 0 }
        return max(p[kCGImagePropertyPixelWidth] as? Int ?? 0, p[kCGImagePropertyPixelHeight] as? Int ?? 0)
    }

    @Test("A large HEIC photo becomes a JPEG under 16 MB, its long edge 2576")
    func aLargeHEICFits() throws {
        let heic = try Self.noise(6000, 4000, as: .heic)
        let image = try #require(ComposeImage.make(heic, type: .heic))
        #expect(image.mime == "image/jpeg")
        #expect(image.data.count < 16 * 1024 * 1024, "\(image.data.count) bytes")
        #expect(Self.longEdge(image.data) == ComposeImage.longestEdge)
    }

    @Test("A transparent TIFF stays a PNG; an opaque one becomes a JPEG; a small PNG is kept")
    func transparencyKeepsPNG() throws {
        let clear = try #require(ComposeImage.make(try Self.noise(300, 200, as: .tiff, alpha: true), type: .tiff))
        #expect(clear.mime == "image/png" && Self.longEdge(clear.data) == 300, "never scaled up")
        let opaque = try #require(ComposeImage.make(try Self.noise(300, 200, as: .tiff), type: .tiff))
        #expect(opaque.mime == "image/jpeg")
        let png = try Self.noise(64, 64, as: .png, alpha: true)
        #expect(ComposeImage.make(png, type: .png)?.data == png)
    }

    @Test("A browser's Copy Image, a web address beside the picture, pastes the picture")
    func copyImageWithItsAddress() {
        let png = NativeComposerTests.png()
        let board = NSPasteboard(name: NSPasteboard.Name("fc-composer-fix-\(UUID().uuidString)"))
        defer { board.releaseGlobally() }
        board.clearContents()
        board.setString("https://example.com/shot.png", forType: .string)
        board.setData(png, forType: .png)
        #expect(ComposeImage.from(board).count == 1 && ComposeImage.offered(on: board))
        board.clearContents()
        board.setString("a cell's words", forType: .string)
        board.setData(png, forType: .png)
        #expect(ComposeImage.from(board).isEmpty && !ComposeImage.offered(on: board), "words beside a picture paste as words")
    }

    @Test("Only a runner offering compose gets a rich box")
    func richFollowsTheOffer() throws {
        for (offered, rich) in [(Set(["agent_rows", "agent_compose"]), false), (Set(["agent_rows", "agent_compose", "compose"]), true)] {
            let agents = NativeAgents(defaults: UserDefaults(suiteName: "native-fix-\(UUID().uuidString)")!)
            agents.pretend(enabled: true, rowsServed: true, core: RunnerCore(), offered: offered)
            let model = agents.model(for: "0199aaaa-0000-7000-8000-00000000000\(rich ? 5 : 4)")
            #expect(model.rich == rich, "\(offered)")
            model.draft = "one\ntwo"
            #expect(model.draft == (rich ? "one\ntwo" : "one two"))
        }
    }

    @Test("Losing compose drops the waiting images and flattens the draft")
    func losingComposeDropsImages() throws {
        let model = NativeAgentTests.model(try NativeAgentTests.terminal())
        model.rich = true
        model.attach([try #require(ComposeImage.make(NativeComposerTests.png(), type: .png))])
        model.draft = "one\ntwo"
        #expect(model.images.count == 1)
        model.rich = false
        #expect(model.images.isEmpty && model.draft == "one two")
    }

    @Test("A Queued echo settles only against a row with as many images")
    func theEchoCountsImages() {
        #expect(NativePaneModel.words("[Image] [Image] fix it") == NativePaneModel.words("[Image #3] [Image #4] fix it"))
        #expect(NativePaneModel.words("[Image]") != NativePaneModel.words("[Image #1] [Image #2]"))
        #expect(NativePaneModel.words("[Image]") == NativePaneModel.words("[Image #9]"))
    }

    @Test("One image too large, and images on a command, each say so")
    func imageSentences() {
        let refused = { (what: String) in RunnerCore.Failure.refused("no", word: "resource-conflict", what: what) }
        #expect(NativePaneModel.issue(for: refused("image_too_large")) == .said(NativePaneModel.imageTooLarge))
        #expect(NativePaneModel.issue(for: refused("images"), command: true) == .said(NativePaneModel.commandWithImages))
        #expect(NativePaneModel.issue(for: refused("images")) == .said(NativePaneModel.tooManyImages))
        #expect(NativePaneModel.issue(for: refused("backslash")) == .said(NativePaneModel.backslash))
    }
}
