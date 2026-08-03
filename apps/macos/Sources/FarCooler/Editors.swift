import AppKit
import Foundation
import SwiftUI

/// An editor Far Cooler can hand a worktree to.
///
/// A command is an argv array, not a shell command line. Worktree paths with
/// spaces in them are ordinary — `~/Dev/My Project/feat-auth` — and a shell
/// string would make quoting the caller's problem in the one place where
/// getting it wrong opens the wrong directory silently rather than failing.
/// There is no shell in the launch path, so there is nothing to quote.
///
/// `remote` is optional and NOT defaulted, because that is the whole gate: an
/// editor with no way to reach another machine says so by having nothing to
/// say, and every remote decision in this file reads that one field.
struct Editor: Identifiable, Hashable, Codable {
    /// Stable across launches and across renames — it is what
    /// `Preferences.lastUsedEditor` stores.
    var id: String
    var name: String

    /// Arguments for a worktree on this Mac.
    ///
    /// `{path}` becomes the worktree's path. For a built-in these are arguments
    /// only — the launcher in front of them comes from `bundle`. For a custom
    /// editor the first element is the binary, because that is what the user
    /// typed.
    var local: [String]

    /// Arguments for a worktree on another machine, with `{host}` as well.
    /// Nil means this editor cannot open one.
    var remote: [String]?

    /// Where the launcher lives, for a built-in. Nil for a custom editor.
    var bundle: BundledLauncher?

    var opensRemote: Bool { remote != nil }
}

/// How to start a built-in editor.
///
/// Through the app bundle, never `PATH`. `CLI.swift` documents the same
/// reasoning for finding the Far Cooler CLI: a double-clicked app inherits no
/// shell environment, so a `code` that works in your terminal is a binary this
/// process cannot find. A `PATH` lookup would work for whoever launched the app
/// from a shell — which includes everyone who built it — and fail silently for
/// everyone else.
struct BundledLauncher: Hashable, Codable {
    /// How the application is asked to open something.
    enum Style: Hashable, Codable {
        /// A launcher script inside the bundle, run directly. It hands the
        /// arguments to a running instance, or starts one, and returns.
        case launcher(String)

        /// `open -na <app> --args …`.
        ///
        /// For applications whose bundle executable IS the application:
        /// `IntelliJ IDEA.app/Contents/MacOS/idea` and `Emacs.app`'s binary do
        /// not fork. Running one directly means Far Cooler holds a child
        /// process for as long as the editor is open — and, worse, the launch
        /// check below would sit through its five-second wait every time and
        /// then report a perfectly healthy editor as still running. `open`
        /// forks and returns, which is what the launcher scripts do for the
        /// editors that ship one.
        case openApplication
    }

    /// Tried in order. Editors ship under more than one identifier: JetBrains
    /// IDEs differ between their paid and community builds, Sublime Text's
    /// changes with its major version, and Zed has three release channels.
    var identifiers: [String]

    /// Application names on disk, for the fallback scan.
    ///
    /// A wrong identifier fails invisibly — the editor simply never appears,
    /// and nobody can tell that apart from a machine that does not have it. The
    /// names give detection a second way to be right. More than one because an
    /// application can be renamed between versions: PyCharm's bundle used to be
    /// `PyCharm Professional Edition.app`.
    var appNames: [String]

    var style: Style

    /// Directories scanned when no identifier matches.
    ///
    /// Not a substitute for `identifiers`, which finds an app wherever it is
    /// installed — Spotlight being disabled on a volume is the case this covers.
    private static let searchPaths = [
        "/Applications",
        "\(NSHomeDirectory())/Applications",
        "/System/Applications",
        "/Applications/Utilities",
    ]

