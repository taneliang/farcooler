import AppKit
import Combine
import WebKit

// Web pages as panes (ov-435).
//
// A web pane is a real tmux pane (`PaneMode.web`), made by `layout open-url`,
// whose rectangle this app draws a `WKWebView` into, the way it draws a diff
// into a changes pane. The runner records the page the pane opened on
// (`Terminal.webUrl`); everything after that stays on this Mac.
//
// Ruling R-41: an agent may open a page for the owner to look at, and nothing
// reads or acts on page contents. So this file injects no script, registers no
// message handler, and sends nothing back to the runner: not the title, not
// the page the owner navigated to.

/// Which addresses a web pane loads: http and https, with a host. The runner
/// refuses everything else before a pane is made (`web_pane::checked_url`);
/// this is the same rule on this side, so a record from anywhere else can't
/// load `file:` or `javascript:` here either.
enum WebAddress {
    static func page(_ raw: String?) -> URL? {
        guard let raw, !raw.contains(where: { $0.isWhitespace || $0.isNewline }),
            let url = URL(string: raw)
        else { return nil }
        return page(url)
    }

    static func page(_ url: URL) -> URL? {
        guard let scheme = url.scheme?.lowercased(), scheme == "http" || scheme == "https",
            let host = url.host(), !host.isEmpty
        else { return nil }
        return url
    }

    /// What a person typed in Open Web Page: an address, or a bare host,
    /// which means https. `github.com/acme` is `https://github.com/acme`.
    /// Anything with another scheme is refused, not rewritten.
    static func typed(_ text: String) -> URL? {
        let text = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty else { return nil }
        if let url = page(text) { return url }
        // Another scheme (`file:`, `javascript:`, `linear:`), or a URL that
        // didn't parse: refused. A host with a port (`localhost:3000`) isn't
        // a scheme, since a scheme is never all digits after its colon.
        if let colon = text.firstIndex(of: ":") {
            let after = text[text.index(after: colon)...].prefix { $0 != "/" }
            if after.isEmpty || !after.allSatisfy(\.isNumber) { return nil }
        }
        guard let url = page("https://" + text), let host = url.host(),
            host.contains(".") || host == "localhost"
        else { return nil }
        return url
    }
}

/// Where a page really is, as the header must say it (H2, ov-435 review 1).
///
/// A page chooses its own title, and an agent chooses the page, so neither
/// can be what tells the owner whether this is the site they expect to sign
/// in to. The host is: read from the address WebKit is showing (the
/// punycode form WebKit itself loads, so a look-alike spelling reads as the
/// `xn--` name it really is), never from the address the pane was opened on,
/// which an open redirect leaves behind.
struct WebOrigin: Equatable {
    /// The host, with a port that isn't the scheme's own.
    let host: String
    let isSecure: Bool

    init?(_ url: URL?) {
        guard let url, let page = WebAddress.page(url) else { return nil }
        let isSecure = page.scheme?.lowercased() == "https"
        // The authority of the address as WebKit loads it: after `//`, before
        // the path, without any `user@` (`https://www.google.com@evil.example`
        // is evil.example).
        let address = page.absoluteString
        guard let start = address.range(of: "://") else { return nil }
        let rest = address[start.upperBound...]
        var authority = rest.prefix { $0 != "/" && $0 != "?" && $0 != "#" }
        if let at = authority.lastIndex(of: "@") { authority = authority[authority.index(after: at)...] }
        var host = String(authority).lowercased()
        // `:port` only when it isn't the scheme's own (`:443`, `:80`).
        if let colon = host.lastIndex(of: ":"), !host[colon...].contains("]") {
            let port = host[host.index(after: colon)...]
            host = String(host[..<colon])
            if !port.isEmpty, port != (isSecure ? "443" : "80") { host += ":" + port }
        }
        guard !host.isEmpty else { return nil }
        self.host = host
        self.isSecure = isSecure
    }

    /// For VoiceOver and the window of a sign-in popup.
    var spoken: String { isSecure ? "Secure connection to \(host)" : "Not secure connection to \(host)" }
}

/// The page's own web view. The tiling prefix gets the first look at a key,
/// as it does in a terminal, so ⌃B reaches the layout instead of the page
/// and a page can't trap the keyboard (M3, ov-435 review 1).
final class PaneWebView: WKWebView {
    override func performKeyEquivalent(with event: NSEvent) -> Bool {
        if MainActor.assumeIsolated({ PrefixMode.shared.handle(event) }) == .handled { return true }
        return super.performKeyEquivalent(with: event)
    }
}

