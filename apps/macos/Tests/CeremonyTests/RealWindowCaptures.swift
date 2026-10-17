import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// The real window, captured (ov-278): `ContentView` itself, in a real titled
/// window off every screen, against the seeded scratch daemon
/// `scripts/mac-capture.sh` starts, one PNG per place and appearance.
///
/// Opt-in, and never in CI: enabled only by `FARCOOLER_CAPTURE_OUT`, which the
/// script sets. It refuses to run unless `FARCOOLER_HOME` and `FARCOOLER_BIN`
/// both name the scratch daemon, so a capture can never open on the fleet the
/// person at this Mac is using. Nothing here sends input: each place is
/// reached through the window's own launch, by writing where the window was
/// as the app records it, so what's drawn is what a relaunch would draw.
///
/// See ui-lane-common.md, "Capturing the real window".
@MainActor
struct RealWindowCaptures {
    nonisolated static let environment = ProcessInfo.processInfo.environment

    /// One place to capture: a file name, and what the window reopens to, as
    /// `SelectionMemory.key` stores it. Nil opens where the launch rule says,
    /// which is Needs You while anything waits. `width`, for a place drawn at
    /// a width of its own, whatever `FARCOOLER_CAPTURE_WIDTH` says.
    struct Place: Equatable {
        let name: String
        let selection: String?
        var width: Double? = nil
    }

    /// The width a navigator place is drawn at (ov-600). Under
    /// `WorkspaceColumns.navigatorFloatsBelow` (1500 pt) a workspace with an
    /// orchestrator floats its navigator over the canvas, put away until ⌘B,
    /// so at the default 1360 no capture had one. At 1500 the task beside it
    /// is too narrow for its own header; 1800 draws both whole.
    static let navigatorWidth: Double = 1800

    /// The places the seed has, by its ids: Needs You, the board, a task, the
    /// worktree's tiled pair of shells (the second sits beside the worktree's
    /// own), and History, then the agent's worktree.
    static func places(seed: [String: String]) -> [Place] {
        let ws = seed["workspace"] ?? "", wt = seed["worktree"] ?? ""
        var places = [
            Place(name: "needs-you", selection: nil),
            Place(name: "board", selection: "|\(ws)|"),
            Place(name: "terminals", selection: "|\(ws)|worktree:\(wt):\(seed["second"] ?? "")"),
            Place(name: "history", selection: "|\(ws)|history:done"),
        ]
        if let task = seed["task"] { places.insert(Place(name: "task", selection: "|\(ws)|task:\(task)"), at: 2) }
        // The agent at work (ov-203). Its pane works a task, so it lands on
        // that task's Agent tab, as choosing the agent does
        // (`WorkspaceSelection.landing`); and with the navigator beside it.
        // Its worktree opened whole, with no pane named, is where Files sits
        // beside its terminals: on the task, Files is a tab instead (ov-600).
        if let agentWT = seed["agentWorktree"], let agent = seed["agent"] {
            let selection = "|\(ws)|worktree:\(agentWT):\(agent)"
            places.append(Place(name: "agent", selection: selection))
            places.append(Place(name: "files-inspector-agent", selection: "|\(ws)|worktree:\(agentWT)"))
            places.append(Place(name: "navigator-agent", selection: selection, width: navigatorWidth))
        }
        return places
    }

    /// `FARCOOLER_CAPTURE_PLACES`: one `name=<selection>` per line, for a
    /// place the seed doesn't name. `name=` alone is the launch rule's place.
    static func extraPlaces(_ text: String?) -> [Place] {
        (text ?? "").split(whereSeparator: \.isNewline).compactMap { line in
            let parts = line.split(separator: "=", maxSplits: 1, omittingEmptySubsequences: false)
            guard parts.count == 2, !parts[0].isEmpty else { return nil }
            return Place(name: String(parts[0]), selection: parts[1].isEmpty ? nil : String(parts[1]))
        }
    }

    /// `FARCOOLER_CAPTURE_DEFAULTS`: one `key=true|false` per line, set for
    /// every place and put back afterwards: a navigator section closed, say,
    /// as `navigator.closed.local.<workspace>.themes=true`.
    static func extraDefaults(_ text: String?) -> [String: Bool] {
        var out: [String: Bool] = [:]
        for line in (text ?? "").split(whereSeparator: \.isNewline) {
            let parts = line.split(separator: "=", maxSplits: 1)
            guard parts.count == 2, let value = Bool(String(parts[1])) else { continue }
            out[String(parts[0])] = value
        }
        return out
    }

