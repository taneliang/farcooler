import AppKit
import Foundation

/// Render a byte stream to a PNG and exit.
///
///     farcooler terminal stream <id> | FARCOOLER_RENDER_PROBE=/tmp/out.png Far Cooler
///
/// Terminal rendering is the one part of this app that cannot be asserted in a
/// unit test — "the glyphs are upright, in the right cells, in the right
/// colors" is a claim about pixels. This makes those pixels inspectable
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
        report(view: view, bitmap: bitmap)
        exit(0)
    }

    /// Compare where ink landed against where the core says the text is.
    ///
    /// "The lines are drawn one row too high" is exactly the kind of claim that
    /// is obvious on screen and invisible to every unit test. This puts the two
    /// side by side so an off-by-one is a mismatch in a table rather than an
    /// argument about a screenshot.
    private static func report(view: TerminalRenderView, bitmap: NSBitmapImageRep) {
        let m = view.metrics
        let scale = CGFloat(bitmap.pixelsHigh) / view.bounds.height
        let background = Palette.backgroundPacked

        // Ink per pixel row.
        var inkByPixelRow = [Int](repeating: 0, count: bitmap.pixelsHigh)
        for y in 0..<bitmap.pixelsHigh {
            var count = 0
            for x in stride(from: 0, to: bitmap.pixelsWide, by: 2) {
                guard let color = bitmap.colorAt(x: x, y: y) else { continue }
                let packed =
                    (UInt32(color.redComponent * 255) << 16)
                    | (UInt32(color.greenComponent * 255) << 8)
                    | UInt32(color.blueComponent * 255)
                // Anti-aliasing means "not exactly the background" is too
                // sensitive; require a visible difference.
                if difference(packed, background) > 24 { count += 1 }
            }
            inkByPixelRow[y] = count
        }

        print("\ncell rows that contain text, per the core, versus where ink landed:")
        view.core.withSnapshot { snapshot in
            for row in 0..<snapshot.rows {
                var text = ""
                for column in 0..<snapshot.columns {
                    text.append(snapshot[row, column].character ?? " ")
                }
                let trimmed = text.trimmingCharacters(in: .whitespaces)

                let top = Int((m.paddingTop + CGFloat(row) * m.cellHeight) * scale)
                let bottom = Int((m.paddingTop + CGFloat(row + 1) * m.cellHeight) * scale)
                let band = inkByPixelRow[max(0, top)..<min(bitmap.pixelsHigh, max(top + 1, bottom))]
                let ink = band.reduce(0, +)

                guard !trimmed.isEmpty || ink > 0 else { continue }
                let verdict =
                    trimmed.isEmpty
                    ? "INK IN AN EMPTY ROW" : (ink > 0 ? "ok" : "TEXT WITH NO INK")
                print(
                    "  row \(String(format: "%3d", row))  px \(top)–\(bottom)  ink \(String(format: "%5d", ink))  \(verdict)  \(trimmed.prefix(40))"
                )
            }
        }
    }

    private static func difference(_ a: UInt32, _ b: UInt32) -> Int {
        let dr = abs(Int((a >> 16) & 0xFF) - Int((b >> 16) & 0xFF))
        let dg = abs(Int((a >> 8) & 0xFF) - Int((b >> 8) & 0xFF))
        let db = abs(Int(a & 0xFF) - Int(b & 0xFF))
        return dr + dg + db
    }
}