    /// The argv this editor's arguments get appended to, or nil if it is not
    /// installed.
    func resolve() -> [String]? {
        guard let app = application() else { return nil }
        switch style {
        case .launcher(let relative):
            let path = app.appendingPathComponent(relative).path
            guard FileManager.default.isExecutableFile(atPath: path) else { return nil }
            return [path]
        case .openApplication:
            return ["/usr/bin/open", "-na", app.path, "--args"]
        }
    }

    /// The application bundle, by identifier first and by name second.
    private func application() -> URL? {
        for identifier in identifiers {
            if let app = NSWorkspace.shared
                .urlForApplication(withBundleIdentifier: identifier)
            {
                return app
            }
        }
        for directory in Self.searchPaths {
            for name in appNames {
                let app = URL(fileURLWithPath: directory).appendingPathComponent(name)
                if FileManager.default.fileExists(atPath: app.path) { return app }
            }
        }
        return nil
    }
}

// MARK: - The editors Far Cooler knows

extension Editor {
    /// A VS Code-family editor.
    ///
    /// The whole family — Insiders, VSCodium, Cursor — is the same CLI with a
    /// different binary name, so it is described once.
    ///
    /// `--folder-uri` both times, never a positional path, for two independent
    /// reasons that happen to have the same fix.
    ///
    /// A remote path cannot be stat'd, so VS Code guesses what it is from the
    /// name: a basename containing a dot is opened as a FILE. A worktree called
    /// `api-v2.1` would arrive as a text document.
    ///
    /// And a positional path is word-split by Cursor's launcher — see
    /// `Editor.encode`. Every worktree Far Cooler creates lives under
    /// `~/Library/Application Support/`, so that is not an edge case here, it is
    /// the default case. A percent-encoded URI has no spaces to split on.
    ///
    /// A `--folder-uri` is also unconditionally a folder, which is the thing
    /// being asked for.
    private static func vscodeFamily(
        id: String, name: String, identifiers: [String], appNames: [String], binary: String
    ) -> Editor {
        Editor(
            id: id,
            name: name,
            local: ["--folder-uri", "file://{encodedPath}"],
            remote: ["--folder-uri", "vscode-remote://ssh-remote+{host}{encodedPath}"],
            bundle: BundledLauncher(
                identifiers: identifiers,
                appNames: appNames,
                style: .launcher("Contents/Resources/app/bin/\(binary)")))
    }

    /// A JetBrains IDE.
    ///
    /// Local only. JetBrains does have remote development, but it is JetBrains
    /// Gateway: a separate application, with its own connection flow and its own
    /// backend to install on the host, and no one-shot "open this remote path"
    /// invocation to call. An editor whose remote story is *go and use a
    /// different application* is one to be honest about not covering.
    private static func jetBrains(
        id: String, name: String, identifiers: [String], appNames: [String]
    ) -> Editor {
        Editor(
            id: id, name: name, local: ["{path}"], remote: nil,
            bundle: BundledLauncher(
                identifiers: identifiers, appNames: appNames, style: .openApplication))
    }