    /// Each appearance a place is drawn in, by the suffix its file takes.
    static let variants: [(name: String, appearance: NSAppearance.Name)] = [
        ("light", .aqua), ("dark", .darkAqua),
        ("light-contrast", .accessibilityHighContrastAqua), ("dark-contrast", .accessibilityHighContrastDarkAqua),
    ]

    @Test(.enabled(if: RealWindowCaptures.environment["FARCOOLER_CAPTURE_OUT"] != nil))
    func capture() async throws {
        let env = Self.environment
        // The scratch daemon, or nothing: never the one this Mac's person uses.
        let home = try #require(env["FARCOOLER_HOME"], "FARCOOLER_HOME must name the scratch daemon")
        try #require(home.hasPrefix("/tmp/") || home.hasPrefix("/private/tmp/"), "FARCOOLER_HOME isn't a scratch home: \(home)")
        try #require(env["FARCOOLER_BIN"] != nil, "FARCOOLER_BIN must name this checkout's CLI")
        unsetenv("FARCOOLER_WORKSPACE")

        let out = URL(fileURLWithPath: try #require(env["FARCOOLER_CAPTURE_OUT"]))
        let stage = env["FARCOOLER_CAPTURE_STAGE"] ?? "capture"
        let wait = Double(env["FARCOOLER_CAPTURE_WAIT"] ?? "") ?? 5
        let seed: [String: String] =
            env["FARCOOLER_CAPTURE_SEED"]
            .flatMap { FileManager.default.contents(atPath: $0) }
            .flatMap { try? JSONSerialization.jsonObject(with: $0) as? [String: String] } ?? [:]
        let wanted = env["FARCOOLER_CAPTURE_ONLY"]
        let places = (Self.places(seed: seed) + Self.extraPlaces(env["FARCOOLER_CAPTURE_PLACES"]))
            .filter { wanted == nil || $0.name == wanted }
        try #require(!places.isEmpty, "no place to capture")

        // What every local test run shares (the xctest domain): put back as
        // it was when the captures are done (review L9).
        let defaults = UserDefaults.standard
        let extra = Self.extraDefaults(env["FARCOOLER_CAPTURE_DEFAULTS"])
        let touched =
            ["window.sessions.v1", SelectionMemory.destinationKey, SelectionMemory.key, "hosts.configured"]
            + extra.keys.sorted()
        let saved = Dictionary(uniqueKeysWithValues: touched.map { ($0, defaults.object(forKey: $0)) })
        defer { Self.restore(saved, in: defaults) }
        let sessions = WindowSessions.shared
        defer { WindowSessions.shared = sessions }
        for (key, value) in extra { defaults.set(value, forKey: key) }
        // `FARCOOLER_CAPTURE_RUNNERS`: runners configured by target, one per
        // line, before the window opens ("carl.invalid" is never reachable, so
        // the toolbar says "carl.invalid offline"). Read once by `Runners`, so
        // set before anything touches it.
        if let runners = env["FARCOOLER_CAPTURE_RUNNERS"] {
            let list = runners.split(whereSeparator: \.isNewline).map { ["target": String($0)] }
            defaults.set(try JSONSerialization.data(withJSONObject: list), forKey: "hosts.configured")
        }
        // `FARCOOLER_CAPTURE_HOVER`: jump bar pieces drawn as if the pointer
        // were on them, by name, one per line ("Go to Billing"), no input sent.
        JumpBarHover.forced = Set((env["FARCOOLER_CAPTURE_HOVER"] ?? "").split(whereSeparator: \.isNewline).map(String.init))
        defer { JumpBarHover.forced = [] }
        // A narrow window, for the layouts that degrade (`FARCOOLER_CAPTURE_WIDTH`).
        let defaultWidth = Double(env["FARCOOLER_CAPTURE_WIDTH"] ?? "") ?? 1360
        let height = Double(env["FARCOOLER_CAPTURE_HEIGHT"] ?? "") ?? 860
        for place in places {
            let width = place.width ?? defaultWidth
            // Where the window was, as a relaunch reads it, and nothing else:
            // no window record, no newer destination.
            defaults.removeObject(forKey: "window.sessions.v1")
            // And none held in memory: closing the last window keeps its
            // record, so the next would adopt the last place's frame (ov-600),
            // a navigator place at 1800 drawn at 1360.
            WindowSessions.shared = WindowSessions()
            defaults.removeObject(forKey: SelectionMemory.destinationKey)
            if let selection = place.selection, selection.hasPrefix("{") {
                // A whole `Destination`, for a task's tab (ov-297).
                defaults.set(selection, forKey: SelectionMemory.destinationKey)
                defaults.removeObject(forKey: SelectionMemory.key)
            } else if let selection = place.selection {
                defaults.set(selection, forKey: SelectionMemory.key)
            } else {
                defaults.removeObject(forKey: SelectionMemory.key)
            }
            let window = try await TitleBarHarness.window(ContentView().captureContrast(), width: width, height: height)
            defer { CaptureContrast.shared.forced = nil }
            try await Task.sleep(for: .seconds(wait))
            // A plan page isn't kept across a relaunch (it reopens as its
            // workspace), so the window is sent there as a click would.
            if let selection = place.selection, selection.contains("|plan:") {
                NotificationCenter.default.post(name: .captureOpen, object: selection)
                try await Task.sleep(for: .seconds(wait))
            }
            // `FARCOOLER_CAPTURE_PEEK`: the plan peeked over the chat, as
            // ⌥⌘P does in a window too narrow for the canvas (ov-298).
            if env["FARCOOLER_CAPTURE_PEEK"] != nil {
                NotificationCenter.default.post(name: .captureOpen, object: "peek")
                try await Task.sleep(for: .seconds(1))
            }
            // `FARCOOLER_CAPTURE_FOLD=collapse|expand`: View > Collapse All or
            // Expand All, as the menu asks, with no input (ov-334).
            if let fold = env["FARCOOLER_CAPTURE_FOLD"] {
                NotificationCenter.default.post(name: .captureOpen, object: "fold:\(fold)")
                try await Task.sleep(for: .seconds(1))
            }
            // `FARCOOLER_CAPTURE_THEN`: more places or folds, one per line,
            // posted in order after the first, as clicks would (ov-476).
            for step in (env["FARCOOLER_CAPTURE_THEN"] ?? "").split(whereSeparator: \.isNewline) {
                NotificationCenter.default.post(name: .captureOpen, object: String(step))
                try await Task.sleep(for: .seconds(1))
            }
            // `FARCOOLER_CAPTURE_FOCUS`: the app active and this window key, as
            // in use, so the glass (the title bar, the tray, the composer) is
            // drawn as an active window's, not an inactive one's (ov-476).
            if env["FARCOOLER_CAPTURE_FOCUS"] != nil {
                NSApp.setActivationPolicy(.regular)
                // Activation is an event the app has to be run to receive, and
                // this process isn't inside `NSApp.run`: run it for a moment.
                // While it runs, swallow the keyboard: the window is key, so a
                // key the person types elsewhere would land in this one.
                let swallow = NSEvent.addLocalMonitorForEvents(matching: [.keyDown, .keyUp, .flagsChanged]) { _ in nil }
                defer { if let swallow { NSEvent.removeMonitor(swallow) } }
                NSApp.activate(ignoringOtherApps: true) // window-exempt: FARCOOLER_CAPTURE_FOCUS, set only by the stage-demo capture, draws active glass (ov-476)
                window.makeKeyAndOrderFront(nil) // window-exempt: as above; no test sets FARCOOLER_CAPTURE_FOCUS (ov-476)
                Timer.scheduledTimer(withTimeInterval: 0.7, repeats: false) { _ in
                    NSApp.stop(nil)
                    NSApp.postEvent(
                        NSEvent.otherEvent(
                            with: .applicationDefined, location: .zero, modifierFlags: [], timestamp: 0,
                            windowNumber: 0, context: nil, subtype: 0, data1: 0, data2: 0)!, atStart: true)
                }
                NSApp.run()
                // Activating brought the window onto the screen and may have
                // resized it: put it back where, and as big as, it was, off every
                // screen, and take the composer out of the way of real keystrokes.
                window.setContentSize(NSSize(width: width, height: height))
                window.setFrameOrigin(NSPoint(x: -6000, y: -6000))
                window.makeFirstResponder(nil)
                try await Task.sleep(for: .milliseconds(300))
                print("capture: app active \(NSApp.isActive), window key \(window.isKeyWindow)")
            }
            // Files beside the worktree, opened as a clicked path opens them:
            // the agent's on the file it changed.
            if place.name.hasPrefix("files-inspector"),
                let worktree = place.name.hasSuffix("-agent") ? seed["agentWorktree"].map({ "\($0):src/Invoice.swift" }) : seed["worktree"]
            {
                NotificationCenter.default.post(name: .captureFiles, object: worktree)
                try await Task.sleep(for: .seconds(wait))
            }
            try await TitleBarHarness.settle(window)
            if let key = env["FARCOOLER_CAPTURE_KEY_HOVER"] { try await Self.hoverKey(key, in: window) }
            defer { TaskKeyHoverForcing.key = nil }
            for variant in Self.variants {
                window.appearance = NSAppearance(named: variant.appearance)
                // Increase Contrast as SwiftUI reads it: a high-contrast
                // appearance alone never reached `colorSchemeContrast`, and
                // every "-contrast" file came out the same as its twin.
                CaptureContrast.shared.forced = variant.name.hasSuffix("-contrast") ? .increased : .standard
                try await TitleBarHarness.settle(window)
                let view = try #require(window.contentView?.superview)
                // One way to render (LookBitmap.swift), at the window's own scale.
                // `FARCOOLER_CAPTURE_SCALE=1`: the same views drawn at CI's 1x,
                // on a 2x Mac, to see where a rule or a glyph lands a pixel off.
                let scale = Int(env["FARCOOLER_CAPTURE_SCALE"] ?? "") ?? max(1, Int(window.backingScaleFactor))
                let drawn = Self.withPopovers(try #require(view.lookBitmap(scale: scale)), of: window, scale: scale)
                let rep = await Self.withWebPages(drawn, in: view)
                // The size asked for, and round lights, or no file (ov-476).
                let problem = CaptureCheck.problem(rep, expectedPixels: (Int(width) * scale, Int(height) * scale))
                try #require(problem == nil, "\(place.name) \(variant.name): \(problem ?? "")")
                let png = try #require(rep.representation(using: .png, properties: [:]))
                try png.write(to: out.appendingPathComponent("\(stage)-\(place.name)-\(variant.name).png"))
            }
            window.close()
        }
    }

    /// Each key back as it was: a value set again, an absent one removed.
    static func restore(_ saved: [String: Any?], in defaults: UserDefaults) {
        for (key, value) in saved {
            if let value { defaults.set(value, forKey: key) } else { defaults.removeObject(forKey: key) }
        }
    }

    @Test func theDefaultsComeBackAsTheyWere() throws {
        let suite = "capture-restore-\(UUID().uuidString)"
        let defaults = try #require(ScratchDefaults.suite(suite))
        defaults.set("|kept|", forKey: "a")
        let saved: [String: Any?] = ["a": defaults.object(forKey: "a"), "b": defaults.object(forKey: "b")]
        defaults.set("|capture|", forKey: "a")
        defaults.set("|capture|", forKey: "b")
        Self.restore(saved, in: defaults)
        #expect(defaults.string(forKey: "a") == "|kept|")
        #expect(defaults.object(forKey: "b") == nil)
    }

    @Test func extraDefaultsReadKeyAndBool() {
        #expect(Self.extraDefaults("a=true\nb=false\nc=maybe\nnoequals") == ["a": true, "b": false])
    }

    static let seed = [
        "workspace": "ws", "worktree": "wt", "second": "t2", "task": "task", "agentWorktree": "awt", "agent": "agent",
    ]

    /// A place drawn for its navigator has one beside what's open, not
    /// floated and put away (ov-600): the same rule `WorkspaceView` draws
    /// by, at the place's width, with the orchestrator's chat and the canvas.
    @Test func navigatorPlacesDrawTheNavigator() throws {
        let wide = Self.places(seed: Self.seed).filter { $0.width != nil }
        #expect(wide.map(\.name) == ["navigator-agent"])
        for place in wide {
            let width = try #require(place.width)
            for opened in [false, true] {
                let drawn = WorkspaceColumns.arrangement(
                    width: width, canvas: WorkspaceColumns.Canvas(), opened: opened, hasConversation: true,
                    hasBoard: true, boardExists: true, floating: false, focused: false)
                #expect(drawn.navigator && !drawn.floats, "\(place.name) at \(width) pt: \(drawn)")
            }
        }
        // Why every other place has none: at the default width it floats.
        let narrow = WorkspaceColumns.arrangement(
            width: 1360, canvas: WorkspaceColumns.Canvas(), opened: true, hasConversation: true, hasBoard: true,
            boardExists: true, floating: false, focused: false)
        #expect(!narrow.navigator)
    }

    /// Files beside the agent's worktree: the worktree opened whole, since a
    /// pane named in it lands on the pane's task, where Files is a tab (ov-600).
    @Test func agentFilesOpenTheWorktreeWhole() throws {
        let place = try #require(Self.places(seed: Self.seed).first { $0.name == "files-inspector-agent" })
        let selection = try #require(place.selection.flatMap(SelectionMemory.decode))
        #expect(selection == .workspace(host: "", workspace: "ws", focus: .worktree("awt", terminal: nil)))
    }

    @Test func extraPlacesReadNameAndSelection() {
        #expect(
            Self.extraPlaces("a=|ws|\nneeds=\n=bad\nnoequals") == [
                Place(name: "a", selection: "|ws|"), Place(name: "needs", selection: nil),
            ])
    }
}

