import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// The Files view (ov-189), drawn offscreen in both appearances and written
/// where `FARCOOLER_GLANCE_OUT` says: a file open with a find up and lines
/// chosen, the tree filtered, and a diff drawn through the shared row. No
/// input is sent to anything: the state is set on the model.
@MainActor
struct FilesSpecimenTests {
    private static let worktree = Worktree(
        id: "w-1", short: "w1", task: "ov-189", branch: "ov-189", repository: nil, host: "",
        path: "/tmp/ov-189", state: "active", terminals: [])

    private static let source = """
        //! A worktree's files, read-only.

        use std::path::Path;

        /// The file at `relative` in the worktree at `root`.
        pub fn read(root: &Path, relative: &str) -> Result<Content, Refusal> {
            let path = plain(relative)?;
            let (dirs, name) = split(path).map_err(refusal)?;
            let dir = open_dir(root, dirs)?;
            // O_NOFOLLOW: a link swapped in since the look is refused.
            let token = std::env::var("TOKEN").unwrap_or_default();
            Ok(Content::Text { text: token, size: 0 })
        }

        """

    private static func model() -> FilesModel {
        let entry = { (name: String, kind: FileEntry.Kind) in FileEntry(name: name, kind: kind, size: 0, linkTarget: "") }
        return FilesModel(
            worktree: worktree,
            source: FilesModel.Source(
                list: { path in
                    switch path {
                    case "": return .success(FileListing(
                        path: "", entries: [entry("apps", .directory), entry("crates", .directory), entry(".env", .file),
                                            entry("Cargo.toml", .file), entry("README.md", .file)],
                        truncated: false))
                    case "crates": return .success(FileListing(
                        path: "crates", entries: [entry("daemon", .directory), entry("protocol", .directory)], truncated: false))
                    case "crates/daemon": return .success(FileListing(
                        path: "crates/daemon", entries: [entry("src", .directory), entry("Cargo.toml", .file)], truncated: false))
                    case "crates/daemon/src": return .success(FileListing(
                        path: "crates/daemon/src", entries: [entry("worktree_files.rs", .file), entry("rpc.rs", .file)],
                        truncated: false))
                    default: return .failure(.missing)
                    }
                },
                read: { path in
                    .success(FileRead(path: path, state: .text, size: 640, text: source, linkTarget: ""))
                },
                search: { _ in ["crates/daemon/src/main.rs", "crates/cli/src/main.rs", "apps/macos/Sources/FarCooler/FarCoolerApp.swift"] }))
    }

    /// A runner's extra read-only folder (ov-232): `logs`, with a file open,
    /// or refused by the runner since it was listed.
    private static func folderModel(refused: Bool) -> FilesModel {
        let entry = { (name: String, kind: FileEntry.Kind, size: UInt64) in
            FileEntry(name: name, kind: kind, size: size, linkTarget: "")
        }
        return FilesModel(
            place: FilesModel.Place(folder: "logs", host: ""),
            source: FilesModel.Source(
                list: { path in
                    if refused { return .failure(.folderGone) }
                    switch path {
                    case "": return .success(FileListing(
                        path: "", entries: [entry("nginx", .directory, 0), entry("syslog", .file, 4_200),
                                            entry("auth.log", .file, 1_900)], truncated: false))
                    case "nginx": return .success(FileListing(
                        path: "nginx", entries: [entry("access.log", .file, 90_000), entry("error.log", .file, 700)],
                        truncated: false))
                    default: return .failure(.missingInFolder)
                    }
                },
                read: { path in
                    .success(FileRead(
                        path: path, state: .text, size: 220,
                        text: "Oct  4 06:49:01 runner sshd[311]: Accepted publickey for farcoolerd\nOct  4 06:49:07 runner cron[402]: (farcoolerd) CMD (backup)\n",
                        linkTarget: ""))
                },
                search: { _ in [] }))
    }

    @Test("Write the Files sheets")
    func writeSheets() async throws {
        let directory = URL(
            fileURLWithPath: ProcessInfo.processInfo.environment["FARCOOLER_GLANCE_OUT"]
                ?? FileManager.default.currentDirectoryPath + "/.build/glance")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)

        let open = Self.model()
        await open.loadIfNeeded()
        await open.open("crates/daemon/src/worktree_files.rs", line: nil)
        open.click(line: 5, extending: false)
        open.click(line: 8, extending: true)
        open.finding = true
        open.findQuery = "dir"
        open.refind()
        #expect(open.lines.count == 13)

        let filtered = Self.model()
        await filtered.loadIfNeeded()
        filtered.filter = "main"
        await filtered.runFilter()
        #expect(filtered.found.count == 3)

        let folder = Self.folderModel(refused: false)
        await folder.loadIfNeeded()
        await folder.toggle("nginx")
        await folder.open("syslog", line: nil, reveal: false)
        #expect(folder.lines.count == 2)
        let refused = Self.folderModel(refused: true)
        await refused.loadIfNeeded()
        #expect(refused.folders[""] == .failed(.folderGone))

        let diff = Diff(
            path: "crates/daemon/src/worktree_files.rs",
            oldText: "let dir = open(root)?;\nlet name = n;\n",
            newText: "let dir = open_dir(root, dirs)?;\nlet name = n;\nlet held = fstat(&fd)?;\n")

        for dark in [false, true] {
            let suffix = dark ? "dark" : "light"
            try write(FilesPane(model: open).frame(width: 980, height: 420), dark: dark,
                      to: directory.appendingPathComponent("files-open-\(suffix).png"))
            try write(FilesPane(model: filtered).frame(width: 980, height: 300), dark: dark,
                      to: directory.appendingPathComponent("files-filtered-\(suffix).png"))
            try write(FilesPane(model: folder).frame(width: 980, height: 320), dark: dark,
                      to: directory.appendingPathComponent("files-folder-\(suffix).png"))
            try write(FilesPane(model: refused).frame(width: 980, height: 220), dark: dark,
                      to: directory.appendingPathComponent("files-folder-refused-\(suffix).png"))
            try write(DiffView(diff: diff).padding(12).frame(width: 620), dark: dark,
                      to: directory.appendingPathComponent("diff-shared-row-\(suffix).png"))
        }
    }

    private func write<V: View>(_ view: V, dark: Bool, to url: URL) throws {
        let host = NSHostingView(rootView: view.background(dark ? Color(white: 0.12) : Color.white))
        host.appearance = NSAppearance(named: dark ? .darkAqua : .aqua)
        host.frame = CGRect(origin: .zero, size: host.fittingSize)
        host.layoutSubtreeIfNeeded()
        let rep = try #require(host.bitmapImageRepForCachingDisplay(in: host.bounds))
        host.cacheDisplay(in: host.bounds, to: rep)
        let png = try #require(rep.representation(using: .png, properties: [:]))
        try png.write(to: url)
    }
}
