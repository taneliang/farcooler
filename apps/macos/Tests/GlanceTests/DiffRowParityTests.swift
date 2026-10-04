import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// The diff draws exactly what it drew before it moved onto the file
/// viewer's row (ov-189).
///
/// `DiffLineRow` is now `CodeLineRow` with two gutter columns, a marker and a
/// wash. The refactor was meant to change nothing a reader sees, and "the
/// same" is a claim about pixels, so this rasterises the real `DiffLineRow`
/// beside `FrozenDiffLineRow`, a verbatim copy of its body from before the
/// move, and asks for identical bytes: every kind of line, both gutters
/// blank and full, an empty line, a long line clipped and running on, in
/// Light and Dark. Change the row's padding, its gutter, its stripe or its
/// marker column and this fails.
@MainActor
struct DiffRowParityTests {
    private static let lines: [DiffComputation.Line] = [
        .init(id: 0, kind: .context, oldNumber: 9, newNumber: 9, text: "fn main() {"),
        .init(id: 1, kind: .removed, oldNumber: 10, newNumber: nil, text: "    let x = 1;"),
        .init(id: 2, kind: .added, oldNumber: nil, newNumber: 10, text: "    let x = 2;"),
        .init(id: 3, kind: .context, oldNumber: 11, newNumber: 11, text: ""),
        .init(
            id: 4, kind: .added, oldNumber: nil, newNumber: 12_345,
            text: String(repeating: "a very long line that runs past the row ", count: 6)),
    ]

    private static let font = Font.system(size: WorkspaceStyle.PaneText.body, design: .monospaced)

    @Test(arguments: [false, true])
    func everyLineDrawsAsItDidBefore(clipped: Bool) throws {
        for scheme in [ColorScheme.light, .dark] {
            for line in Self.lines {
                let now = try bytes(
                    DiffLineRow(line: line, gutter: 34, font: Self.font, clipsLongLines: clipped), scheme)
                let before = try bytes(
                    FrozenDiffLineRow(line: line, gutter: 34, font: Self.font, clipsLongLines: clipped), scheme)
                #expect(now.size == before.size, "line \(line.id), \(scheme), clipped \(clipped)")
                #expect(now.pixels == before.pixels, "line \(line.id), \(scheme), clipped \(clipped)")
            }
        }
    }

    /// One row, at a width a pane might be, as raw RGBA.
    private func bytes<V: View>(_ row: V, _ scheme: ColorScheme) throws -> (size: CGSize, pixels: [UInt8]) {
        let renderer = ImageRenderer(
            content: row.frame(width: 420, alignment: .leading)
                .background(scheme == .dark ? Color.black : Color.white)
                .environment(\.colorScheme, scheme))
        renderer.proposedSize = ProposedViewSize(width: 420, height: nil)
        renderer.scale = 2
        let cg = try #require(renderer.cgImage, "ImageRenderer produced no image")
        var pixels = [UInt8](repeating: 0, count: cg.width * cg.height * 4)
        pixels.withUnsafeMutableBytes { raw in
            let context = CGContext(
                data: raw.baseAddress, width: cg.width, height: cg.height,
                bitsPerComponent: 8, bytesPerRow: cg.width * 4,
                space: CGColorSpace(name: CGColorSpace.sRGB)!,
                bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)
            context?.draw(cg, in: CGRect(x: 0, y: 0, width: cg.width, height: cg.height))
        }
        return (CGSize(width: cg.width, height: cg.height), pixels)
    }
}

/// `DiffLineRow`'s body as it was before ov-189, copied verbatim, with no
/// anchor: what the diff is held to.
private struct FrozenDiffLineRow: View {
    let line: DiffComputation.Line
    let gutter: CGFloat
    let font: Font
    let clipsLongLines: Bool

    var body: some View {
        HStack(spacing: 0) {
            Rectangle()
                .fill(line.kind.accent)
                .frame(width: 2)
            HStack(spacing: 0) {
                Text(line.oldNumber.map(String.init) ?? "")
                    .frame(width: gutter, alignment: .trailing)
                Text(line.newNumber.map(String.init) ?? "")
                    .frame(width: gutter, alignment: .trailing)
            }
            .foregroundStyle(.tertiary)
            .background(WorkspaceStyle.diffGutter)
            .overlay(alignment: .trailing) {
                Rectangle().fill(WorkspaceStyle.hairline.opacity(0.65)).frame(width: 1)
            }
            Text(line.kind.marker)
                .foregroundStyle(line.kind.accent)
                .frame(width: 12)
            if clipsLongLines {
                Text(line.text.isEmpty ? " " : line.text)
                    .lineLimit(1)
                    .frame(maxWidth: .infinity, alignment: .leading)
            } else {
                Text(line.text.isEmpty ? " " : line.text)
                    .fixedSize(horizontal: true, vertical: false)
                Spacer(minLength: 0)
            }
        }
        .font(font)
        .padding(.vertical, 0.5)
        .background(line.kind.wash)
        .overlay(alignment: .leading) { EmptyView() }
    }
}
