import AgentKit
import AppKit
import SwiftUI
import Testing
import WebKit

@testable import Far_Cooler

/// The first review of web panes (ov-435): what the header says about where
/// a page is, who gets the keyboard, and what a closed pane leaves running.
@MainActor
struct WebPaneReviewTests {
    private static func defaults() -> UserDefaults {
        let name = "WebPaneReviewTests-\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: name)!
        defaults.removePersistentDomain(forName: name)
        return defaults
    }

    /// A window nobody sees, with `view` in it, laid out.
    private static func offscreen(_ view: NSView, size: NSSize = NSSize(width: 480, height: 320)) -> NSWindow {
        let window = NSWindow(
            contentRect: NSRect(origin: NSPoint(x: -4000, y: -4000), size: size), styleMask: [.borderless],
            backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = view
        window.makeKeyAndOrderFront(nil)
        view.layoutSubtreeIfNeeded()
        return window
    }

    /// Poll for `condition` (30 s at most, returning as soon as it holds).
    private static func eventually(_ condition: () -> Bool) async -> Bool {
        for _ in 0..<1_500 {
            if condition() { return true }
            try? await Task.sleep(for: .milliseconds(20))
        }
        return condition()
    }

    // MARK: - H2: the origin

    @Test("The origin is the address WebKit shows: no userinfo, no default port, punycode, lowercase")
    func originReadsTheHost() throws {
        let cases: [(String, String, Bool)] = [
            ("https://github.com/acme", "github.com", true),
            ("HTTPS://GitHub.COM:443/x", "github.com", true),
            ("https://www.google.com@evil.example/signin", "evil.example", true),
            ("https://accounts.google.com:evil.example@evil.example/", "evil.example", true),
            ("https://accounts.google.com.evil.example/", "accounts.google.com.evil.example", true),
            ("https://example.com:8443/", "example.com:8443", true),
            ("http://localhost:3000/x", "localhost:3000", false),
            ("http://example.com:80/", "example.com", false),
            ("https://[::1]:8080/", "[::1]:8080", true),
            ("https://аррӏе.com/", "xn--80ak6aa92e.com", true),
        ]
        for (address, host, secure) in cases {
            let origin = try #require(WebOrigin(URL(string: address)), "\(address)")
            #expect(origin.host == host, "\(address): \(origin.host)")
            #expect(origin.isSecure == secure, "\(address)")
        }
        #expect(WebOrigin(URL(string: "file:///etc/passwd")) == nil)
        #expect(WebOrigin(nil) == nil)
        #expect(try #require(WebOrigin(URL(string: "http://example.com/"))).spoken.hasPrefix("Not secure"))
    }

    /// The test that would have caught it: an open redirect lands on another
    /// site, whose page calls itself Google. WebKit, not a stand-in, reports
    /// both, and the header's model must say evil.example, and say the title
    /// separately.
    @Test("A page that calls itself Google, on another host, shows its real host")
    func theHeaderShowsWhereThePageIs() async throws {
        let model = WebPaneModel(terminal: "origin-1", memory: WebPaneMemory(defaults: Self.defaults()))
        let window = Self.offscreen(NSView())
        defer { window.close() }
        window.contentView?.addSubview(model.webView)
        model.webView.loadHTMLString(
            "<title>Sign in – Google Accounts</title><p>Your session expired",
            baseURL: URL(string: "https://accounts.google.com.evil.example/signin")!)
        let arrived = await Self.eventually { model.origin != nil && model.pageTitle != nil }
        #expect(arrived)
        #expect(model.origin?.host == "accounts.google.com.evil.example")
        #expect(model.pageTitle == "Sign in – Google Accounts", "the title is kept, apart from the origin")
        #expect(model.origin?.host != model.pageTitle)

        // And it is drawn, in a header too narrow for the title: the host is
        // there, first, whole, and the title is what gives way.
        for width in [420.0, 240.0, 170.0] {
            let seen = Seen()
            let host = NSHostingView(
                rootView: WebPaneNavigation(model: model, isFocused: true)
                    .frame(width: width, height: 24, alignment: .leading)
                    .environment(\.gridProbing, true)
                    .overlayPreferenceValue(ProbedViewsKey.self) { probed in
                        GeometryReader { proxy in
                            let _ = seen.views = Dictionary(
                                probed.map { ($0.id, proxy[$0.bounds]) }, uniquingKeysWith: { first, _ in first })
                            Color.clear
                        }
                    })
            let drawn = Self.offscreen(host, size: NSSize(width: width, height: 24))
            defer { drawn.close() }
            let found = await Self.eventually {
                host.layoutSubtreeIfNeeded()
                return seen.views["web-origin-host"] != nil
            }
            #expect(found, "no origin in a header \(width) wide: \(seen.views.keys.sorted())")
            _ = await Self.eventually {
                host.layoutSubtreeIfNeeded()
                return (seen.views["web-origin-host"]?.width ?? 0) >= 100
            }
            guard let origin = seen.views["web-origin-host"], let lock = seen.views["web-origin-lock"] else { continue }
            _ = lock
            #expect(lock.maxX <= origin.minX + 1, "the lock leads the host")
            #expect(origin.maxX <= width + 1, "the host is inside the header")
            #expect(origin.width >= (width >= 240 ? 100 : 90), "\(width): the host isn't squeezed away by the title (\(origin.width))")
            if width < WebPaneNavigation.showsTitle {
                #expect(seen.views["web-origin-title"] == nil, "\(width): a narrow header drops the title, not the host")
            } else if let title = seen.views["web-origin-title"] {
                #expect(title.minX >= origin.maxX - 1, "the title comes after the host")
            }
        }
    }

    final class Seen {
        var views: [String: CGRect] = [:]
    }

    @Test("A sign-in popup is titled with its origin, and says when it isn't secure")
    func aPopupSaysWhereItIs() {
        let secure = WebPopup.titles(for: URL(string: "https://www.google.com@evil.example/"), title: "Sign in – Google Accounts")
        #expect(secure.title == "evil.example")
        #expect(secure.subtitle == "Sign in – Google Accounts")
        let plain = WebPopup.titles(for: URL(string: "http://localhost:3000/"), title: "Dev")
        #expect(plain.title == "Not Secure  localhost:3000")
        #expect(WebPopup.titles(for: nil, title: "Blank").title == "Blank")
    }

    @Test("A web pane's navigator row never names a host it may have left")
    func theNavigatorNamesNoHost() throws {
        let data = Data(
            #"{"id":"w","short":"w","title":"Web","preset":"web","state":"running","epoch":0,"paneMode":"web","webUrl":"https://www.google.com/url?q=https://evil.example/"}"#.utf8)
        let terminal = try JSONDecoder().decode(Terminal.self, from: data)
        #expect(terminal.isWebPane)
        #expect(terminal.label == "Web Page", "the opened host is not where the page is")
        #expect(terminal.headerName == "Web Page")
    }

    // MARK: - H1: the keyboard

    @Test("A web pane in a focused tile does not take the owner's keyboard")
    func aWebPaneNeverTakesTheKeyboard() async throws {
        let model = WebPaneModel(terminal: "focus-1", memory: WebPaneMemory(defaults: Self.defaults()))
        let field = NSTextField(frame: NSRect(x: 0, y: 0, width: 200, height: 24))
        let root = NSView(frame: NSRect(x: 0, y: 0, width: 480, height: 320))
        root.addSubview(field)
        let window = Self.offscreen(root)
        defer { window.close() }
        #expect(window.makeFirstResponder(field), "the owner is typing in a field")
        // Then the page arrives in the layout, as an agent's `layout open-url`
        // makes it, and is redrawn as layout events come.
        // Panes come and go in a layout, so the view is handed one page and
        // then another, and each time SwiftUI updates it with the window there.
        let other = WebPaneModel(terminal: "focus-2", memory: WebPaneMemory(defaults: Self.defaults()))
        let page = NSHostingView(rootView: WebPageView(model: other))
        page.frame = NSRect(x: 0, y: 40, width: 480, height: 280)
        root.addSubview(page)
        for n in 1...6 {
            page.rootView = WebPageView(model: n.isMultiple(of: 2) ? other : model)
            page.layoutSubtreeIfNeeded()
            try await Task.sleep(for: .milliseconds(40))
        }
        #expect(model.webView.superview != nil, "the page is on screen")
        // Still in the field the owner was typing in: the field itself, or the
        // field editor it lends the window while it has the keyboard.
        let responder = window.firstResponder
        let stillTheField = responder === field || (responder as? NSText)?.delegate === field
        #expect(stillTheField, "the page took the keyboard: \(String(describing: responder))")
    }

    // MARK: - M3: the prefix

    private func controlB() throws -> NSEvent {
        try #require(
            NSEvent.keyEvent(
                with: .keyDown, location: .zero, modifierFlags: .control, timestamp: 0, windowNumber: 0,
                context: nil, characters: "\u{02}", charactersIgnoringModifiers: "b", isARepeat: false, keyCode: 11))
    }

    @Test("⌃B in a page reaches the tiling prefix, and an ordinary key reaches the page")
    func thePrefixWorksInAPage() throws {
        let model = WebPaneModel(terminal: "prefix-1", memory: WebPaneMemory(defaults: Self.defaults()))
        #expect(model.webView is PaneWebView)
        PrefixMode.shared.cancel()
        defer { PrefixMode.shared.cancel() }
        model.webView.keyDown(with: try controlB())
        #expect(PrefixMode.shared.armed, "⌃B armed the prefix instead of going to the page")
        PrefixMode.shared.cancel()

        let letter = try #require(
            NSEvent.keyEvent(
                with: .keyDown, location: .zero, modifierFlags: [], timestamp: 0, windowNumber: 0, context: nil,
                characters: "a", charactersIgnoringModifiers: "a", isARepeat: false, keyCode: 0))
        #expect(PrefixMode.shared.handle(letter) == .passThrough)
        #expect(!PrefixMode.shared.armed)
    }

