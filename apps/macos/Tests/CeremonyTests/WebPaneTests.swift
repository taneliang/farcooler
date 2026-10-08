import AgentKit
import AppKit
import Foundation
import Testing
import WebKit

@testable import Far_Cooler

/// Web pages as panes (ov-435): what the runner's record says, which pages
/// load, where a pane reopens, and the one place logins live.
@MainActor
struct WebPaneTests {
    private static var root: URL {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<5 { root.deleteLastPathComponent() }
        return root
    }

    private static func defaults() -> UserDefaults {
        let name = "WebPaneTests-\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: name)!
        defaults.removePersistentDomain(forName: name)
        return defaults
    }

    /// `test/fixtures/web-pane-terminal.json` is the CLI's own output for a
    /// web pane (`a_web_panes_json_is_the_shape_the_mac_reads`), so this is
    /// the real parser on real bytes.
    @Test("A web pane from the CLI is a web pane here, and no agent")
    func decodesTheCLIsWebPane() throws {
        let data = try Data(contentsOf: Self.root.appendingPathComponent("test/fixtures/web-pane-terminal.json"))
        let terminal = try JSONDecoder().decode(Terminal.self, from: data)
        #expect(terminal.isWebPane)
        #expect(terminal.isClientDrawn)
        #expect(terminal.webPage == URL(string: "https://github.com/"))
        #expect(terminal.label == "Web Page", "never the host it opened on (WebPaneReviewTests)")
        #expect(terminal.headerName == "Web Page")
        #expect(!terminal.hasDetectedAgent, "the pane runs `farcooler pane-host`, not an agent")
        #expect(!terminal.runsAgent)
        #expect(!terminal.canSwitchPaneMode)
        #expect(!DaemonClient.isLiveAgent(terminal))
        #expect(ContentView.treeTerminal(terminal).isWeb, "its navigator row draws a globe")
        #expect(OneTreeGlyph.web == "globe")
    }

    @Test("A lone web pane keeps its header, where its controls are")
    func aLoneWebPaneHasAHeader() {
        var terminal = Terminal(id: "w", short: "w", title: "Web", preset: "web", state: "running", epoch: 0)
        terminal.paneMode = "web"
        let group = PaneGroup(
            id: "@1", name: "", active: true, columns: 80, rows: 24, layout: "a",
            panes: [PaneRect(id: "w", short: "w", title: nil, left: 0, top: 0, columns: 80, rows: 24, focused: true, zoomed: false)])
        #expect(TileView.headerHeight(group, terminals: [terminal]) == WorkspaceStyle.paneHeaderHeight)
    }

    @Test("Only http and https load, from the runner or a link")
    func onlyHTTPAndHTTPSLoad() {
        for page in ["https://github.com/", "http://localhost:3000/x", "HTTPS://linear.app/acme/issue/ENG-1"] {
            #expect(WebAddress.page(page) != nil, "\(page)")
        }
        for page in [
            "file:///etc/passwd", "javascript:alert(1)", "data:text/html,hi", "about:blank", "linear://issue/1",
            "https://", "https://exa mple.com/", "github.com", "", nil,
        ] as [String?] {
            #expect(WebAddress.page(page) == nil, "\(page ?? "nil")")
        }
        // A navigation in the pane: the main frame only goes to http and https.
        #expect(WebPaneModel.allows(URL(string: "https://notion.so/x"), mainFrame: true))
        #expect(!WebPaneModel.allows(URL(string: "file:///etc/passwd"), mainFrame: true))
        #expect(!WebPaneModel.allows(URL(string: "linear://issue/1"), mainFrame: true))
        #expect(WebPaneModel.allows(URL(string: "about:blank"), mainFrame: true))
        #expect(WebPaneModel.allows(URL(string: "blob:https://notion.so/1"), mainFrame: false), "a page's own frames")
    }

    @Test("A typed host means https, and another scheme is refused")
    func typedAddresses() {
        #expect(WebAddress.typed("github.com/acme") == URL(string: "https://github.com/acme"))
        #expect(WebAddress.typed("  https://linear.app/  ") == URL(string: "https://linear.app/"))
        #expect(WebAddress.typed("localhost:3000") == URL(string: "https://localhost:3000"))
        for typed in ["javascript:alert(1)", "file:///etc/passwd", "linear://issue/1", "mailto:a@b.c", "hello", "", "a b.com"] {
            #expect(WebAddress.typed(typed) == nil, "\(typed)")
        }
    }

    @Test("A pane reopens where the owner left it, else where it opened")
    func reopensWhereItWasLeft() {
        let memory = WebPaneMemory(defaults: Self.defaults())
        let model = WebPaneModel(terminal: "t-1", memory: memory)
        let opened = URL(string: "https://github.com/")!
        #expect(model.firstPage(opened: opened) == opened)
        #expect(model.firstPage(opened: URL(string: "file:///etc/passwd")) == nil)

        memory.remember(URL(string: "https://github.com/acme/repo/pull/7")!, for: "t-1")
        #expect(model.firstPage(opened: opened) == URL(string: "https://github.com/acme/repo/pull/7"))
        #expect(memory.page(for: "t-2") == nil, "each pane its own")

        memory.remember(URL(string: "file:///etc/passwd")!, for: "t-1")
        #expect(memory.page(for: "t-1") == URL(string: "https://github.com/acme/repo/pull/7"), "never a local file")
    }

    @Test("At most the limit of panes are remembered, the oldest forgotten")
    func memoryIsCapped() {
        let memory = WebPaneMemory(defaults: Self.defaults())
        let start = Date(timeIntervalSince1970: 1_000)
        for n in 0...WebPaneMemory.limit {
            memory.remember(URL(string: "https://example.com/\(n)")!, for: "t-\(n)", at: start + Double(n))
        }
        #expect(memory.page(for: "t-0") == nil, "the oldest went")
        #expect(memory.page(for: "t-1") != nil)
        #expect(memory.page(for: "t-\(WebPaneMemory.limit)") != nil)
    }

    /// One persistent store of their own, so a login survives relaunch and
    /// covers every pane, with nothing a page could call into (R-41).
    @Test("Every web pane shares one persistent store, and no scripts")
    func oneStoreForLogins() {
        let first = WebPanes.shared.model(for: "store-a").webView.configuration
        let second = WebPanes.shared.model(for: "store-b").webView.configuration
        #expect(first.websiteDataStore.isPersistent)
        #expect(first.websiteDataStore.identifier == WebSession.identifier)
        #expect(first.websiteDataStore === second.websiteDataStore)
        #expect(first.websiteDataStore !== WKWebsiteDataStore.default())
        #expect(first.userContentController.userScripts.isEmpty)
        #expect(WebPanes.shared.model(for: "store-a") === WebPanes.shared.model(for: "store-a"), "kept, not remade")
    }
}
