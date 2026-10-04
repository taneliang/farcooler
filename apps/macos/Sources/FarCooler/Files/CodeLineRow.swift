import SwiftUI

/// One line of code, drawn the one way this app draws a line of code
/// (ov-189): a stripe down its leading edge, a gutter of line numbers, a
/// marker, and the text, on a wash.
///
/// Shared by the file viewer (`CodeView`) and both diff renderers, through
/// `DiffLineRow`, so a line reads the same in a file and in a diff of it: the
/// owner's ruling, 3 Oct, "the diff viewer should probably use the same file
/// viewer components as the file viewer". A diff passes two numbers (old and
/// new), a marker and its kind's wash; a file passes one number and nothing
/// else. Colors for a language and editing, when they come, are this row's
/// and the view around it, so both get them at once.
///
/// The text arrives as a `Text`, built by the caller: a diff's plain string,
/// a file's with its find matches marked.
struct CodeLineRow: View {
    /// The marker column's glyph and its color: a diff's `+` and `−`.
    struct Marker {
        var glyph: String
        var color: Color
    }

    /// One per gutter column; nil leaves the cell blank.
    let numbers: [Int?]
    let text: Text
    /// One gutter column's width.
    let gutter: CGFloat
    let font: Font
    var marker: Marker?
    /// The leading edge's stripe, 2 pt. None draws no stripe at all.
    var stripe: Color?
    var wash: Color = .clear
    /// Cut a long line off at the row's width instead of running past it.
    var clipsLongLines = false

    var body: some View {
        HStack(spacing: 0) {
            if let stripe {
                Rectangle()
                    .fill(stripe)
                    .frame(width: 2)
            }
            HStack(spacing: 0) {
                ForEach(numbers.indices, id: \.self) { column in
                    Text(numbers[column].map(String.init) ?? "")
                        .frame(width: gutter, alignment: .trailing)
                }
            }
            .foregroundStyle(.tertiary)
            .background(WorkspaceStyle.diffGutter)
            .overlay(alignment: .trailing) {
                // style-exempt: the gutter's rule, a grid line drawn as the diff always drew it
                Rectangle().fill(WorkspaceStyle.hairline.opacity(0.65)).frame(width: 1)
            }
            if let marker {
                Text(marker.glyph)
                    .foregroundStyle(marker.color)
                    .frame(width: 12)
            }
            if clipsLongLines {
                text
                    .lineLimit(1)
                    .frame(maxWidth: .infinity, alignment: .leading)
            } else {
                text
                    .fixedSize(horizontal: true, vertical: false)
                Spacer(minLength: 0)
            }
        }
        .font(font)
        .padding(.vertical, 0.5)
        .background(wash)
    }
}
