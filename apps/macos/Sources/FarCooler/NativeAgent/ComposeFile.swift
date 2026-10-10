import AgentKit
import AppKit
import Foundation
import SwiftUI
import UniformTypeIdentifiers

/// A file waiting in the native composer that isn't an image (ov-454): a
/// PDF, a log, a CSV. The runner writes it into its paste directory under
/// its name and types its path before the message, so the agent reads it
/// where it runs, on this Mac or over ssh. An image goes as a
/// `ComposeImage` instead, which claude reads as a picture.
struct ComposeFile: Identifiable, Equatable, Sendable {
    let id = UUID()
    let name: String
    let data: Data

    static func == (a: ComposeFile, b: ComposeFile) -> Bool { a.id == b.id }

    /// The largest file sent (`MAX_PASTE_FILE_BYTES`).
    static let largest = 16 * 1024 * 1024

    /// The files on `pasteboard` that aren't images or folders: a Finder copy,
    /// or a drop.
    static func urls(on pasteboard: NSPasteboard) -> [URL] {
        let options: [NSPasteboard.ReadingOptionKey: Any] = [.urlReadingFileURLsOnly: true]
        let urls = pasteboard.readObjects(forClasses: [NSURL.self], options: options) as? [URL] ?? []
        return urls.filter(isFile)
    }

    /// Whether `url` is a file this sends: not a folder, and not an image,
    /// which goes as one.
    static func isFile(_ url: URL) -> Bool {
        let values = try? url.resourceValues(forKeys: [.isDirectoryKey, .contentTypeKey])
        if values?.isDirectory == true { return false }
        let type = values?.contentType ?? UTType(filenameExtension: url.pathExtension)
        return type?.conforms(to: .image) != true
    }

    /// The file at `url`, read; nil when it can't be, and `tooLarge` past
    /// `largest`.
    static func read(_ url: URL) -> (file: ComposeFile?, tooLarge: Bool) {
        let size = (try? url.resourceValues(forKeys: [.fileSizeKey]))?.fileSize ?? 0
        if size > largest { return (nil, true) }
        guard let data = try? Data(contentsOf: url), !data.isEmpty, data.count <= largest else { return (nil, false) }
        return (ComposeFile(name: url.lastPathComponent, data: data), false)
    }

    /// A chip's symbol, by the file's kind.
    var symbol: String {
        let type = UTType(filenameExtension: (name as NSString).pathExtension)
        if type?.conforms(to: .pdf) == true { return "doc.richtext" }
        if type?.conforms(to: .sourceCode) == true || type?.conforms(to: .plainText) == true { return "doc.text" }
        return "doc"
    }
}

/// A file waiting in the composer: its kind's symbol and its name, cut in
/// the middle so the extension shows, with a button to take it out.
struct ComposeFileChip: View {
    let file: ComposeFile
    let remove: () -> Void

    var body: some View {
        HStack(spacing: Spacing.tight) {
            Image(systemName: file.symbol)
                .foregroundStyle(.secondary)
            Text(file.name)
                .lineLimit(1)
                .truncationMode(.middle)
                .frame(maxWidth: 180, alignment: .leading)
            Button(action: remove) {
                Image(systemName: "xmark.circle.fill")
                    .symbolRenderingMode(.hierarchical)
                    .foregroundStyle(.secondary)
            }
            .buttonStyle(.plain)
            .help("Remove File")
            .accessibilityLabel("Remove \(file.name)")
        }
        .font(.callout)
        .padding(.horizontal, Spacing.group)
        .frame(height: 48)
        .surface(.inset, in: .control)
        .accessibilityElement(children: .contain)
        .accessibilityLabel(file.name)
        .identified("native-file-chip")
    }
}