/// The one place web panes keep cookies and logins: a persistent data store
/// of its own, shared by every web pane, so signing in to Linear once covers
/// every Linear pane and survives relaunch. Its own identifier, rather than
/// the default store, so it can be cleared on its own later.
///
/// A plain file store under `~/Library/WebKit`, readable by any agent running
/// as the owner; the options for that are the design's H3 section.
@MainActor
enum WebSession {
    static let identifier = UUID(uuidString: "0435F00D-7EB0-4A5E-9C0D-FA2C0001E0B5")!
    static let store = WKWebsiteDataStore(forIdentifier: identifier)

    /// A fresh configuration on the shared store. No user scripts and no
    /// script message handlers: nothing on a page can reach this app (R-41).
    static func configuration() -> WKWebViewConfiguration {
        let configuration = WKWebViewConfiguration()
        configuration.websiteDataStore = store
        configuration.preferences.javaScriptCanOpenWindowsAutomatically = false
        return configuration
    }
}

/// Where each web pane was last, on this Mac: a pane comes back to the page
/// the owner left it on, not the one an agent opened it on. Kept here, never
/// sent to the runner (R-41). At most `limit` panes, the oldest forgotten
/// first, so panes long closed don't pile up.
struct WebPaneMemory {
    static let key = "WebPanePages"
    static let limit = 200
    var defaults: UserDefaults = .standard

    func page(for terminal: String) -> URL? {
        let pages = defaults.dictionary(forKey: Self.key) as? [String: [String: Any]]
        return WebAddress.page(pages?[terminal]?["url"] as? String)
    }

    /// A closed pane's place is forgotten with it.
    func forget(_ terminal: String) {
        var pages = defaults.dictionary(forKey: Self.key) as? [String: [String: Any]] ?? [:]
        guard pages.removeValue(forKey: terminal) != nil else { return }
        defaults.set(pages, forKey: Self.key)
    }

    func remember(_ url: URL, for terminal: String, at now: Date = .now) {
        guard WebAddress.page(url) != nil else { return }
        var pages = defaults.dictionary(forKey: Self.key) as? [String: [String: Any]] ?? [:]
        pages[terminal] = ["url": url.absoluteString, "at": now.timeIntervalSince1970]
        while pages.count > Self.limit {
            let oldest = pages.min { ($0.value["at"] as? Double ?? 0) < ($1.value["at"] as? Double ?? 0) }
            guard let oldest else { break }
            pages[oldest.key] = nil
        }
        defaults.set(pages, forKey: Self.key)
    }
}

/// One web pane's page: its `WKWebView` and what the header shows about it.
///
/// Held by `WebPanes` rather than by a view, so a layout switch or a redraw
/// moves the web view to its new place instead of loading the page again.
@MainActor
final class WebPaneModel: NSObject, ObservableObject {
    let terminal: String
    let webView: WKWebView
    @Published private(set) var title = ""
    @Published private(set) var url: URL?
    @Published private(set) var canGoBack = false
    @Published private(set) var canGoForward = false
    @Published private(set) var isLoading = false
    /// Why the page didn't load, in the system's words, or nil.
    @Published private(set) var failure: String?

    private let memory: WebPaneMemory
    /// Whether the first page has been asked for.
    private(set) var started = false
    private var watching: Set<AnyCancellable> = []
    private var popups: [WebPopup] = []

    init(terminal: String, memory: WebPaneMemory = WebPaneMemory()) {
        self.terminal = terminal
        self.memory = memory
        webView = PaneWebView(frame: .zero, configuration: WebSession.configuration())
        super.init()
        webView.navigationDelegate = self
        webView.uiDelegate = self
        webView.allowsBackForwardNavigationGestures = true
        webView.allowsMagnification = true
        watch()
    }

    /// Load the page this pane was last on here, else `opened`, the first
    /// time the pane is shown. Later calls do nothing: the page is live.
    func start(opened: URL?) {
        guard !started, let page = firstPage(opened: opened) else { return }
        started = true
        webView.load(URLRequest(url: page))
    }

