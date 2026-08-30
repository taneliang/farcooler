import SwiftUI

// Every workspace at once, arrived at by keeping going.
//
// The overview is not a screen you navigate to. It is the far end of the same
// upward drag that unfurls the column: past the last row the column dissolves
// and this arrives in its place, so "show me everything" is the same gesture
// as "show me this workspace's tabs", continued. That is why there is no
// button anywhere that opens it.
//
// It is also the design's answer to forty workspaces. Every strip-shaped
// design died on that number — a bar that compresses to fit forty of anything
// is a bar with forty illegible things on it — and the answer is that the bar
// never tries: it shows ONE workspace, and the fleet lives here, in a grid
// that can scroll.

/// One workspace as a card: what it is called, what its tabs are doing, and
/// whether it is the one you are in.
///
/// 168×132, from the mechanics doc. Two across on a phone, which at 393 points
/// leaves the grid its own margins without any of the cards having to be a
/// different width from the others.
private struct ShellOverviewCard: View {
    let workspace: ShellWorkspace
    let isCurrent: Bool
    let onOpen: () -> Void

    /// `server · N tabs`, with the server left out when it is the local one —
    /// the same rule the bar follows, so a card and the bar never disagree
    /// about whether a workspace is worth naming a runner for.
    private static func subtitle(for workspace: ShellWorkspace) -> String {
        let tabs = "\(workspace.tabs.count) \(workspace.tabs.count == 1 ? "tab" : "tabs")"
        guard let server = workspace.server else { return tabs }
        return "\(server) · \(tabs)"
    }

    var body: some View {
        Button(action: onOpen) {
            VStack(alignment: .leading, spacing: PaneMetrics.step) {
                Text(workspace.name)
                    .font(.system(size: 15, weight: .medium))
                    .lineLimit(2)
                    .multilineTextAlignment(.leading)
                    .truncationMode(.middle)
                    .frame(maxWidth: .infinity, alignment: .leading)

                // What this workspace last said, which is the whole reason a
                // card beats a list row. Mono because it came off a machine.
                //
                // `Spacer` AFTER it rather than before: a tail of one line and
                // a tail of three must both sit under the name, or the text
                // floats at a different height on every card and the grid stops
                // being scannable. The empty case still spaces correctly
                // because the spacer is doing the work either way.
                VStack(alignment: .leading, spacing: 1) {
                    ForEach(Array(workspace.tail.suffix(3).enumerated()), id: \.offset) { _, line in
                        Text(line)
                            .font(.system(size: 9, design: .monospaced))
                            .foregroundStyle(.tertiary)
                            .lineLimit(1)
                            .truncationMode(.tail)
                            .frame(maxWidth: .infinity, alignment: .leading)
                    }
                }

                Spacer(minLength: 0)

                // The same ribbon the bar carries, at 5 rather than 6 — the
                // card is a smaller thing saying the same thing, and a second
                // way of summarizing a workspace would be a second thing that
                // can disagree with the bar about it.
                //
                // No current mark: `current` is -1 because the card is a
                // WORKSPACE, and which tab you are on inside it is not a fact
                // about the workspace. The amber outline says which workspace
                // you are in.
                ShellRibbon(tabs: workspace.tabs, current: -1, size: 5)

                Text(Self.subtitle(for: workspace))
                    // Tabular, like every number in this app: a count that
                    // changes width as it changes value makes a grid of cards
                    // twitch.
                    .font(.system(size: 12).monospacedDigit())
                    .foregroundStyle(.secondary)
            }
            .padding(PaneMetrics.card)
            .frame(width: 168, height: 132, alignment: .topLeading)
            .background(Color.white.opacity(0.06))
            .clipShape(RoundedRectangle(cornerRadius: 16))
            .overlay(
                RoundedRectangle(cornerRadius: 16)
                    // Amber, and only on the one you are in. It is the same
                    // "you are here" the elongated mark is in the ribbon, at
                    // the scale where a mark would be lost.
                    .stroke(
                        isCurrent ? Color.orange.opacity(0.7) : Color.white.opacity(0.08),
                        lineWidth: isCurrent ? 1.5 : 1))
        }
        .buttonStyle(.plain)
        .accessibilityIdentifier("shell-card-\(workspace.id)")
        .accessibilityLabel(workspace.name)
    }
}

