import AppKit
import Foundation

/// Render a byte stream to a PNG and exit.
///
///     overnight terminal stream <id> | OVERNIGHT_RENDER_PROBE=/tmp/out.png Overnight
///
/// Terminal rendering is the one part of this app that cannot be asserted in a
/// unit test — "the glyphs are upright, in the right cells, in the right
/// colours" is a claim about pixels. This makes those pixels inspectable
/// without a window, a human, or a screenshot, so a rendering regression can be
/// caught by looking at a file.
@MainActor
enum RenderProbe {
    static func run(writingTo path: String, columns: Int = 100, rows: Int = 30) -> Never {
        let view = TerminalRenderView()
        view.frame = NSRect(x: 0, y: 0, width: 900, height: 520)
        view.layoutSubtreeIfNeeded()

        // Read to EOF: the caller decides how much to feed.
        let input = FileHandle.standardInput
        while true {
            let data = input.availableData
            if data.isEmpty { break }
            view.feed([UInt8](data))
        }

        guard let bitmap = view.bitmapImageRepForCachingDisplay(in: view.bounds) else {
            FileHandle.standardError.write(Data("cannot allocate a bitmap\n".utf8))
            exit(1)
        }
        view.cacheDisplay(in: view.bounds, to: bitmap)

        guard let png = bitmap.representation(using: .png, properties: [:]) else {
            FileHandle.standardError.write(Data("cannot encode a PNG\n".utf8))
            exit(1)
        }
        do {
            try png.write(to: URL(fileURLWithPath: path))
        } catch {
            FileHandle.standardError.write(Data("cannot write \(path): \(error)\n".utf8))
            exit(1)
        }

        print("wrote \(path) — \(bitmap.pixelsWide)×\(bitmap.pixelsHigh)")
        exit(0)
    }
}