    /// Where the pane opens: where the owner left it on this Mac, else the
    /// page the runner says it opened on.
    func firstPage(opened: URL?) -> URL? {
        memory.page(for: terminal) ?? opened.flatMap(WebAddress.page)
    }

    func goBack() { webView.goBack() }
    func goForward() { webView.goForward() }

    /// Reload, or stop a load in progress. A failed first load is tried
    /// again from the address it failed on.
    func reloadOrStop() {
        if isLoading { webView.stopLoading(); return }
        failure = nil
        if webView.url == nil, let page = url { webView.load(URLRequest(url: page)) } else { webView.reload() }
    }

    /// The page in the owner's default browser.
    func openInBrowser() {
        guard let page = url.flatMap(WebAddress.page) else { return }
        NSWorkspace.shared.open(page)
    }

    /// Where the page is now, or nil before the first address.
    var origin: WebOrigin? { WebOrigin(url) }

    /// The page's own title, when it says something its origin doesn't:
    /// shown beside the origin, never in place of it.
    var pageTitle: String? {
        let title = title.trimmingCharacters(in: .whitespacesAndNewlines)
        return title.isEmpty || title == origin?.host ? nil : title
    }

    /// Let go of the page: a pane that was closed must not keep playing
    /// audio or video, holding sockets or running timers, and its sign-in
    /// popups go with it (M2, ov-435 review 1).
    func close() {
        webView.stopLoading()
        webView.load(URLRequest(url: URL(string: "about:blank")!))
        for popup in popups { popup.close() }
        popups.removeAll()
    }

    private func watch() {
        webView.publisher(for: \.title).sink { [weak self] title in
            MainActor.assumeIsolated { self?.title = title ?? "" }
        }.store(in: &watching)
        // `url` moves on a single-page app's own navigation too (Linear,
        // Notion and GitHub all do it), which no delegate call reports.
        webView.publisher(for: \.url).sink { [weak self] url in
            MainActor.assumeIsolated { self?.landed(on: url) }
        }.store(in: &watching)
        webView.publisher(for: \.canGoBack).sink { [weak self] value in
            MainActor.assumeIsolated { self?.canGoBack = value }
        }.store(in: &watching)
        webView.publisher(for: \.canGoForward).sink { [weak self] value in
            MainActor.assumeIsolated { self?.canGoForward = value }
        }.store(in: &watching)
        webView.publisher(for: \.isLoading).sink { [weak self] value in
            MainActor.assumeIsolated { self?.isLoading = value }
        }.store(in: &watching)
    }

    private func landed(on url: URL?) {
        guard let url, let page = WebAddress.page(url) else { return }
        self.url = page
        memory.remember(page, for: terminal)
    }

    fileprivate func failed(_ error: Error) {
        let error = error as NSError
        // A load this pane stopped or replaced, which isn't a failure: a
        // click before the last page finished, or a cancelled navigation.
        if error.domain == NSURLErrorDomain && error.code == NSURLErrorCancelled { return }
        if error.domain == "WebKitErrorDomain" && error.code == 102 { return }
        if let page = (error.userInfo[NSURLErrorFailingURLErrorKey] as? URL).flatMap(WebAddress.page) {
            url = page
        }
        failure = error.localizedDescription
    }
}

extension WebPaneModel: WKNavigationDelegate {
    /// http and https in the pane itself; anything else is cancelled. Frames
    /// inside a page load what the page loads.
    func webView(
        _ webView: WKWebView, decidePolicyFor action: WKNavigationAction,
        decisionHandler: @escaping @MainActor (WKNavigationActionPolicy) -> Void
    ) {
        decisionHandler(WebPaneModel.allows(action.request.url, mainFrame: action.targetFrame?.isMainFrame ?? true) ? .allow : .cancel)
    }

    nonisolated static func allows(_ url: URL?, mainFrame: Bool) -> Bool {
        guard mainFrame else { return true }
        guard let url else { return false }
        return WebAddress.page(url) != nil || url.absoluteString == "about:blank"
    }

    func webView(_ webView: WKWebView, didStartProvisionalNavigation navigation: WKNavigation!) {
        failure = nil
    }

    func webView(_ webView: WKWebView, didFailProvisionalNavigation navigation: WKNavigation!, withError error: Error) {
        failed(error)
    }

