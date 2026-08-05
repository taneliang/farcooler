# Theming

**Status:** approved
**Applies to:** `crates/vt`, `crates/core`, `crates/protocol`, `crates/daemon`,
`crates/client`, `apps/macos`, `apps/ios`, `apps/android`

## The complaint

> this black theme hurts my eyes

It is not a preference the app has and defaults badly on — there is no
preference. The palette is sixteen hex constants compiled into
`crates/vt/src/grid.rs`, and two of the three apps force dark chrome around it
outright (`FarCoolerApp.swift`'s `.preferredColorScheme(.dark)`,
`Theme.kt`'s "Dark, always"). There is no light anything, anywhere, at any
setting.

## What a theme is

One value, used in two places that must not disagree:

```rust
pub struct Theme {
    pub name: String,
    /// Whether the app's own surfaces go dark. Not derived from the
    /// background's luminance: a theme author picking a mid-grey ground gets
    /// to say which way the chrome around it should go, and guessing from a
    /// single colour would flip the whole app on a one-point change.
    pub dark: bool,
    pub background: u32,
    pub foreground: u32,
    pub cursor: u32,
    /// The sixteen ANSI colours, in the order `SGR 30-37` and `90-97` name
    /// them. Everything above 16 is the xterm cube, computed rather than
    /// stored — no theme in the world hand-picks 240 shades.
    pub ansi: [u32; 16],
}
```

`dark` is what makes this one feature rather than two. The user asked for the
app chrome as well as the terminal, and a light terminal inside black chrome is
the same "two applications" problem the phones' forced dark was introduced to
avoid — stated in `Theme.kt` and in `FarCoolerApp.swift` in almost the same
words. So the theme carries the answer and the chrome follows it.

## Where the palette is resolved

`crates/vt/src/grid.rs` resolves named and indexed colours to packed RGB during
`snapshot`, and says why: *"resolved HERE rather than in each renderer, so Mac,
iOS and Android cannot drift into three different palettes."* That reasoning
holds and the location does not change. What changes is that the palette stops
being three `const`s and becomes state on `Terminal`, settable through one new
FFI call:

```c
// 19 packed 0x00RRGGBB values: 16 ANSI, then foreground, background, cursor.
void farcooler_vt_set_palette(void *handle, const uint32_t *colors, size_t len);
```

Nineteen positional `u32`s rather than a struct, because this crate's FFI is
already POD-only and a struct across the boundary would need a matching
declaration maintained by hand in Swift and again in Kotlin.

Setting it bumps the revision, so the next frame redraws — the same mechanism a
font change already uses.

Truecolor (`Color::Spec`) is untouched: a program that asked for `#12345 6` gets
`#123456` under every theme. A theme colours what the program left to the
terminal to decide, which is the whole distinction ANSI colour indices exist to
draw.

## Where themes come from

Two sources, resolved in one list.

**Built in**, compiled into `crates/core`, so every client has them with no
round trip and a phone that has never reached a host still has a light mode:

| Light | Dark | Contrast |
|---|---|---|
| Solarized Light | Solarized Dark | High Contrast Light |
| GitHub Light | Nord | High Contrast Dark |
| Tomorrow | Dracula | |
| | Gruvbox Dark | |
| | Catppuccin Mocha | |
| | Far Cooler *(today's palette)* | |

Today's palette stays, named, and stays the default. It was not among the
built-ins the user picked, and keeping it is a deliberate call rather than an
oversight: it is what every existing terminal is currently rendered in, and
changing what people are already looking at is not this feature's job. This
feature's job is to make it one tap to leave.

**Host-defined**, from `~/.config/farcooler/config.toml`, alongside the
`[adapters.*]` tables that already live there:

```toml
[themes.dim]
dark = true
background = "#1b1d21"
foreground = "#c8ccd4"
cursor = "#e6c05c"
normal = ["#424754", "#f06166", "#6bd182", "#e6c05c", "#70a9f2", "#ca8cf0", "#5cc9d1", "#d4d9e0"]
bright = ["#6b7282", "#ff8285", "#8feba1", "#fadb7d", "#94c2ff", "#e1aeff", "#82e5eb", "#f5f7fa"]
```

That file was chosen for this over a per-device setting for the reasons its own
module doc already gives: one path on every platform, dotfiles-tracked, and it
works on remote hosts that run the daemon with no Mac app. A theme written once
is offered by the Mac, the iPhone and the Android client.

Malformed entries are dropped with a warning, one theme at a time, never
failing the file — the rule `registry_from` already follows for adapters. A
theme missing `normal`/`bright` inherits the default's, so a file that only
wants to change the background says only that.

### Precedence

A host theme and a built-in with the same name: the host wins. It is the more
specific statement, and it is the one a person edited on purpose.

## Getting host themes to the clients

A new method, `themes`, returning the resolved host-defined list. Clients merge
it over their built-ins.

The Mac already reads its config through the daemon it drives, so this is one
more call in `FleetStore.seed()` — the same reconnection-seeded read that
repositories, roots and layouts use, which means a machine that drops and comes
back re-reads themes too. The phones call it once per connection.

Themes are per-machine, and the picker shows which machine a custom theme came
from. Two hosts defining different `dim` themes is not an error to resolve; it
is two machines with two opinions, and the app is connected to both.

## What the client stores

The chosen theme's NAME, per device, in each platform's ordinary preference
store (`@AppStorage` / `UserDefaults` / DataStore). Not the colours: a host
theme that gets edited should change what you see the next time you connect,
and a client that cached the values would show the old ones forever.

A name that no longer resolves falls back to the default rather than to
nothing, and says so once in the picker rather than silently — a theme that
vanished because a config file moved is worth one sentence.

## Chrome

- **macOS** already has `@AppStorage("app.appearance")` with system/light/dark.
  That preference stays and keeps meaning what it means; the theme's `dark`
  becomes its default rather than replacing it, so someone who has explicitly
  chosen "always light" is not overruled by picking a dark terminal theme.
- **iOS** drops the unconditional `.preferredColorScheme(.dark)` in
  `FarCoolerApp.swift` and takes the theme's `dark` instead.
- **Android** drops "Dark, always" in `Theme.kt`. Material You dynamic colour is
  kept where the theme is the default one and the device offers a palette —
  that is the platform-native behaviour the file argues for at length — and
  yields to the theme's own colours once a theme is explicitly chosen. Picking
  Solarized Light and getting wallpaper-derived surfaces would be the app
  ignoring what it was just told.

## Testing

`crates/core`: every built-in parses, has 16 ANSI entries, and its foreground
clears WCAG AA (4.5:1) against its background — a shipped theme nobody can read
is a bug, and the two high-contrast ones assert AAA (7:1). Config parsing gets
the same table tests `registry_from` has: a good file, a malformed theme
dropped without taking the file with it, and a partial theme inheriting.

`crates/vt`: a snapshot under two palettes gives two different packed colours
for the same indexed cell, and identical ones for a truecolor cell.

The apps get no new test target. The pickers are verified by building each app
and looking at them.

## Not doing

**Per-workspace or per-agent themes.** A colour that changes when you switch
panes is a colour that tells you nothing about the pane.

**Live-reloading config.toml.** Themes are re-read on connect and on
reconnect, which covers editing the file and restarting, and avoids a file
watcher on every host for a file that changes twice a year.

**Importing `.itermcolors` / Alacritty TOML / Ghostty themes.** Worth doing and
not now; the TOML table above is a superset of what those carry, so an importer
is a converter that can be written later without changing anything here.