    @Test("⌃B sent to a window whose web page has the keyboard arms the prefix (ov-436)")
    func thePrefixWorksThroughTheWindow() throws {
        let model = WebPaneModel(terminal: "prefix-2", memory: WebPaneMemory(defaults: Self.defaults()))
        let host = NSHostingView(rootView: WebPageView(model: model))
        let window = Self.offscreen(host)
        defer { window.close() }
        host.layoutSubtreeIfNeeded()
        #expect(model.webView.window === window)
        #expect(window.makeFirstResponder(model.webView), "the page has the keyboard")
        PrefixMode.shared.cancel()
        defer { PrefixMode.shared.cancel() }
        let event = try #require(
            NSEvent.keyEvent(
                with: .keyDown, location: .zero, modifierFlags: .control, timestamp: 0,
                windowNumber: window.windowNumber, context: nil, characters: "\u{02}",
                charactersIgnoringModifiers: "b", isARepeat: false, keyCode: 11))
        window.sendEvent(event)
        #expect(PrefixMode.shared.armed, "⌃B went to the page, not the layout")
    }

    // MARK: - ov-436: links to other apps

    @Test("A clicked mailto:, linear:// or notion:// link goes to the system; the main frame stays on http and https")
    func appLinksGoToTheSystem() {
        for link in ["mailto:a@b.c", "linear://acme/issue/ENG-1", "notion://www.notion.so/x", "slack://open"] {
            let url = URL(string: link)
            #expect(WebPaneModel.route(url, mainFrame: true, clicked: true) == .app, "\(link)")
            #expect(WebPaneModel.route(url, mainFrame: true, clicked: false) == .refused, "\(link) without a click")
        }
        for link in ["file:///Applications/Calculator.app", "javascript:alert(1)", "data:text/html,hi", "ssh://h", "vnc://h"] {
            #expect(WebPaneModel.route(URL(string: link), mainFrame: true, clicked: true) == .refused, "\(link)")
        }
        #expect(WebPaneModel.route(URL(string: "https://notion.so/x"), mainFrame: true, clicked: true) == .page)
        #expect(WebPaneModel.route(URL(string: "linear://x"), mainFrame: false, clicked: false) == .page, "a frame's own")
    }

    @Test("A link click in a real page asks first, then opens; a redirect does not")
    func aClickedLinkOpensItsApp() async throws {
        let opened = Box()
        let saved = (WebPaneModel.open, WebPaneModel.confirm, WebPaneModel.appName)
        WebPaneModel.open = { opened.urls.append($0) }
        WebPaneModel.appName = { _ in "Linear" }
        WebPaneModel.confirm = { name, _ in opened.asked.append(name); return opened.answer }
        defer { (WebPaneModel.open, WebPaneModel.confirm, WebPaneModel.appName) = saved }
        let model = WebPaneModel(terminal: "app-link-1", memory: WebPaneMemory(defaults: Self.defaults()))
        let window = Self.offscreen(model.webView)
        defer { window.close() }
        // A page of its own with one link, loaded without the network.
        model.webView.loadHTMLString(
            "<a id=l href='linear://acme/issue/ENG-1'>x</a><script>setTimeout(()=>{location.href='notion://auto'},50)</script>",
            baseURL: nil)
        // Loaded, then long enough for the page's own redirect to have fired.
        for _ in 0..<300 {
            try await Task.sleep(for: .milliseconds(100))
            if (try? await model.webView.evaluateJavaScript("document.getElementById('l') !== null") as? Bool) == true { break }
        }
        try await Task.sleep(for: .seconds(1))
        #expect(opened.urls.isEmpty, "a page's own redirect launched an app: \(opened.urls)")
        _ = try await model.webView.evaluateJavaScript("document.getElementById('l').click()")
        for _ in 0..<300 where opened.asked.isEmpty { try await Task.sleep(for: .milliseconds(100)) }
        try await Task.sleep(for: .milliseconds(300))
        #expect(opened.asked == ["Linear"], "the owner is asked, naming the app")
        #expect(opened.urls.isEmpty, "a non-web link opened without a yes")
        // A yes opens it.
        opened.answer = true
        _ = try await model.webView.evaluateJavaScript("document.getElementById('l').click()")
        for _ in 0..<300 where opened.urls.isEmpty { try await Task.sleep(for: .milliseconds(100)) }
        #expect(opened.urls == [URL(string: "linear://acme/issue/ENG-1")!])
    }

    final class Box {
        var urls: [URL] = []
        var asked: [String] = []
        var answer = false
    }

    // MARK: - M1: a page that arrives late

    @Test("A pane whose page is recorded after it appears loads it then")
    func aLatePageLoads() async throws {
        let model = WebPaneModel(terminal: "late-1", memory: WebPaneMemory(defaults: Self.defaults()))
        let host = NSHostingView(rootView: WebPane(model: model, opened: nil))
        let window = Self.offscreen(host)
        defer { window.close() }
        try await Task.sleep(for: .milliseconds(200))
        #expect(!model.started, "no page yet")
        host.rootView = WebPane(model: model, opened: URL(string: "http://127.0.0.1:9/"))
        let started = await Self.eventually {
            host.layoutSubtreeIfNeeded()
            return model.started
        }
        #expect(started, "the page arrived and the pane never loaded it")
    }

    // MARK: - M2: closing

    @Test("A closed pane's page is let go, forgotten, and its popups close")
    func aClosedPaneStopsItsPage() async throws {
        let memory = WebPaneMemory(defaults: Self.defaults())
        let panes = WebPanes()
        let model = panes.model(for: "closed-1")
        memory.remember(URL(string: "https://github.com/")!, for: "closed-1")
        model.webView.loadHTMLString("<title>Playing</title>", baseURL: URL(string: "https://example.com/")!)
        #expect(await Self.eventually { model.origin != nil })
        panes.close(terminals: ["closed-1", "never-opened"], memory: memory)
        #expect(!panes.holds("closed-1"))
        #expect(memory.page(for: "closed-1") == nil)
        #expect(
            await Self.eventually { model.webView.url?.absoluteString == "about:blank" },
            "the page is still there: \(String(describing: model.webView.url))")
        #expect(panes.model(for: "closed-1") !== model, "a pane reopened gets a new page")
    }

    @Test("A page on screen is never let go, however many there are; an unseen one is")
    func evictionLeavesWhatIsShown() {
        let panes = WebPanes()
        let shown = NSView()
        var models: [WebPaneModel] = []
        for n in 0..<(WebPanes.limit + 3) {
            let model = panes.model(for: "shown-\(n)")
            shown.addSubview(model.webView)
            models.append(model)
        }
        for n in 0..<models.count { #expect(panes.holds("shown-\(n)"), "shown-\(n) was reloaded") }
        // Off screen, the oldest are let go, down to the limit.
        for model in models { model.webView.removeFromSuperview() }
        _ = panes.model(for: "one-more")
        #expect(!panes.holds("shown-0"))
        #expect(panes.holds("one-more"))
    }

    // MARK: - M4, L4: what a page may ask

    @Test("A page is never given the camera or the microphone, in a pane or a popup")
    func noMediaCapture() {
        #expect(WebPaneModel.mediaCapture == .deny)
        let selector = #selector(
            WKUIDelegate.webView(_:requestMediaCapturePermissionFor:initiatedByFrame:type:decisionHandler:))
        let model = WebPaneModel(terminal: "media-1", memory: WebPaneMemory(defaults: Self.defaults()))
        #expect(model.responds(to: selector), "the pane lets WebKit ask")
        let popup = WebPopup(configuration: WebSession.configuration(), over: nil) { _ in }
        defer { popup.close() }
        #expect(popup.responds(to: selector), "the popup lets WebKit ask")
    }

    @Test("A page's alert, confirm and file chooser are answered, not dropped")
    func pageDialogsAreAnswered() {
        let model = WebPaneModel(terminal: "dialogs-1", memory: WebPaneMemory(defaults: Self.defaults()))
        for selector in [
            #selector(WKUIDelegate.webView(_:runJavaScriptAlertPanelWithMessage:initiatedByFrame:completionHandler:)),
            #selector(WKUIDelegate.webView(_:runJavaScriptConfirmPanelWithMessage:initiatedByFrame:completionHandler:)),
            #selector(WKUIDelegate.webView(_:runOpenPanelWith:initiatedByFrame:completionHandler:)),
        ] {
            #expect(model.responds(to: selector), "\(selector)")
        }
    }

    // MARK: - M5, L3

    @Test("Open Web Page is dimmed against a runner that doesn't serve web panes")
    func openWebPageNeedsTheCapability() {
        let group = PaneGroup(
            id: "@1", name: "", active: true, columns: 80, rows: 24, layout: "a",
            panes: [PaneRect(id: "a", short: "a", title: nil, left: 0, top: 0, columns: 80, rows: 24, focused: true, zoomed: false)])
        var focus = MainWindowFocus(overlayOpen: false)
        focus.layout = LayoutMenuFocus.make(
            group: group, here: group.panes[0], layouts: [group], switchesMode: false, opensWebPage: false)
        #expect(MainWindowFocus.lays(\.splits, focus), "a split is still offered")
        #expect(!MainWindowFocus.lays(\.opensWebPage, focus))
        focus.layout = LayoutMenuFocus.make(
            group: group, here: group.panes[0], layouts: [group], switchesMode: false, opensWebPage: true)
        #expect(MainWindowFocus.lays(\.opensWebPage, focus))
        #expect(Capability.webPane.rawValue == "web_pane")
    }

    @Test("A refused address says what to type, and the prompt says where it opens")
    func copyIsAccurate() {
        #expect(!WebPagePrompt.refusal.contains("start with http"))
        #expect(WebPagePrompt.refusal.contains("linear.app/acme"))
        #expect(!WebPagePrompt.message.contains("focused pane"), "from the orchestrator it opens among the checkout's layouts")
        #expect(WebAddress.typed("hello") == nil)
    }
}