    func webView(_ webView: WKWebView, didFail navigation: WKNavigation!, withError error: Error) {
        failed(error)
    }
}

extension WebPaneModel: WKUIDelegate {
    /// A link that asks for a new window (`target=_blank`) opens here, where
    /// Back returns from it. A script's `window.open`, which is how a sign-in
    /// popup works, gets a small window of its own sharing this pane's
    /// configuration, so the page that opened it hears back.
    func webView(
        _ webView: WKWebView, createWebViewWith configuration: WKWebViewConfiguration,
        for action: WKNavigationAction, windowFeatures: WKWindowFeatures
    ) -> WKWebView? {
        let url = action.request.url
        guard url == nil || url?.absoluteString.isEmpty == true || WebPaneModel.allows(url, mainFrame: true) else {
            return nil
        }
        if action.navigationType == .linkActivated, let url {
            webView.load(URLRequest(url: url))
            return nil
        }
        let popup = WebPopup(configuration: configuration, over: webView.window) { [weak self] closed in
            self?.popups.removeAll { $0 === closed }
        }
        popups.append(popup)
        return popup.webView
    }

    /// A page never gets the camera or the microphone: the app holds the
    /// camera entitlement for scanning a device code, so a prompt from a page
    /// an agent opened would read as Far Cooler asking (M4, ov-435 review 1).
    nonisolated static let mediaCapture: WKPermissionDecision = .deny

    func webView(
        _ webView: WKWebView, requestMediaCapturePermissionFor origin: WKSecurityOrigin,
        initiatedByFrame frame: WKFrameInfo, type: WKMediaCaptureType,
        decisionHandler: @escaping @MainActor (WKPermissionDecision) -> Void
    ) {
        decisionHandler(Self.mediaCapture)
    }

    // The page's own dialogs (L4). Without them `confirm()` answers no, so
    // GitHub's "Are you sure?" buttons do nothing, and a file input never
    // opens. Each says which site is asking.

    func webView(
        _ webView: WKWebView, runJavaScriptAlertPanelWithMessage message: String,
        initiatedByFrame frame: WKFrameInfo
    ) async {
        _ = await WebDialogs.ask(message, from: frame, in: webView, cancellable: false)
    }

    func webView(
        _ webView: WKWebView, runJavaScriptConfirmPanelWithMessage message: String,
        initiatedByFrame frame: WKFrameInfo
    ) async -> Bool {
        await WebDialogs.ask(message, from: frame, in: webView, cancellable: true)
    }

    func webView(
        _ webView: WKWebView, runOpenPanelWith parameters: WKOpenPanelParameters,
        initiatedByFrame frame: WKFrameInfo
    ) async -> [URL]? {
        await WebDialogs.chooseFiles(parameters, in: webView)
    }
}

/// A page's alert, confirm and file chooser, as sheets on the window the
/// page is in.
@MainActor
enum WebDialogs {
    /// Who is asking, as the dialog's title: the frame's own host, since an
    /// embedded frame is not the site in the header.
    static func title(for frame: WKFrameInfo) -> String {
        let host = WebOrigin(frame.request.url)?.host ?? "This page"
        return "\(host) says"
    }

    static func ask(_ message: String, from frame: WKFrameInfo, in webView: WKWebView, cancellable: Bool) async -> Bool {
        let alert = NSAlert()
        alert.messageText = title(for: frame)
        alert.informativeText = message
        alert.addButton(withTitle: "OK")
        if cancellable { alert.addButton(withTitle: "Cancel") }
        let response: NSApplication.ModalResponse
        if let window = webView.window {
            response = await alert.beginSheetModal(for: window)
        } else {
            response = alert.runModal()
        }
        return response == .alertFirstButtonReturn
    }

    static func chooseFiles(_ parameters: WKOpenPanelParameters, in webView: WKWebView) async -> [URL]? {
        let panel = NSOpenPanel()
        panel.allowsMultipleSelection = parameters.allowsMultipleSelection
        panel.canChooseDirectories = parameters.allowsDirectories
        panel.canChooseFiles = true
        let response: NSApplication.ModalResponse
        if let window = webView.window {
            response = await panel.beginSheetModal(for: window)
        } else {
            response = panel.runModal()
        }
        return response == .OK ? panel.urls : nil
    }
}