    /// Every editor Far Cooler can find by itself, in menu order.
    ///
    /// Order is deliberate: the ones that can also open a worktree on another
    /// machine come first, because this is a tool for driving a fleet and the
    /// default for someone who has several editors installed should be one that
    /// works everywhere they work.
    static let builtIns: [Editor] = [
        Editor(
            id: "zed",
            name: "Zed",
            local: ["{path}"],
            // Verified against the shipped `cli`: it parses the URL itself and
            // rejects a malformed one before Zed is contacted. An unencoded
            // space parses, so the raw path is what goes here.
            remote: ["ssh://{host}{path}"],
            bundle: BundledLauncher(
                identifiers: [
                    "dev.zed.Zed", "dev.zed.Zed-Preview", "dev.zed.Zed-Nightly",
                    "dev.zed.Zed-Dev",
                ],
                appNames: ["Zed.app", "Zed Preview.app", "Zed Nightly.app", "Zed Dev.app"],
                style: .launcher("Contents/MacOS/cli"))),

        vscodeFamily(
            id: "vscode", name: "Visual Studio Code",
            identifiers: ["com.microsoft.VSCode"],
            appNames: ["Visual Studio Code.app"], binary: "code"),
        vscodeFamily(
            id: "vscode-insiders", name: "VS Code Insiders",
            identifiers: ["com.microsoft.VSCodeInsiders"],
            // The launcher inside Insiders is `code`, not `code-insiders`.
            appNames: ["Visual Studio Code - Insiders.app"], binary: "code"),
        vscodeFamily(
            id: "cursor", name: "Cursor",
            identifiers: ["com.todesktop.230313mzl4w4u92"],
            appNames: ["Cursor.app"], binary: "cursor"),
        vscodeFamily(
            id: "vscodium", name: "VSCodium",
            identifiers: ["com.vscodium", "com.visualstudio.code.oss"],
            appNames: ["VSCodium.app", "Code - OSS.app"], binary: "codium"),

        Editor(
            id: "sublime", name: "Sublime Text", local: ["{path}"], remote: nil,
            bundle: BundledLauncher(
                identifiers: ["com.sublimetext.4", "com.sublimetext.3"],
                appNames: ["Sublime Text.app"],
                style: .launcher("Contents/SharedSupport/bin/subl"))),
        Editor(
            id: "nova", name: "Nova", local: ["{path}"], remote: nil,
            bundle: BundledLauncher(
                identifiers: ["com.panic.Nova"], appNames: ["Nova.app"],
                style: .launcher("Contents/SharedSupport/nova"))),
        Editor(
            id: "bbedit", name: "BBEdit", local: ["{path}"], remote: nil,
            bundle: BundledLauncher(
                identifiers: ["com.barebones.bbedit"], appNames: ["BBEdit.app"],
                style: .launcher("Contents/Helpers/bbedit_tool"))),
        Editor(
            id: "textmate", name: "TextMate", local: ["{path}"], remote: nil,
            bundle: BundledLauncher(
                identifiers: ["com.macromates.TextMate"], appNames: ["TextMate.app"],
                style: .launcher("Contents/Resources/mate"))),
        Editor(
            id: "xcode", name: "Xcode", local: ["{path}"], remote: nil,
            bundle: BundledLauncher(
                identifiers: ["com.apple.dt.Xcode"], appNames: ["Xcode.app"],
                style: .launcher("Contents/Developer/usr/bin/xed"))),
        Editor(
            id: "macvim", name: "MacVim", local: ["{path}"], remote: nil,
            bundle: BundledLauncher(
                identifiers: ["org.vim.MacVim"], appNames: ["MacVim.app"],
                style: .openApplication)),
        Editor(
            // `Emacs.app`'s bundle executable does not fork, so it is opened
            // rather than run. `emacsclient` would attach to a running server
            // instead — better, when there is one, and nothing at all when
            // there is not.
            id: "emacs", name: "Emacs", local: ["{path}"], remote: nil,
            bundle: BundledLauncher(
                identifiers: ["org.gnu.Emacs"], appNames: ["Emacs.app"],
                style: .openApplication)),

        jetBrains(
            id: "idea", name: "IntelliJ IDEA",
            identifiers: ["com.jetbrains.intellij", "com.jetbrains.intellij.ce"],
            appNames: ["IntelliJ IDEA.app", "IntelliJ IDEA Community Edition.app"]),
        jetBrains(
            id: "pycharm", name: "PyCharm",
            identifiers: ["com.jetbrains.pycharm", "com.jetbrains.pycharm.ce"],
            appNames: [
                "PyCharm.app", "PyCharm Community Edition.app",
                "PyCharm Professional Edition.app",
            ]),
        jetBrains(
            id: "webstorm", name: "WebStorm",
            identifiers: ["com.jetbrains.WebStorm"], appNames: ["WebStorm.app"]),
        jetBrains(
            id: "goland", name: "GoLand",
            identifiers: ["com.jetbrains.goland"], appNames: ["GoLand.app"]),
        jetBrains(
            id: "rustrover", name: "RustRover",
            identifiers: ["com.jetbrains.rustrover"], appNames: ["RustRover.app"]),
        jetBrains(
            id: "clion", name: "CLion",
            identifiers: ["com.jetbrains.CLion"], appNames: ["CLion.app"]),
        jetBrains(
            id: "rubymine", name: "RubyMine",
            identifiers: ["com.jetbrains.rubymine"], appNames: ["RubyMine.app"]),
        jetBrains(
            id: "phpstorm", name: "PhpStorm",
            identifiers: ["com.jetbrains.PhpStorm"], appNames: ["PhpStorm.app"]),
        jetBrains(
            id: "android-studio", name: "Android Studio",
            identifiers: ["com.google.android.studio"], appNames: ["Android Studio.app"]),
    ]
}

