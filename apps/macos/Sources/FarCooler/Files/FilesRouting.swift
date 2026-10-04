import SwiftUI

/// Where a window's Files are (ov-189): each worktree's model, the
/// inspector's worktree, and the Files last clicked into.
///
/// Its own object, out of `ContentView`, which owns one and decides only
/// what it alone knows: whether the task on screen works in a worktree, so a
/// path opens on that task's Files tab rather than in the inspector.
@MainActor
final class FilesRouting: ObservableObject {
    /// Each worktree's Files, by the connection it reads through, so a runner
    /// reconnected reads through its new one. Not published: making a model
    /// changes nothing on screen, so it may be made while a view is drawn.
    private var models: [String: FilesModel] = [:]
    /// The worktree whose Files the inspector shows; nil while it's closed.
    @Published var inspector: Worktree?
    /// A runner's extra read-only folder (ov-232): which one, by its runner
    /// and name. Shown in the same inspector, one at a time with `inspector`.
    struct FolderRef: Hashable {
        var host: String
        var name: String
        var id: String { FilesModel.Place(folder: name, host: host).id }
    }

    /// The extra folder whose Files the inspector shows; nil while it's not.
    @Published var inspectorFolder: FolderRef?
    /// Whether the inspector is open on anything.
    var inspectorOpen: Bool { inspector != nil || inspectorFolder != nil }
    /// What the inspector shows, by `FilesModel.Place.id`.
    var inspectorID: String? { inspector?.id ?? inspectorFolder?.id }

    /// The worktree whose Files was clicked into last: what ⌘F and ⇧⌘L act
    /// on while that Files is on screen.
    @Published var focus: String?

    /// `ws`'s Files, made the first time it's asked for and kept.
    func model(for ws: Worktree, client: DaemonClient) -> FilesModel {
        let key = "\(ObjectIdentifier(client).hashValue)/\(ws.id)"
        if let existing = models[key] { return existing }
        let made = FilesModel(worktree: ws, client: client)
        models[key] = made
        return made
    }

    /// `ref`'s Files, made the first time it's asked for and kept.
    func model(for ref: FolderRef, client: DaemonClient) -> FilesModel {
        let key = "\(ObjectIdentifier(client).hashValue)/\(ref.id)"
        if let existing = models[key] { return existing }
        let made = FilesModel(folder: ref.name, client: client)
        models[key] = made
        return made
    }

    /// Open the inspector on a runner's extra folder, in place of any
    /// worktree's Files.
    func openFolder(host: String, name: String) {
        let ref = FolderRef(host: host, name: name)
        inspector = nil
        inspectorFolder = ref
        focus = ref.id
    }

    /// The toolbar's Show Files: open the inspector on `ws`, or close it.
    func toggleInspector(for ws: Worktree) {
        if inspector?.id == ws.id {
            inspector = nil
        } else {
            inspectorFolder = nil
            inspector = ws
            focus = ws.id
        }
    }

    /// Open `path` in `ws`'s Files at 1-based `line`: on the task's Files
    /// tab when `onTaskTab` (the caller has brought it to the front), else
    /// in the inspector.
    func show(_ path: String, line: Int?, in ws: Worktree, client: DaemonClient, onTaskTab: Bool) {
        let model = self.model(for: ws, client: client)
        if !onTaskTab {
            inspectorFolder = nil
            inspector = ws
        }
        focus = ws.id
        Task { await model.open(path, line: line) }
    }

    /// The Files clicked into last, when it's the one on screen (`shown`)
    /// and has a file open: what ⌘F and ⇧⌘L act on.
    func focused(shown: String?) -> FilesModel? {
        guard let focus, shown == focus else { return nil }
        return models.values.first { $0.place.id == focus && !$0.lines.isEmpty }
    }

    /// Going somewhere else closes the inspector: Files is beside the
    /// worktree on screen (`target`), never a leftover from the last one.
    func follow(_ target: Worktree?) {
        if let shown = inspector, target?.id != shown.id { inspector = nil }
    }

    /// Show Files in the toolbar, for the worktree Changes would split
    /// (`ws`) on a runner that can read files, lit while the inspector shows it.
    func toolbar(_ ws: Worktree?, client: DaemonClient?) -> WorktreeToolbar.Files? {
        guard let ws, client?.showsFiles != false else { return nil }
        return WorktreeToolbar.Files(worktree: ws, open: inspector?.id == ws.id)
    }

    /// What ⌘P's `/` searches: `ws`'s files, on a runner that can read them.
    static func palette(_ ws: Worktree?, client: DaemonClient?) -> PaletteFiles? {
        guard let ws, let client, client.showsFiles != false else { return nil }
        return PaletteFiles(worktree: ws) { query in await client.searchFiles(in: ws, query: query) }
    }
}

/// What the Files inspector draws: its worktree's Files, or why not.
struct FilesInspector: View {
    @ObservedObject var routing: FilesRouting
    let client: (Worktree) -> DaemonClient?
    /// The runner a folder is on, by its host.
    let folderClient: (String) -> DaemonClient?

    var body: some View {
        if let ref = routing.inspectorFolder {
            if let client = folderClient(ref.host) {
                FilesPane(
                    model: routing.model(for: ref, client: client), onFocus: { routing.focus = ref.id },
                    onClose: { routing.inspectorFolder = nil })
            } else {
                PaneNotice(title: "No Runner", detail: "This folder’s runner isn’t connected.")
            }
        } else if let ws = routing.inspector, let client = client(ws) {
            FilesPane(
                model: routing.model(for: ws, client: client), onFocus: { routing.focus = ws.id },
                onClose: { routing.inspector = nil })
        } else {
            PaneNotice(title: "No Worktree", detail: "This worktree isn’t on a connected runner.")
        }
    }
}
