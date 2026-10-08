import AgentKit
import AppKit
import SwiftUI
import WebKit

/// A web pane's content: the page, why it didn't load, or that there's none
/// (ov-435). The header's controls are `WebPaneNavigation` and
/// `WebPaneOpenButton`, which a tiled pane draws in its header strip and a
/// pane on its own above this (`WebPaneBar`).
struct WebPane: View {
    @ObservedObject var model: WebPaneModel
    /// The page the runner says this pane opened on.
    let opened: URL?

    var body: some View {
        ZStack {
            WebPageView(model: model)
            if let failure = model.failure {
                ContentUnavailableView {
                    Label("Can’t Open Page", systemImage: "exclamationmark.triangle")
                } description: {
                    Text(failure)
                } actions: {
                    Button("Try Again") { model.reloadOrStop() }
                }
                .background(.background)
            } else if opened == nil && model.url == nil {
                ContentUnavailableView(
                    "No Page", systemImage: "globe",
                    description: Text("Open a page with Layout ▸ Open Web Page."))
                .background(.background)
            }
        }
        // Also when the page arrives after the pane does: the runner makes
        // the pane, then records its page, and a layout event can land
        // between the two (M1, ov-435 review 1).
        .onAppear { model.start(opened: opened) }
        .onChange(of: opened) { _, page in model.start(opened: page) }
    }
}

/// The pane's `WKWebView`, moved into this view rather than made by it, so a
/// layout switch keeps the page (`WebPanes`).
struct WebPageView: NSViewRepresentable {
    let model: WebPaneModel

    func makeNSView(context: Context) -> NSView {
        let container = NSView()
        place(in: container)
        return container
    }

    /// Never takes the keyboard. The page gets it when the owner clicks in it
    /// (WebKit does that itself), not when a layout event says this pane is
    /// the focused one: an agent can make that event (`layout focus`), and
    /// the keystrokes that follow, a password or a pasted token, are the
    /// owner's (H1, ov-435 review 1).
    func updateNSView(_ container: NSView, context: Context) {
        if model.webView.superview !== container { place(in: container) }
    }

    private func place(in container: NSView) {
        let webView = model.webView
        webView.removeFromSuperview()
        webView.frame = container.bounds
        webView.autoresizingMask = [.width, .height]
        container.addSubview(webView)
    }
}

/// Back, Forward, Reload and where the page is, for a pane's header strip.
struct WebPaneNavigation: View {
    @ObservedObject var model: WebPaneModel
    let isFocused: Bool

    /// How much header the controls leave the origin. A tiled pane can be a
    /// couple of hundred points wide, and the host is the one thing that must
    /// stay: Back and Forward go first, then Reload, so the origin never
    /// shrinks to nothing beside them (H2).
    @State private var width: CGFloat = 400
    static let showsHistory: CGFloat = 270
    static let showsReload: CGFloat = 210
    /// Below this the title and the words Not Secure go, leaving the lock
    /// (or the warning mark) and the host.
    static let showsTitle: CGFloat = 320

    var body: some View {
        HStack(spacing: 2) {
            if width >= Self.showsHistory {
                control("chevron.left", help: "Back", enabled: model.canGoBack) { model.goBack() }
                control("chevron.right", help: "Forward", enabled: model.canGoForward) { model.goForward() }
            }
            if width >= Self.showsReload {
                control(
                    model.isLoading ? "xmark" : "arrow.clockwise",
                    help: model.isLoading ? "Stop Loading" : "Reload Page", enabled: model.url != nil
                ) { model.reloadOrStop() }
            }
            WebOriginLabel(model: model, isFocused: isFocused, compact: width < Self.showsTitle)
                .padding(.leading, Spacing.tight)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .onGeometryChange(for: CGFloat.self, of: { $0.size.width }) { width = $0 }
    }

    private func control(_ symbol: String, help: String, enabled: Bool, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            Image(systemName: symbol)
                .font(.system(size: 10, weight: .medium))
                .frame(width: WorkspaceStyle.controlTarget, height: WorkspaceStyle.controlTarget)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .foregroundStyle(.secondary)
        .disabled(!enabled)
        .opacity(enabled ? 1 : 0.4)
        .help(help)
        .accessibilityLabel(help)
    }
}

/// Where the page is, always: a lock and the host, with the page's own title
/// after it and quieter (H2, ov-435 review 1). The title is the page's to
/// choose, so it can say "Sign In to Google"; the host is what the address
/// really is. A narrow header drops the title first and then the front of
/// the host, never the end of it, which is the part that names the site
/// (`accounts.google.com.example.net` must not shorten to `accounts.google`).
/// Plain http is marked in words and in the attention color.
struct WebOriginLabel: View {
    @ObservedObject var model: WebPaneModel
    let isFocused: Bool
    /// A narrow header: the host alone, after its mark.
    var compact = false
    @Environment(\.colorScheme) private var scheme