// MARK: - Launching

extension Editor {
    /// What running this editor on this worktree would actually execute, or nil
    /// if it cannot.
    ///
    /// Separate from running it so the menu can ask the same question the
    /// launch will answer — an item is enabled exactly when this returns
    /// something — and so it can be tested without starting an editor.
    func command(path: String, host: String) -> [String]? {
        let remoteTarget = host.trimmingCharacters(in: .whitespaces)
        let template = remoteTarget.isEmpty ? local : remote
        guard let template, !template.isEmpty else { return nil }

        let arguments = template.map {
            $0.replacingOccurrences(of: "{path}", with: path)
                .replacingOccurrences(of: "{encodedPath}", with: Self.encode(path))
                .replacingOccurrences(of: "{host}", with: remoteTarget)
        }

        // A custom editor's first word is already a binary the user typed. A
        // built-in's launcher is whatever `resolve` found, which is also how
        // "not installed" is answered.
        guard let bundle else { return arguments }
        guard let prefix = bundle.resolve() else { return nil }
        return prefix + arguments
    }

    /// A path as it must appear inside a URI.
    ///
    /// Deliberately stricter than `.urlPathAllowed`, which permits the
    /// sub-delimiters `$ & ' ( ) * + , ; = ! @`. Everything outside the
    /// unreserved set is encoded, leaving only `/` as structure.
    ///
    /// This is not pedantry about URIs — percent-encoding a sub-delimiter is
    /// always legal, so nothing is lost, and what is gained is that the result
    /// cannot be re-interpreted by a shell. Cursor's launcher ends in
    /// `eval "$CURSOR_CLI" "$@"`, which word-splits and expands its arguments a
    /// second time. A worktree under `~/Library/Application Support/…` — where
    /// Far Cooler puts every worktree it creates — arrives at the editor as two
    /// paths, and opens as two empty files instead of the project. A `$` or a
    /// `;` in a branch name would be worse than useless.
    static func encode(_ path: String) -> String {
        var allowed = CharacterSet.alphanumerics
        allowed.insert(charactersIn: "-._~/")
        return path.addingPercentEncoding(withAllowedCharacters: allowed) ?? path
    }

    /// Why this editor is unavailable for this worktree, in a sentence.
    ///
    /// The menu shows an item disabled rather than hiding it: an editor you have
    /// installed, missing from a list of editors, reads as a bug. Saying which
    /// of the two reasons applies is the difference between a control that is
    /// broken and one that is honest.
    func unavailability(host: String) -> String? {
        if !host.trimmingCharacters(in: .whitespaces).isEmpty, !opensRemote {
            return "\(name) cannot open worktrees on another machine"
        }
        if let bundle, bundle.resolve() == nil {
            return "\(name) is not installed"
        }
        return nil
    }
}

