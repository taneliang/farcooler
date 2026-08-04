# Command-line tools on PATH — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let the macOS app symlink its own bundled `farcooler`/`farcoolerd` into `~/.local/bin`, so an SSH session onto the Mac (which is how the iOS app reaches it) can actually find `farcoolerd` instead of failing with "did not answer."

**Architecture:** One new `@MainActor` `ObservableObject`, `CommandLineTools`, mirroring `ServiceRegistration.swift`'s shape exactly (a `State` enum, `refresh()`/`install()`/`uninstall()`). A Settings row in the existing Startup tab drives it. A one-time `.alert` on first launch offers it to anyone who never opens Settings.

**Tech Stack:** Swift 6, SwiftUI, `FileManager` (no shelling out, no new process, no entitlements — the app carries no `.entitlements` file today).

## Global Constraints

- Spec: `docs/superpowers/specs/2026-08-04-cli-tools-install-design.md` (source of truth for anything this plan doesn't spell out).
- `apps/macos` has **no XCTest target** (`Package.swift` defines only the `"Far Cooler"` executable target). Verification here follows the app's existing probe pattern (`FARCOOLER_SERVICE_PROBE` in `FarCoolerApp.swift` / `ServiceRegistration.swift`): a `FARCOOLER_CLI_TOOLS_PROBE={install,uninstall,refresh}` env var runs the action headlessly against the real built bundle and prints the resulting state. Do not add a new test target — that would be new infrastructure the codebase doesn't otherwise have, for one class.
- `farcooler` and `farcoolerd` are tracked and acted on **as a pair**, never independently.
- Never overwrite or delete anything at `~/.local/bin/farcooler(d)` that isn't already a symlink to this app's own bundled binary. A conflict is reported, never resolved automatically.
- Target directory: `~/.local/bin` (`FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent(".local/bin")`), matching what `docs/remote-hosts.md` already documents.
- Bundled binary location: `Bundle.main.bundleURL.appendingPathComponent("Contents/Resources").appendingPathComponent(name)` — confirmed by inspecting the installed app (`Contents/Resources/farcooler`, `Contents/Resources/farcoolerd`).
- Follow existing copy style: plain prose in UI strings, no Markdown/backtick formatting (none of the existing Settings captions use it).

---

### Task 1: `CommandLineTools` + probe

**Files:**
- Create: `apps/macos/Sources/FarCooler/CommandLineTools.swift`
- Modify: `apps/macos/Sources/FarCooler/FarCoolerApp.swift:40-51` (the `Entry.main()` enum)

**Interfaces:**
- Produces: `@MainActor final class CommandLineTools: ObservableObject` with `@Published private(set) var state: State`, `enum State: Equatable { case notInstalled, installed, conflict(String), unavailable(String) }`, `init()` (calls `refresh()`), `func refresh()`, `func install()`, `func uninstall()`. Tasks 2 and 3 construct it with `CommandLineTools()` and read `.state`.

- [ ] **Step 1: Write `CommandLineTools.swift`**

```swift
import Foundation

/// Symlinking the app's own bundled `farcooler` and `farcoolerd` into
/// `~/.local/bin`, so a shell — including one an SSH session execs on this
/// Mac — can find them.
///
/// Without this, "run the app once" (see docs/remote-hosts.md) is true for
/// the login-item daemon and false for everything else: an SSH-invoked shell
/// has no idea either binary exists, `farcoolerd --stdio` finds nothing, and
/// the client reports the closed pipe as "did not answer" — indistinguishable,
/// from the wire, from a hung daemon.
@MainActor
final class CommandLineTools: ObservableObject {
    static let binaryNames = ["farcooler", "farcoolerd"]

    enum State: Equatable {
        case notInstalled
        case installed
        /// Something at this path is not a symlink to this app's own binary —
        /// a real file, or a symlink pointing somewhere else. Named so the
        /// user can go look, rather than something this app will overwrite.
        case conflict(String)
        case unavailable(String)
    }

    @Published private(set) var state: State = .notInstalled

    private enum SlotState: Equatable {
        case missing
        case ours
        case conflict(String)
    }

    private var localBinDirectory: URL {
        FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent(".local/bin")
    }

    private func localBinURL(for name: String) -> URL {
        localBinDirectory.appendingPathComponent(name)
    }

    private func bundledBinaryURL(for name: String) -> URL {
        Bundle.main.bundleURL
            .appendingPathComponent("Contents/Resources")
            .appendingPathComponent(name)
    }

    init() {
        refresh()
    }

    /// What's actually on disk, in the user's terms.
    func refresh() {
        // Mirrors ServiceRegistration's identical guard: a bare executable has
        // no Contents/Resources to point a symlink at.
        guard Bundle.main.bundleURL.pathExtension == "app" else {
            state = .unavailable("Run Far Cooler from the app bundle to enable this.")
            return
        }

        let slots = Self.binaryNames.map(slotState(for:))
        let conflicts = slots.compactMap { slot -> String? in
            if case .conflict(let path) = slot { return path }
            return nil
        }

        if let path = conflicts.first {
            state = .conflict("\(path) already exists and isn't managed by Far Cooler.")
        } else if slots.allSatisfy({ $0 == .ours }) {
            state = .installed
        } else {
            state = .notInstalled
        }
    }

    /// Symlink both binaries in. Refuses outright if either slot is a
    /// conflict — this never overwrites a path it did not create.
    func install() {
        if case .conflict = state { return }

        do {
            try FileManager.default.createDirectory(
                at: localBinDirectory, withIntermediateDirectories: true)
            for name in Self.binaryNames {
                let link = localBinURL(for: name).path
                // A symlink already there (e.g. a previous install) is
                // replaced; nothing else reaches this point, since a real
                // file would have read as `.conflict` above and returned already.
                if (try? FileManager.default.destinationOfSymbolicLink(atPath: link)) != nil {
                    try FileManager.default.removeItem(atPath: link)
                }
                try FileManager.default.createSymbolicLink(
                    atPath: link, withDestinationPath: bundledBinaryURL(for: name).path)
            }
        } catch {
            state = .unavailable((error as NSError).localizedDescription)
            return
        }
        refresh()
    }

    /// Remove only the symlinks this app owns. A conflicting path is left
    /// exactly as `install()` would have left it: untouched.
    func uninstall() {
        for name in Self.binaryNames where slotState(for: name) == .ours {
            try? FileManager.default.removeItem(atPath: localBinURL(for: name).path)
        }
        refresh()
    }

    private func slotState(for name: String) -> SlotState {
        let path = localBinURL(for: name).path
        // Read as a symlink first: this also catches a *dangling* symlink
        // (target currently missing), which `fileExists` alone would follow
        // through and misreport as "missing" rather than "ours but broken".
        if let destination = try? FileManager.default.destinationOfSymbolicLink(atPath: path) {
            return destination == bundledBinaryURL(for: name).path ? .ours : .conflict(path)
        }
        return FileManager.default.fileExists(atPath: path) ? .conflict(path) : .missing
    }
}

/// Drive install/uninstall from the command line, for checking it without a
/// window.
///
///     FARCOOLER_CLI_TOOLS_PROBE=install './Far Cooler.app/Contents/MacOS/Far Cooler'
@MainActor
enum CLIToolsProbe {
    static func run(_ action: String) -> Never {
        let tools = CommandLineTools()
        switch action {
        case "install": tools.install()
        case "uninstall": tools.uninstall()
        default: break
        }
        tools.refresh()
        print("bundle: \(Bundle.main.bundleURL.path)")
        print("state:  \(tools.state)")
        exit(0)
    }
}
```

- [ ] **Step 2: Wire the probe into `Entry.main()`**

In `apps/macos/Sources/FarCooler/FarCoolerApp.swift`, the `Entry` enum currently reads:

```swift
@main
enum Entry {
    static func main() {
        if let path = ProcessInfo.processInfo.environment["FARCOOLER_RENDER_PROBE"] {
            MainActor.assumeIsolated { RenderProbe.run(writingTo: path) }
        }
        if let action = ProcessInfo.processInfo.environment["FARCOOLER_SERVICE_PROBE"] {
            MainActor.assumeIsolated { ServiceProbe.run(action) }
        }
        FarCoolerApp.main()
    }
}
```

Add a third check, in the same style, right after the `FARCOOLER_SERVICE_PROBE` block:

```swift
        if let action = ProcessInfo.processInfo.environment["FARCOOLER_CLI_TOOLS_PROBE"] {
            MainActor.assumeIsolated { CLIToolsProbe.run(action) }
        }
```

- [ ] **Step 3: Build**

Run: `cd apps/macos && ./build-app.sh`
Expected: ends with `Built build/Far Cooler.app` and no Swift compiler errors. If it fails on `CommandLineTools.swift`, fix the reported line before continuing — there is no separate "compile check" step here since `swift build` inside `build-app.sh` is the compile check.

- [ ] **Step 4: Run the install probe and verify state + real symlinks**

Run:
```bash
FARCOOLER_CLI_TOOLS_PROBE=install "apps/macos/build/Far Cooler.app/Contents/MacOS/Far Cooler"
```
Expected output: `state:  installed`

Then verify the actual filesystem effect:
```bash
ls -la ~/.local/bin/farcooler ~/.local/bin/farcoolerd
readlink ~/.local/bin/farcooler
readlink ~/.local/bin/farcoolerd
```
Expected: both `readlink` outputs equal `<repo>/apps/macos/build/Far Cooler.app/Contents/Resources/farcooler` and `.../farcoolerd` respectively.

- [ ] **Step 5: Run the probe again (idempotency) and then uninstall**

Run:
```bash
FARCOOLER_CLI_TOOLS_PROBE=install "apps/macos/build/Far Cooler.app/Contents/MacOS/Far Cooler"
```
Expected: `state:  installed` again, no error — re-running install on an already-correct install is a no-op modulo re-linking.

Run:
```bash
FARCOOLER_CLI_TOOLS_PROBE=uninstall "apps/macos/build/Far Cooler.app/Contents/MacOS/Far Cooler"
ls ~/.local/bin/farcooler 2>&1
```
Expected: probe prints `state:  notInstalled`; the `ls` reports `No such file or directory`.

- [ ] **Step 6: Verify conflict detection manually**

Run:
```bash
mkdir -p ~/.local/bin
touch ~/.local/bin/farcooler
FARCOOLER_CLI_TOOLS_PROBE=install "apps/macos/build/Far Cooler.app/Contents/MacOS/Far Cooler"
```
Expected: `state:  conflict("/Users/<you>/.local/bin/farcooler already exists and isn\'t managed by Far Cooler.")` (exact punctuation of Swift's `Equatable` enum printing may vary slightly — the important thing is the message text and that `~/.local/bin/farcooler` was NOT replaced: `file ~/.local/bin/farcooler` should still report a regular empty file, not a symlink).

Clean up before moving on:
```bash
rm ~/.local/bin/farcooler
FARCOOLER_CLI_TOOLS_PROBE=install "apps/macos/build/Far Cooler.app/Contents/MacOS/Far Cooler"
```
Expected: back to `state:  installed`, ready for Task 4's end-to-end check.

- [ ] **Step 7: Commit**

```bash
git add apps/macos/Sources/FarCooler/CommandLineTools.swift apps/macos/Sources/FarCooler/FarCoolerApp.swift
git commit -m "$(cat <<'EOF'
feat(macos): symlink bundled farcooler/farcoolerd into ~/.local/bin

Closes the gap where "run the app once" only made the login-item
daemon reachable, not the CLI on PATH -- so an SSH session onto this
Mac couldn't exec farcoolerd and the client reported it as "did not
answer". Verified via the app's existing probe pattern
(FARCOOLER_CLI_TOOLS_PROBE), since apps/macos has no XCTest target.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 2: Settings row

**Files:**
- Modify: `apps/macos/Sources/FarCooler/Preferences.swift:181-247` (`SettingsView` struct and its `host` computed property)

**Interfaces:**
- Consumes: `CommandLineTools` (Task 1) — `ObservableObject`, `.state: CommandLineTools.State`, `.install()`, `.uninstall()`, `.refresh()`.

- [ ] **Step 1: Add the `@StateObject` alongside the existing one**

In `Preferences.swift`, `SettingsView` currently declares:

```swift
struct SettingsView: View {
    @ObservedObject private var preferences = Preferences.shared
    @StateObject private var service = ServiceRegistration()
```

Change to:

```swift
struct SettingsView: View {
    @ObservedObject private var preferences = Preferences.shared
    @StateObject private var service = ServiceRegistration()
    @StateObject private var cliTools = CommandLineTools()
```

- [ ] **Step 2: Add the row to the `host` view**

The `host` property currently ends with:

```swift
    private var host: some View {
        Form {
            Setting("Keeps this Mac reachable from your iPhone while Far Cooler is closed.") {
                switch service.state {
                case .registered:
                    Toggle("Start the daemon at login", isOn: .constant(true))
                        .onTapGesture { service.unregister() }
                case .notRegistered:
                    Toggle("Start the daemon at login", isOn: .constant(false))
                        .onTapGesture { service.register() }
                case .awaitingApproval:
                    Button("Approve in System Settings") { service.register() }
                case .unavailable(let why):
                    Text(why).font(.callout).foregroundStyle(.secondary)
                }
            }
        }
        .formStyle(.grouped)
        .padding()
        .onAppear { service.refresh() }
    }
```

Replace it with (new `Setting` row added inside the same `Form`, and `cliTools.refresh()` added to `.onAppear`):

```swift
    private var host: some View {
        Form {
            Setting("Keeps this Mac reachable from your iPhone while Far Cooler is closed.") {
                switch service.state {
                case .registered:
                    Toggle("Start the daemon at login", isOn: .constant(true))
                        .onTapGesture { service.unregister() }
                case .notRegistered:
                    Toggle("Start the daemon at login", isOn: .constant(false))
                        .onTapGesture { service.register() }
                case .awaitingApproval:
                    Button("Approve in System Settings") { service.register() }
                case .unavailable(let why):
                    Text(why).font(.callout).foregroundStyle(.secondary)
                }
            }

            Setting("Puts farcooler and farcoolerd on your PATH, so a terminal or an SSH session can find them.") {
                switch cliTools.state {
                case .installed:
                    Toggle("Command-line tools", isOn: .constant(true))
                        .onTapGesture { cliTools.uninstall() }
                case .notInstalled:
                    Toggle("Command-line tools", isOn: .constant(false))
                        .onTapGesture { cliTools.install() }
                case .conflict(let why), .unavailable(let why):
                    Text(why).font(.callout).foregroundStyle(.secondary)
                }
            }
        }
        .formStyle(.grouped)
        .padding()
        .onAppear {
            service.refresh()
            cliTools.refresh()
        }
    }
```

- [ ] **Step 3: Build**

Run: `cd apps/macos && ./build-app.sh`
Expected: `Built build/Far Cooler.app`, no errors. (`Setting` is `private` to this file, which is fine — the new row is declared inside the same file.)

- [ ] **Step 4: Manual visual check**

Run: `open "apps/macos/build/Far Cooler.app"`, then open Settings (⌘,) → Startup tab.
Expected: two rows now visible — "Start the daemon at login" and "Command-line tools" — the second one reading **on** (Task 1's Step 6 left the real `~/.local/bin` in the `installed` state). Toggle it off, confirm the row switches to off and `ls ~/.local/bin/farcooler` (in a terminal) reports missing; toggle back on, confirm it returns and the symlink reappears.

- [ ] **Step 5: Commit**

```bash
git add apps/macos/Sources/FarCooler/Preferences.swift
git commit -m "$(cat <<'EOF'
feat(macos): add command-line tools toggle to Startup settings

Same tab as the login-item toggle -- both answer "is this Mac
reachable when nobody is at it," just through different mechanisms
(SMAppService vs. two symlinks), which is why they're separate rows
rather than one combined toggle.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 3: One-time launch prompt

**Files:**
- Modify: `apps/macos/Sources/FarCooler/FarCoolerApp.swift:1-33` (the `FarCoolerApp` struct)

**Interfaces:**
- Consumes: `CommandLineTools` (Task 1) — same interface as Task 2.
- Uses `UserDefaults.standard` key `"hasPromptedCLIToolsInstall"` (`Bool`). This key is local to this task; nothing else reads or writes it.

- [ ] **Step 1: Add the state, the prompt trigger, and the alert**

`FarCoolerApp.swift` currently declares:

```swift
struct FarCoolerApp: App {
    /// Present only to catch the APNs device token, which arrives nowhere else.
    @NSApplicationDelegateAdaptor(PushDelegate.self) private var pushDelegate

    var body: some Scene {
        WindowGroup {
            ContentView()
                .onAppear { Appearance.apply(Preferences.shared.appearance) }
                .frame(minWidth: 600, minHeight: 400)
        }
        .windowStyle(.titleBar)
        .commands { FarCoolerCommands() }

        Settings { SettingsView() }
    }
}
```

Replace with:

```swift
struct FarCoolerApp: App {
    /// Present only to catch the APNs device token, which arrives nowhere else.
    @NSApplicationDelegateAdaptor(PushDelegate.self) private var pushDelegate
    @State private var showsCLIToolsPrompt = false

    var body: some Scene {
        WindowGroup {
            ContentView()
                .onAppear {
                    Appearance.apply(Preferences.shared.appearance)
                    promptForCLIToolsIfNeeded()
                }
                .frame(minWidth: 600, minHeight: 400)
                .alert("Install command-line tools?", isPresented: $showsCLIToolsPrompt) {
                    Button("Install") { CommandLineTools().install() }
                    Button("Not Now", role: .cancel) {}
                } message: {
                    Text(
                        "Lets your terminal and SSH sessions find farcooler. "
                            + "You can always do this later in Settings."
                    )
                }
        }
        .windowStyle(.titleBar)
        .commands { FarCoolerCommands() }

        Settings { SettingsView() }
    }

    /// Fires at most once, ever, per machine. The flag is set the moment the
    /// decision to show is made — not from inside the button actions — so
    /// quitting the app with the alert still on screen doesn't leave it
    /// primed to reappear next launch.
    private func promptForCLIToolsIfNeeded() {
        let key = "hasPromptedCLIToolsInstall"
        guard !UserDefaults.standard.bool(forKey: key) else { return }

        let tools = CommandLineTools()
        guard tools.state == .notInstalled else { return }

        UserDefaults.standard.set(true, forKey: key)
        showsCLIToolsPrompt = true
    }
}
```

- [ ] **Step 2: Build**

Run: `cd apps/macos && ./build-app.sh`
Expected: `Built build/Far Cooler.app`, no errors.

- [ ] **Step 3: Verify the prompt fires when not installed, and only once**

```bash
defaults delete com.farcooler.FarCooler hasPromptedCLIToolsInstall 2>/dev/null
FARCOOLER_CLI_TOOLS_PROBE=uninstall "apps/macos/build/Far Cooler.app/Contents/MacOS/Far Cooler"
open "apps/macos/build/Far Cooler.app"
```
Expected: within a moment of the main window appearing, the "Install command-line tools?" alert shows. Click **Install**.
Verify: `ls ~/.local/bin/farcooler` now succeeds (symlinked).
Quit and relaunch (`open "apps/macos/build/Far Cooler.app"` again) — expected: no alert this time, even though nothing new was installed.

(`com.farcooler.FarCooler` is this app's `CFBundleIdentifier`, confirmed from `Contents/Info.plist`. If `defaults delete` reports "does not exist," that's fine — it just means this is the first run of the check.)

- [ ] **Step 4: Commit**

```bash
git add apps/macos/Sources/FarCooler/FarCoolerApp.swift
git commit -m "$(cat <<'EOF'
feat(macos): prompt once to install command-line tools

Catches the person who never opens Settings: on first launch, if the
CLI tools aren't on PATH yet, offer to install them right there. Fires
at most once ever, tracked by a UserDefaults flag set the instant the
decision to show is made -- not from the button actions -- so quitting
mid-alert can't leave it primed to reappear.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 4: Build, install, verify end-to-end, push

**Files:** none (build/verification only).

- [ ] **Step 1: Confirm the working tree is clean for this feature's files**

```bash
git status --short apps/macos/Sources/FarCooler/CommandLineTools.swift \
  apps/macos/Sources/FarCooler/FarCoolerApp.swift \
  apps/macos/Sources/FarCooler/Preferences.swift
```
Expected: no output — everything from Tasks 1-3 is already committed.

- [ ] **Step 2: Rebuild release and replace the installed app**

```bash
cd apps/macos
./build-app.sh release
pkill -x "Far Cooler" 2>/dev/null || true
rm -rf "/Applications/Far Cooler.app"
cp -R "build/Far Cooler.app" "/Applications/Far Cooler.app"
open "/Applications/Far Cooler.app"
```
Expected: the app launches. Since `~/.local/bin/farcooler(d)` are already installed from Task 1/3's manual checks (pointing at `apps/macos/build/Far Cooler.app/Contents/Resources/...`), reinstalling to `/Applications` leaves those symlinks pointing at the *build* directory, not `/Applications` — the last step below fixes that.

- [ ] **Step 3: Re-point the real install at `/Applications` and do a full end-to-end check**

Open Settings (⌘,) → Startup tab. If "Command-line tools" reads on, toggle it off then on again — this re-creates the symlinks against `Bundle.main`, which for the launched-from-`/Applications` copy now resolves to `/Applications/Far Cooler.app/Contents/Resources/...`.

Verify:
```bash
readlink ~/.local/bin/farcooler
readlink ~/.local/bin/farcoolerd
```
Expected: both point into `/Applications/Far Cooler.app/Contents/Resources/`.

```bash
PATH="$HOME/.local/bin:$PATH" which farcoolerd
PATH="$HOME/.local/bin:$PATH" farcoolerd --version
```
Expected: resolves to `~/.local/bin/farcoolerd` and prints a version, confirming a fresh shell (the kind SSH execs) can now find it.

If Remote Login is on (enabled earlier this session) and the phone's key is already authorized, this is also a good moment to retry the phone's SSH connection from the FarCooler iOS app and confirm the daemon answers instead of erroring.

- [ ] **Step 4: Push**

```bash
git log --oneline origin/main..HEAD
git push origin main
```
Expected: the four commits from this plan (Task 1's, Task 2's, Task 3's, plus the two design-doc commits from earlier) appear in the pre-push log, and the push succeeds.
