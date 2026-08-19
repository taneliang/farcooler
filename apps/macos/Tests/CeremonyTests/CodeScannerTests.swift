import AppKit
import Foundation
import Testing

@testable import Far_Cooler

struct CodeScannerTests {
    @Test func aQueuedStopFinishesBeforeAnImmediateRestart() async {
        let queue = CaptureSessionQueue(label: "com.farcooler.scanner.tests")
        let probe = OverlapProbe()

        queue.enqueue { probe.run("stop") }
        await queue.perform { probe.run("start") }

        #expect(probe.maximumConcurrentCalls == 1)
        #expect(probe.completedCalls == ["stop", "start"])
    }

    /// The regression that crashed the app on every Mac that granted the
    /// camera: this end used to ask `AVCaptureMetadataOutput` for `.qr`, which
    /// reads codes on iOS and offers only faces, bodies and pets on macOS. The
    /// Mac reads its own frames now, so the check is that it can actually read
    /// one — a code this app generated, decoded by the thing that decodes
    /// camera frames.
    @Test func aCodeThisAppDrewIsReadBackOutOfItsOwnPicture() throws {
        let payload = "farcooler://offer/2f8ac41d?k=AAAAC3NzaC1lZDI1NTE5"
        let drawn = try #require(qrImage(payload))

        #expect(FrameReader.payload(in: try cgImage(of: drawn)) == payload)
    }

    /// The frame this reader sees the most of: the room, before anybody holds
    /// anything up to the camera.
    @Test func aPictureWithNoCodeInItReadsAsNothing() throws {
        let blank = NSImage(size: NSSize(width: 240, height: 240))
        blank.lockFocus()
        NSColor.white.setFill()
        NSRect(x: 0, y: 0, width: 240, height: 240).fill()
        blank.unlockFocus()

        #expect(FrameReader.payload(in: try cgImage(of: blank)) == nil)
    }

    /// One code per scan, and one more after "Try Again".
    ///
    /// The claim is taken on the frame queue rather than by checking the
    /// published value on the main actor, because frames arrive faster than the
    /// sheet reacts and several would pass that check at once.
    @Test func onlyTheFirstCodeIsReportedUntilTheReaderIsResumed() {
        let reader = FrameReader { _, _ in }

        #expect(reader.claim())
        #expect(!reader.claim())
        #expect(!reader.claim())

        reader.resume()

        #expect(reader.claim())
        #expect(!reader.claim())
    }

    private func cgImage(of image: NSImage) throws -> CGImage {
        var rect = CGRect(origin: .zero, size: image.size)
        return try #require(image.cgImage(forProposedRect: &rect, context: nil, hints: nil))
    }
}

private final class OverlapProbe: @unchecked Sendable {
    private let lock = NSLock()
    private var activeCalls = 0
    private(set) var maximumConcurrentCalls = 0
    private(set) var completedCalls: [String] = []

    func run(_ name: String) {
        lock.withLock {
            activeCalls += 1
            maximumConcurrentCalls = max(maximumConcurrentCalls, activeCalls)
        }
        Thread.sleep(forTimeInterval: 0.03)
        lock.withLock {
            activeCalls -= 1
            completedCalls.append(name)
        }
    }
}
