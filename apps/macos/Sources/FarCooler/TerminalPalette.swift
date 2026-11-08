import AppKit

/// Colors the renderer owns. Cell colors come from the core already resolved;
/// these are the chrome around them.
///
/// Read from the theme in force rather than fixed, and read on each access
/// rather than captured: these were `static let`s, which is what made the
/// palette unchangeable without relaunching. The cost is a dictionary lookup
/// per draw call against a value that is already in memory.
@MainActor
enum Palette {
    private static var theme: Theme { Themes.shared.current }

    static var backgroundPacked: UInt32 { theme.background }
    static var background: NSColor { theme.backgroundColor }
    /// The theme's body text, for chrome drawn ON `background`.
    ///
    /// Cells never need this — the core resolves each one's own color — but
    /// anything this app paints over a terminal ground does, and the two have
    /// to move together. `ScreenPreview` is the case that proves it: it had a
    /// hardcoded white, which was correct while this enum was `static let`s of
    /// one dark palette and became invisible the moment `background` started
    /// following the theme. Three shipped themes are `#FFFFFF`.
    static var foreground: NSColor { theme.foregroundColor }
    static var cursor: NSColor { theme.cursorColor }
    static var selection: NSColor { theme.selectionColor }

    /// Packed RGB as a `CGColor`, memoised.
    ///
    /// Called once per run of same-colored cells, in both the background pass
    /// and the glyph pass — so on a screen of syntax-highlighted output, which
    /// is what an agent produces, it is called a thousand times a frame at 120
    /// Hz. `CGColor` is a reference type and each of those was an allocation.
    ///
    /// The key is the packed value the core already resolved, and it never
    /// needs invalidating: a theme change changes which packed values arrive,
    /// not what a packed value means.
    ///
    /// Capped because the key space is not small. Sixteen palette entries and a
    /// handful of truecolor picks is the normal case, but `\e[38;2;r;g;bm` can
    /// name sixteen million colors — a program drawing an image in the
    /// terminal would name a great many of them — and an unbounded table would
    /// grow to hundreds of megabytes to save an allocation. Emptied wholesale
    /// rather than evicted one at a time: the cheap case is a screen with a few
    /// dozen colors on it, which refills in one frame.
    private static var colors: [UInt32: CGColor] = [:]
    private static let colorCeiling = 4096

    static func cgColor(_ packed: UInt32) -> CGColor {
        if let known = colors[packed] { return known }
        if colors.count >= colorCeiling { colors.removeAll(keepingCapacity: true) }
        let color = CGColor(
            srgbRed: CGFloat((packed >> 16) & 0xFF) / 255,
            green: CGFloat((packed >> 8) & 0xFF) / 255,
            blue: CGFloat(packed & 0xFF) / 255,
            alpha: 1)
        colors[packed] = color
        return color
    }
}