/// A page's own popup window: a sign-in, mostly. Closes when the page closes
/// it, and loads only http and https, as a pane does.
@MainActor
final class WebPopup: NSObject, WKUIDelegate, WKNavigationDelegate, NSWindowDelegate {
    let webView: WKWebView
    private let window: NSWindow
    private let onClose: (WebPopup) -> Void

    init(configuration: WKWebViewConfiguration, over parent: NSWindow?, onClose: @escaping (WebPopup) -> Void) {
        webView = WKWebView(frame: NSRect(x: 0, y: 0, width: 520, height: 680), configuration: configuration)
        window = NSWindow(
            contentRect: webView.frame, styleMask: [.titled, .closable, .resizable],
            backing: .buffered, defer: false)
        self.onClose = onClose
        super.init()
        window.isReleasedWhenClosed = false
        window.contentView = webView
        window.delegate = self
        webView.uiDelegate = self
        webView.navigationDelegate = self
        if let parent {
            window.setFrameOrigin(NSPoint(x: parent.frame.midX - 260, y: parent.frame.midY - 340))
        } else {
            window.center()
        }
        window.makeKeyAndOrderFront(nil)
    }

    func webViewDidClose(_ webView: WKWebView) { window.close() }

    /// Close the window from the pane that owns it.
    func close() { window.close() }

    func webView(
        _ webView: WKWebView, requestMediaCapturePermissionFor origin: WKSecurityOrigin,
        initiatedByFrame frame: WKFrameInfo, type: WKMediaCaptureType,
        decisionHandler: @escaping @MainActor (WKPermissionDecision) -> Void
    ) {
        decisionHandler(WebPaneModel.mediaCapture)
    }

    /// What the window's title bar says about the page: its origin, always,
    /// with a not-secure mark, and the page's own title under it (H2).
    static func titles(for url: URL?, title: String?) -> (title: String, subtitle: String) {
        guard let origin = WebOrigin(url) else { return (title ?? "", "") }
        let shown = (title ?? "") == origin.host ? "" : (title ?? "")
        return ((origin.isSecure ? "" : "Not Secure  ") + origin.host, shown)
    }

    private func retitle() {
        let titles = Self.titles(for: webView.url, title: webView.title)
        window.title = titles.title
        window.subtitle = titles.subtitle
    }

    func windowWillClose(_ notification: Notification) { onClose(self) }

    func webView(
        _ webView: WKWebView, decidePolicyFor action: WKNavigationAction,
        decisionHandler: @escaping @MainActor (WKNavigationActionPolicy) -> Void
    ) {
        decisionHandler(WebPaneModel.allows(action.request.url, mainFrame: action.targetFrame?.isMainFrame ?? true) ? .allow : .cancel)
    }

    func webView(_ webView: WKWebView, didCommit navigation: WKNavigation!) { retitle() }

    func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) { retitle() }
}

/// Every web pane's page, by terminal id, so a pane keeps its page while
/// SwiftUI rebuilds the view around it. The least recently shown are let go
/// past `limit`; one shown again reloads where it was (`WebPaneMemory`).
@MainActor
final class WebPanes {
    static let shared = WebPanes()
    static let limit = 12
    private var models: [String: WebPaneModel] = [:]
    private var order: [String] = []

    func model(for terminal: String) -> WebPaneModel {
        order.removeAll { $0 == terminal }
        order.append(terminal)
        if let model = models[terminal] { return model }
        let model = WebPaneModel(terminal: terminal)
        models[terminal] = model
        // Only a page nobody is looking at is let go: evicting one on screen
        // reloads it, and past the limit every redraw would reload another.
        while order.count > Self.limit,
            let idle = order.first(where: { models[$0]?.webView.superview == nil && $0 != terminal })
        {
            release(idle)
        }
        return model
    }

    /// Whether a page is held for `terminal`, without making one.
    func holds(_ terminal: String) -> Bool { models[terminal] != nil }

    /// The panes that were closed: their pages stop, and nothing remembers
    /// where they were (M2, ov-435 review 1).
    func close(terminals: some Sequence<String>, memory: WebPaneMemory = WebPaneMemory()) {
        for terminal in terminals {
            memory.forget(terminal)
            guard let model = models[terminal] else { continue }
            model.close()
            release(terminal)
        }
    }

    private func release(_ terminal: String) {
        models[terminal] = nil
        order.removeAll { $0 == terminal }
    }
}