/// Starting editors, and saying so when one will not start.
enum EditorLaunch {
    /// The environment an editor's launcher is started in.
    ///
    /// This process's own, minus the variables that make a VS Code-family CLI
    /// stop being a launcher. `VSCODE_IPC_HOOK_CLI` makes `code` forward the
    /// invocation to whichever VS Code window owns that socket instead of
    /// opening a window here; Cursor's equivalents do the same. Far Cooler
    /// inherits them whenever it is started from a terminal inside one of those
    /// editors — which, for the people this feature is for, is often.
    static var childEnvironment: [String: String] {
        var environment = ProcessInfo.processInfo.environment
        for key in ["VSCODE_IPC_HOOK_CLI", "CURSOR_CLI", "CURSOR_CLI_MODE"] {
            environment.removeValue(forKey: key)
        }
        return environment
    }

    /// Run the editor, and hand back what went wrong.
    ///
    /// Returns nil on success. Callers put the message in the app's banner.
    ///
    /// The launchers are all thin: `code` and `cursor` are shell scripts that
    /// hand off to Electron, Zed's `cli` talks to a running instance or starts
    /// one, and anything that would block is started through `open` instead
    /// (see `BundledLauncher.Style`). All of them return promptly, so waiting
    /// for the exit status is what makes a mistyped custom command say
    /// something instead of nothing. The wait is bounded anyway, so an editor
    /// that does block cannot hang the app.
    ///
    /// What this CANNOT catch: VS Code's CLI exits 0 for almost everything,
    /// including flags it does not recognise, and reports a failed SSH
    /// connection or a missing Remote-SSH extension inside its own window
    /// rather than to whoever spawned it. So a zero exit here means "the
    /// launcher ran", not "the worktree opened". Detection resolves the app and
    /// the launcher on disk before any of this, which is the part that can be
    /// checked honestly.
    static func run(_ argv: [String]) async -> String? {
        guard let binary = argv.first else { return "Nothing to run." }

        return await withCheckedContinuation { continuation in
            DispatchQueue.global(qos: .userInitiated).async {
                let process = Process()
                process.executableURL = URL(fileURLWithPath: binary)
                process.arguments = Array(argv.dropFirst())
                process.environment = childEnvironment
                // The launcher's own complaint is the useful one — "command not
                // found", "Remote-SSH is not installed" — so it is captured
                // rather than sent to a console nobody is reading.
                let errors = Pipe()
                process.standardError = errors
                process.standardOutput = Pipe()

                do {
                    try process.run()
                } catch {
                    continuation.resume(returning: "\(binary): \(error.localizedDescription)")
                    return
                }

                let deadline = Date().addingTimeInterval(5)
                while process.isRunning, Date() < deadline {
                    Thread.sleep(forTimeInterval: 0.05)
                }

                // Still going after five seconds is an editor that stays in the
                // foreground, not a failure. Leave it alone: killing it here
                // would close the window the user just asked for.
                guard !process.isRunning else {
                    continuation.resume(returning: nil)
                    return
                }

                guard process.terminationStatus != 0 else {
                    continuation.resume(returning: nil)
                    return
                }

                let said = String(
                    data: errors.fileHandleForReading.readDataToEndOfFile(),
                    encoding: .utf8)?
                    .trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
                let name = URL(fileURLWithPath: binary).lastPathComponent
                continuation.resume(
                    returning: said.isEmpty
                        ? "\(name) exited \(process.terminationStatus)."
                        : said)
            }
        }
    }
}

// MARK: - The catalogue

/// Which editors this Mac has, which one you used last, and the ones you added.
///
/// A singleton for the same reason `Hosts` is one: several unrelated views ask
/// the same question — the title bar control, the sidebar menu, and Settings —
/// and two of them computing their own answer would be two lists that disagree
/// about what is installed.
@MainActor
final class Editors: ObservableObject {
    static let shared = Editors()

    /// Every built-in found on this machine, in table order, plus every custom
    /// editor. Custom ones come last: they are the exception, and the exception
    /// should not sort above the thing it is an exception to.
    @Published private(set) var available: [Editor] = []