/// The grid, its search, and the way out.
///
/// A `LazyVGrid`, and the pane invariant does not reach here. The rule that
/// forbids a lazy container — `WorkspaceView.swift:83-97`, and the three
/// comments the mechanics doc cites — is about PANES: a recycled pane loses a
/// scroll position, a half-typed message, an open stream. A card holds a name
/// and some dots, it is rebuilt from the fleet every time the fleet changes
/// anyway, and forty of them realized at once is forty views' worth of layout
/// for a surface somebody is about to leave.
struct ShellOverview: View {
    let fleet: ShellFleet
    let current: Int
    @Binding var search: String
    let onOpen: (Int) -> Void
    let onDismiss: () -> Void

    private var order: [Int] { fleet.overviewOrder(matching: search) }

    var body: some View {
        overviewBody
            // A ground of its own, and nearly opaque.
            //
            // Without one this view is transparent, so the page and the bar
            // behind it — which recede to 0.2 and 0.1, not to nothing — show
            // THROUGH the cards as legible text: a workspace name ghosted
            // across a card that names a different workspace. The prototype
            // avoids it by being 94% opaque over the app's ground, and nets
            // about a percent of bleed; glass alone nets far more.
            //
            // So: the theme's own ground at the prototype's alpha, with the
            // material above it. Depth, not a double exposure.
            .background {
                Themes.shared.current.backgroundColor
                    .opacity(0.94)
                    .ignoresSafeArea()
            }
    }

    private var overviewBody: some View {
        VStack(spacing: 0) {
            header
            if order.isEmpty {
                Spacer()
                Text("No workspace matches “\(search)”.")
                    .font(.system(size: 15))
                    .foregroundStyle(.secondary)
                Spacer()
            } else {
                ScrollView {
                    LazyVGrid(
                        columns: [
                            GridItem(.fixed(168), spacing: PaneMetrics.card),
                            GridItem(.fixed(168), spacing: PaneMetrics.card),
                        ],
                        spacing: PaneMetrics.card
                    ) {
                        ForEach(order, id: \.self) { index in
                            ShellOverviewCard(
                                workspace: fleet.workspaces[index],
                                isCurrent: index == current,
                                onOpen: { onOpen(index) })
                        }
                    }
                    .padding(.vertical, PaneMetrics.edge)
                }
                .scrollDismissesKeyboard(.immediately)
            }
        }
        // `.contain`, not a bare identifier on the stack.
        //
        // An accessibility modifier on a container that is not ITSELF an
        // element is pushed down onto every element inside it, so naming this
        // stack renamed the Done button, the search field and all forty cards
        // to "shell-overview" — a grid whose every handle was the same word.
        // Declaring it a container keeps its own name and leaves theirs alone.
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("shell-overview")
    }

    /// The count, the search, and Done.
    ///
    /// "Done" in words rather than a glyph, because the gesture that opened
    /// this is a drag and the gesture that closes it is not obvious: a person
    /// who arrived here by accident needs a way out that does not require
    /// guessing which direction to swipe. Dragging the header back down works
    /// too, which is the gesture that opened it, reversed — and it is on the
    /// header alone rather than on the whole surface because the grid below
    /// scrolls vertically, and two vertical gestures over one view is the
    /// scroll losing an argument it should always win.
    private var header: some View {
        VStack(spacing: PaneMetrics.step) {
            HStack {
                Text("\(fleet.workspaces.count) Workspaces")
                    .font(.system(size: 17, weight: .semibold).monospacedDigit())
                Spacer()
                Button("Done", action: onDismiss)
                    .font(.system(size: 15))
                    .accessibilityIdentifier("shell-overview-done")
            }

            HStack(spacing: PaneMetrics.step) {
                Image(systemName: "magnifyingglass")
                    .font(.system(size: 13))
                    .foregroundStyle(.secondary)
                TextField("Search", text: $search)
                    .font(.system(size: 15))
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .accessibilityIdentifier("shell-search")
                if !search.isEmpty {
                    Button {
                        search = ""
                    } label: {
                        Image(systemName: "xmark.circle.fill")
                            .font(.system(size: 13))
                            .foregroundStyle(.secondary)
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel("Clear Search")
                }
            }
            .padding(.horizontal, PaneMetrics.card)
            .frame(height: 36)
            .background(Color.white.opacity(0.06))
            .clipShape(Capsule())
        }
        .padding(.horizontal, PaneMetrics.edge)
        .contentShape(.rect)
        .gesture(
            DragGesture(minimumDistance: 20).onEnded { value in
                if value.translation.height > 40 { onDismiss() }
            })
    }
}