    var body: some View {
        HStack(spacing: 4) {
            if let origin = model.origin {
                Image(systemName: origin.isSecure ? "lock.fill" : "exclamationmark.triangle.fill")
                    .font(.system(size: 9, weight: .medium))
                    .foregroundStyle(origin.isSecure ? AnyShapeStyle(.secondary) : AnyShapeStyle(Tint.attention(scheme)))
                    .accessibilityHidden(true)
                    .identified("web-origin-lock")
                Text(origin.host)
                    .font(WorkspaceStyle.paneTitle)
                    .fontWeight(isFocused ? .semibold : .medium)
                    .foregroundStyle(origin.isSecure ? (isFocused ? AnyShapeStyle(.primary) : AnyShapeStyle(.secondary)) : AnyShapeStyle(Tint.attention(scheme)))
                    .lineLimit(1)
                    .truncationMode(.head)
                    .layoutPriority(2)
                    .identified("web-origin-host")
                if !origin.isSecure, !compact {
                    Text("Not Secure")
                        .font(WorkspaceStyle.paneTitle)
                        .foregroundStyle(Tint.attention(scheme))
                        .lineLimit(1)
                        .layoutPriority(1)
                }
                if let title = model.pageTitle, !compact {
                    Text(title)
                        .font(WorkspaceStyle.paneTitle)
                        .foregroundStyle(.tertiary)
                        .lineLimit(1)
                        .truncationMode(.tail)
                        .identified("web-origin-title")
                }
            } else {
                Text("Web Page")
                    .font(WorkspaceStyle.paneTitle)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }
        }
        .help(model.url?.absoluteString ?? "")
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(model.origin?.spoken ?? "Web Page")
        .accessibilityValue(model.pageTitle ?? "")
    }
}

/// Open in Browser: the page in the owner's default browser.
struct WebPaneOpenButton: View {
    @ObservedObject var model: WebPaneModel

    var body: some View {
        Button { model.openInBrowser() } label: {
            Image(systemName: "safari")
                .font(.system(size: 11))
                .frame(width: WorkspaceStyle.controlTarget, height: WorkspaceStyle.controlTarget)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .foregroundStyle(.secondary)
        .disabled(model.url == nil)
        .help("Open in Browser")
        .accessibilityLabel("Open in Browser")
    }
}

/// The controls on their own strip, for a web pane drawn outside a layout,
/// which has no header of its own (`TerminalPane`).
struct WebPaneBar: View {
    let model: WebPaneModel

    var body: some View {
        HStack(spacing: 6) {
            WebPaneNavigation(model: model, isFocused: true)
            Spacer(minLength: 4)
            WebPaneOpenButton(model: model)
        }
        .padding(.horizontal, 8)
        .frame(height: WorkspaceStyle.paneHeaderHeight)
    }
}

/// Layout ▸ Open Web Page: an address, asked for in a sheet on the window.
enum WebPagePrompt {
    static let title = "Open Web Page"
    static let message = "Enter a web address. It opens in a new pane."
    /// What a refused address says: what to type, not which schemes are allowed.
    static let refusal = "Enter a web address, like linear.app/acme or https://github.com."

    @MainActor
    static func makeAlert() -> (NSAlert, NSTextField) {
        let alert = NSAlert()
        alert.messageText = title
        alert.informativeText = message
        alert.addButton(withTitle: "Open")
        alert.addButton(withTitle: "Cancel")
        let field = NSTextField(frame: NSRect(x: 0, y: 0, width: 320, height: 22))
        field.placeholderString = "https://linear.app/…"
        field.cell?.isScrollable = true
        field.cell?.wraps = false
        alert.accessoryView = field
        alert.window.initialFirstResponder = field
        return (alert, field)
    }

    /// What was typed, or nil for Cancel. `WebAddress.typed` reads it.
    @MainActor
    static func ask(on window: NSWindow?) async -> String? {
        let (alert, field) = makeAlert()
        let response: NSApplication.ModalResponse
        if let window {
            response = await alert.beginSheetModal(for: window)
        } else {
            response = alert.runModal()
        }
        guard response == .alertFirstButtonReturn else { return nil }
        return field.stringValue
    }
}

extension ContentView {
    /// Layout ▸ Open Web Page (ov-435): ask for an address, then open it
    /// beside `here` in the layout on screen, as `layout open-url` does for
    /// an agent. From the orchestrator's column it names neither, so the
    /// runner opens it among the checkout's own layouts, never in the
    /// orchestrator's window.
    func openWebPage(in worktree: Worktree, beside here: PaneRect?, layout shown: String?) async {
        guard let typed = await WebPagePrompt.ask(on: NSApp.keyWindow) else { return }
        guard let page = WebAddress.typed(typed) else {
            errorBanner = WebPagePrompt.refusal
            return
        }
        let inOrchestrator = WorkspaceScreen.opensShellInstead(.splitRight, key: selectedPane, in: self.shown)
        let groups = await act(.arrange, on: worktree, default: []) { c in
            await c.openWebPage(
                page, in: worktree, beside: inOrchestrator ? nil : here?.short,
                layout: inOrchestrator ? nil : shown)
        }
        reveal(groups, in: worktree)
    }
}

extension DaemonClient {
    /// `farcooler layout open-url`: `page` in a new pane beside `terminal`,
    /// or the focused pane of `layout`, or in a window of its own when the
    /// worktree has no layout. Reads the fleet afterwards, as `split` does,
    /// because it makes a terminal.
    @discardableResult
    func openWebPage(
        _ page: URL, in worktree: Worktree, beside terminal: String?, layout group: String?
    ) async -> [PaneGroup] {
        var rest = [page.absoluteString] + (terminal.map { [$0] } ?? [])
        rest += Self.naming(group)
        let groups = await layout(worktree, ["open-url"], rest)
        await refresh()
        return groups
    }
}