    /// Editors the user defined by hand.
    @Published private(set) var custom: [Editor] = [] {
        didSet { saveCustom(); refresh() }
    }

    private let customKey = "editors.custom"

    private init() {
        loadCustom()
        refresh()
    }

    // MARK: - Detection

    /// Re-probe what is installed.
    ///
    /// Cheap enough to run whenever a menu opens — a handful of
    /// `urlForApplication` calls against a cache Launch Services already keeps —
    /// and running it then is what makes an editor installed while the app was
    /// open appear without a relaunch.
    func refresh() {
        available = Editor.builtIns.filter { $0.bundle?.resolve() != nil } + custom
    }

    // MARK: - Choosing

    /// Which editor a click should use for this worktree.
    ///
    /// The last-used one, unless that editor cannot reach the machine the
    /// worktree is on — in which case the first that can, WITHOUT changing what
    /// is stored. Zed on this Mac and VS Code on the box is a normal way to
    /// work; it should not need a second setting, and it should not leave the
    /// preference flipping every time you switch machines.
    func preferred(host: String) -> Editor? {
        let remote = !host.trimmingCharacters(in: .whitespaces).isEmpty
        let usable = remote ? available.filter(\.opensRemote) : available

        if let id = Preferences.shared.lastUsedEditor.nilIfEmpty,
           let stored = usable.first(where: { $0.id == id })
        {
            return stored
        }
        return usable.first
    }

    /// Record a deliberate choice.
    ///
    /// Only called from an explicit pick in the menu. The fallback in
    /// `preferred(host:)` deliberately does not come through here.
    func remember(_ editor: Editor) {
        Preferences.shared.lastUsedEditor = editor.id
    }

    // MARK: - Opening

    /// Open a worktree, and hand back what went wrong.
    func open(_ workspace: Workspace, with editor: Editor) async -> String? {
        guard let argv = editor.command(
            path: workspace.worktree, host: workspace.host ?? "")
        else {
            return editor.unavailability(host: workspace.host ?? "")
                ?? "\(editor.name) could not be started."
        }
        return await EditorLaunch.run(argv)
    }

    // MARK: - Custom editors

    func addCustom(name: String, local: String, remote: String) {
        let entry = Editor(
            id: "custom:\(UUID().uuidString)",
            name: name.trimmingCharacters(in: .whitespaces),
            local: Editor.split(local),
            remote: Editor.split(remote).isEmpty ? nil : Editor.split(remote),
            bundle: nil)
        guard !entry.name.isEmpty, !entry.local.isEmpty else { return }
        custom.append(entry)
    }

    func updateCustom(_ editor: Editor) {
        guard let index = custom.firstIndex(where: { $0.id == editor.id }) else { return }
        custom[index] = editor
    }

    func removeCustom(_ editor: Editor) {
        custom.removeAll { $0.id == editor.id }
        // A default pointing at an editor that no longer exists would silently
        // fall back forever with no way to tell why.
        if Preferences.shared.lastUsedEditor == editor.id {
            Preferences.shared.lastUsedEditor = ""
        }
    }

    private func loadCustom() {
        guard let data = UserDefaults.standard.data(forKey: customKey),
              let decoded = try? JSONDecoder().decode([Editor].self, from: data)
        else { return }
        custom = decoded
    }

    private func saveCustom() {
        guard let data = try? JSONEncoder().encode(custom) else { return }
        UserDefaults.standard.set(data, forKey: customKey)
    }
}

extension Editor {
    /// Split a typed command into argv.
    ///
    /// Whitespace, and that is the documented limit. Quoting rules here would be
    /// a shell parser nobody asked for, and the argument that needs them —
    /// the worktree path — is substituted after the split, so it never sees one.
    static func split(_ command: String) -> [String] {
        command.split(whereSeparator: \.isWhitespace).map(String.init)
    }
}

private extension String {
    var nilIfEmpty: String? { isEmpty ? nil : self }
}
