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
    let isFocused: Bool

    var body: some View {
        ZStack {
            WebPageView(model: model, isFocused: isFocused)
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
        .onAppear { model.start(opened: opened) }
    }
}

/// The pane's `WKWebView`, moved into this view rather than made by it, so a
/// layout switch keeps the page (`WebPanes`).
struct WebPageView: NSViewRepresentable {
    let model: WebPaneModel
    let isFocused: Bool

    final class Coordinator {
        var wasFocused = false
    }

    func makeCoordinator() -> Coordinator { Coordinator() }

    func makeNSView(context: Context) -> NSView {
        let container = NSView()
        place(in: container)
        return container
    }

    func updateNSView(_ container: NSView, context: Context) {
        if model.webView.superview !== container { place(in: container) }
        // The keyboard follows the focus ring when it arrives here, as it
        // does into a terminal; never taken back once the page has it.
        if isFocused, !context.coordinator.wasFocused, let window = container.window {
            window.makeFirstResponder(model.webView)
        }
        context.coordinator.wasFocused = isFocused
    }

    private func place(in container: NSView) {
        let webView = model.webView
        webView.removeFromSuperview()
        webView.frame = container.bounds
        webView.autoresizingMask = [.width, .height]
        container.addSubview(webView)
    }
}

/// Back, Forward, Reload and the page's name, for a pane's header strip.
struct WebPaneNavigation: View {
    @ObservedObject var model: WebPaneModel
    let isFocused: Bool

    var body: some View {
        HStack(spacing: 2) {
            control("chevron.left", help: "Back", enabled: model.canGoBack) { model.goBack() }
            control("chevron.right", help: "Forward", enabled: model.canGoForward) { model.goForward() }
            control(
                model.isLoading ? "xmark" : "arrow.clockwise",
                help: model.isLoading ? "Stop Loading" : "Reload Page", enabled: model.url != nil
            ) { model.reloadOrStop() }
            Text(model.name)
                .font(WorkspaceStyle.paneTitle)
                .fontWeight(isFocused ? .semibold : .medium)
                .foregroundStyle(isFocused ? .primary : .secondary)
                .lineLimit(1)
                .truncationMode(.tail)
                .padding(.leading, Spacing.tight)
                .help(model.url?.absoluteString ?? "")
        }
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
    static let message = "Enter an address to open beside the focused pane."

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
            errorBanner = "Far Cooler opens only web addresses that start with http or https."
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
