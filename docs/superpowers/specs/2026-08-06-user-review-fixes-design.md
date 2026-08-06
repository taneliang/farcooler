# User review fixes

**Status:** approved
**Applies to:** `crates/vt`, `crates/core`, `crates/protocol`, `crates/daemon`,
`crates/cli`, `apps/macos`, `apps/ios`, `apps/android`

## The review

Seven items, verbatim:

- Shift click to detect and click URLs
- Support OSC 52 escape sequences for copy paste
- Allow collapsing repos in the sidebar
- When changing focused pane, it seems to be delayed a bit
- When a new worktree is created, it should just open a terminal as well
- Allow customization of default branch name prefix
- Deleting a worktree should just automatically close existing terminals

## Where each one lands

Two of the seven have no analogue on the phones, and saying so up front is
cheaper than discovering it per-item. iOS and Android list workspaces flat —
neither groups by repository, so there is nothing to collapse — and neither has
a pane or a focus concept at all, since both use a tab strip where macOS tiles.

| Item | Lands in |
|---|---|
| Click URLs | `crates/vt`, then all three renderers |
| OSC 52 | `crates/vt`, then all three clipboards |
| Collapse repos | `apps/macos` only |
| Focus latency | `apps/macos` only |
| Terminal with a new worktree | `crates/daemon` — every client and the CLI inherit it |
| Branch prefix | `crates/core` config → protocol → all three apps and the CLI |
| Delete closes terminals | `crates/daemon` — same reason |

Three of the seven are therefore behavior the daemon owns rather than behavior
each client reimplements. That is deliberate: "a new worktree comes with a
terminal" and "removing a worktree closes what is running in it" are product
rules, not client preferences, and a rule implemented three times is a rule
three clients can disagree about.

## 1. Opening a URL

### Finding one

New `crates/vt/src/url.rs`, exposing one question the renderers ask about a
cell:

```rust
pub struct UrlMatch {
    pub url: String,
    /// Display coordinates, the same space `snapshot` reports in, so a
    /// renderer can underline the span without converting anything.
    pub start_row: u16,
    pub start_column: u16,
    pub end_row: u16,
    pub end_column: u16,
}

pub fn url_at(term: &Terminal, row: u16, column: u16) -> Option<UrlMatch>;
```

Two sources, in this order:

1. **The cell's OSC 8 hyperlink**, if the program set one.
   `alacritty_terminal` already tracks these (`Cell::hyperlink()`), and a URI
   the program stated explicitly beats one we inferred from the text — the
   whole point of OSC 8 is that the visible text and the target differ.
2. **A regex sweep**, via `Term::regex_search_left` and `regex_search_right`
   from that point, then `Term::bounds_to_string` for the text.

The regex path uses the emulator's own search rather than scanning a snapshot
row, and that is the load-bearing choice here: long URLs wrap across rows
constantly in agent output, and a per-row scan finds the first half of one and
calls it a URL. The emulator knows which lines are continuations of which.

Coordinates convert through the display offset exactly as `grid::snapshot`
does — `Point { line: Line(row as i32 - display_offset), column }` — so a URL
found while scrolled back is the one under the pointer rather than the one that
happens to be at the same screen position on the live grid.

### Which schemes

An explicit allowlist, and a short one:

```
http https mailto ftp ftps ssh git news file gemini gopher ipfs ipns magnet
```

Terminal output is not trusted input. An agent working overnight prints
whatever it read, and some of what it read came off the internet. A URL opener
that honors arbitrary schemes is a local-application-launch primitive driven by
text an attacker may have chosen, so the set of schemes is stated here rather
than inherited from whatever the platform will dispatch. Nothing is ever opened
without the modifier and the click: there is no auto-linkification that a
stray click could fire.

### The FFI

```c
typedef struct {
  uint16_t start_row;
  uint16_t start_column;
  uint16_t end_row;
  uint16_t end_column;
} FarCoolerVtUrlSpan;

// Returns the byte length the URL needs, 0 if there is none under the cell.
// Writes nothing when that exceeds `capacity` — call again with a buffer at
// least that large, the same contract `farcooler_vt_encode_paste` uses.
size_t farcooler_vt_url_at(void *handle, uint16_t row, uint16_t column,
                           FarCoolerVtUrlSpan *span,
                           uint8_t *out, size_t capacity);
```

`include/farcooler_vt.h` and `tests/header.rs` change together, as always.

### The gesture

**macOS: ⌘-click**, not shift-click as written in the review.

Shift-click is already spoken for, and load-bearing: `mouseDown` treats shift
as "I want to select, not click", which is the only way to copy text out of a
full-screen program that has grabbed the mouse. Overloading it would make one
gesture mean two things depending on what happens to be under the pointer, and
would cost the ability to start a selection on a line containing a URL — which
in agent output is a lot of lines. ⌘-click is what Terminal.app and iTerm2 use,
so it is the gesture people already have, and it is free.

Concretely, in `TerminalRenderView`:

- An `NSTrackingArea` (`.mouseMoved`, `.mouseEnteredAndExited`,
  `.activeInKeyWindow`) — the view has none today — plus `flagsChanged`, so the
  view knows both where the pointer is and whether ⌘ is down.
- With ⌘ held over a URL: underline the span and set `NSCursor.pointingHand`.
  The underline is drawn from the span, so a wrapped URL underlines on both
  rows.
- ⌘-click opens it through `NSWorkspace`. `mouseDown` checks this before
  anything else, so a program tracking the mouse never sees the click.

**iOS and Android: long-press over a URL** offers **Open Link** and **Copy
Link**. Tap already means "focus" on both and neither can afford to make that
ambiguous, so the new capability goes on a gesture with nothing to lose.

The two platforms start from different places, which is worth stating because
the diff is not symmetric:

- **Android** already long-presses the terminal to paste
  (`TerminalScreen.kt`: *"A long press pastes. A phone has no other way to"*).
  So a long-press over a URL offers the two link actions instead, and a
  long-press anywhere else keeps pasting, unchanged.
- **iOS** has no long-press on the terminal grid at all — only a tap for focus
  and a pan for scrolling — so this adds a `UILongPressGestureRecognizer`
  beside them. Over a URL it offers the two link actions; elsewhere it does
  nothing, because iOS has no terminal paste to preserve (see *Not doing*).

## 2. OSC 52

The parser already does the hard half. `alacritty_terminal` raises
`Event::ClipboardStore` for an OSC 52 write, and `Collector::send_event` in
`crates/vt/src/lib.rs` drops it on the floor with the rest of its `_ => {}`
arm. So this is a plumbing job, not an emulation job.

`Signals` gains one field, beside the title it already carries for the same
reason:

```rust
/// Text the program asked to put on the clipboard (OSC 52).
pub clipboard: Option<String>,
```

`ClipboardType::Selection` is ignored rather than mapped onto the clipboard.
It is X11's PRIMARY selection, which has no analogue on any of the three
platforms, and quietly treating it as a copy would let a program overwrite the
clipboard through a channel the user has no reason to expect one on.

Out through the ABI on the drain pattern `take_writes` already uses:

```c
// Returns the byte length the pending clipboard text needs, 0 if there is
// none. Drains only when it fits, so a short buffer cannot truncate a copy.
size_t farcooler_vt_take_clipboard(void *handle, uint8_t *out, size_t capacity);
```

Length is bounded by the parser's own OSC limit. A second cap invented here
would be a number with nothing behind it.

Each renderer writes what it drains: `NSPasteboard`, `UIPasteboard`, and
Compose's clipboard through the existing `Clipboard.kt` helpers.

### The read half stays off

`OSC 52 ; c ; ?` asks the terminal to send the clipboard's contents *to the
program*. `Config::osc52` defaults to `Osc52::OnlyCopy`, which already refuses
it, and that default stands.

This is the security posture the product's premise demands. The whole point of
Far Cooler is agents running unattended on machines you are not sitting at; a
program on one of them being able to read the clipboard of the Mac watching it
is a data path in the wrong direction, over a link that exists to carry
terminal output. Copy is a program handing you something. Paste is a program
taking something.

Worth noting this is a bigger win on the phones than on the Mac: neither iOS
nor Android has text selection in the terminal, so OSC 52 becomes the only way
to get text out of one at all.

## 3. Collapsing repos in the sidebar

`ProjectHeader` gains a disclosure chevron in `SidebarGrid.gutter` — the column
the header already reserves and currently leaves empty, so this costs no
horizontal space and lands in the column every other disclosure in the sidebar
uses. The header row becomes the button; the project's `WorkspaceSection`s and
its `HiddenWorktrees` section collapse with it. The count already on the header
is what carries the information while collapsed.

Collapsed state is keyed by the existing `groupKey(host:project:)`, not by
project name — two machines can have a project of the same name, and
`hiddenExpanded` is already keyed this way for exactly that reason.

Unlike `hiddenExpanded`, this persists across launches. Collapsing a repository
is a statement about how you want the sidebar to look, not a transient view
state, and one that resets every launch would have to be re-made every launch.
Stored in `Preferences` as a newline-joined string behind `@AppStorage`
(`sidebar.collapsedProjects`) — newline because `groupKey` already uses `\u{1}`
as its own separator and a host or a project display name cannot contain a
newline. Default expanded.

## 4. Focus latency

### What is actually slow

Every daemon action on macOS spawns a `farcooler` CLI subprocess, which
connects over a socket and runs `tmux select-pane`
(`DaemonClient.runRaw`). The focus ring, the header tint and the keyboard claim
are all driven by `PaneRect.focused`, which is a fact the daemon reports — so
none of them move until that whole round trip completes. Locally that is a
fork, an exec and a socket connect; over SSH it is all of that plus the link.
The delay in the review is that round trip, rendered.

There is a second, smaller cost stacked on it: `run()` sets `busy = true`,
which is `@Published`, so every focus click re-evaluates the entire view tree
including the terminal surface.

### The fix

Optimistic focus. `DaemonClient.focusPane` flips `focused` in its own
`layouts` copy *before* the call, then the reply replaces the whole list as it
does today:

```swift
@discardableResult
func focusPane(_ terminal: String, in workspace: Workspace) async -> [PaneGroup] {
    assumeFocus(terminal, in: workspace.id)
    return await layout(workspace, ["focus"], [terminal], background: true)
}
```

`assumeFocus` marks that pane focused, clears the flag on its siblings, and
marks its group active. Every focus path can do this because every one of them
already knows its target locally: `⌃H`/`⌃J`/`⌃K`/`⌃L` resolve through
`PaneGroup.neighbour`, `⌃B 1…9` names an index, and `⌃B o`/`⌃B ;` are a step
through a pane order the app already holds.

`layout(_:_:_:)` gains a `background:` parameter, defaulting to false, which it
forwards to `run`. Only the focus paths pass true: a split or a preset change
genuinely is the app doing something the user should see it doing, and `busy`
is how it says so.

**On failure the lie has to be cleaned up.** `layout(_:_:_:)` returns
`layouts[workspace.id] ?? []` when the command fails — which is now the
optimistically-modified copy, so a failed focus would leave the ring on a pane
that never got it. So a failed focus call ends in `refreshLayout(workspace)`,
which re-reads the truth. This is the one new failure mode the optimism
introduces and it is the one thing here that must not be left implicit.

## 5. A new worktree comes with a terminal

`WorkspaceCreate` already carries a field for this that nothing reads:
`cli_preset`, tag 4, passed as `String::new()` by all six of its call sites.
Rename it `terminal_preset` — same tag, so the wire is unchanged — and have
`workspace.create` honor it: once the worktree exists, create a terminal with
that preset, joining no layout (a fresh worktree has none to join).

Empty means no terminal, so every existing caller keeps its current behavior
and the daemon's own tests need no new expectations.

All three apps' creation sheets pass `"shell"`, matching what
"New terminal in \<project\>" already uses for the main checkout. The New
Workspace sheet is the manual path — `startTask` is the one that starts an
agent — so a shell is what you expect to land in.

The CLI gains `--terminal <preset>`, defaulting to `shell`, and
`--no-terminal`. `DaemonClient.startTask` passes `--no-terminal`, because it
creates its own agent terminal immediately afterward and would otherwise leave
every task with an unused shell beside its agent.

**A terminal that fails to start is logged, not fatal.** The worktree exists
and is useful, and failing the call would report an error for a workspace that
was in fact created — which is the worse of the two outcomes, and the one that
sends someone looking for a worktree that is already there. The returned
workspace view shows no terminals, so the failure is visible without being
reported as a failure to create the thing that was created.

## 6. The branch prefix

### Where it lives

```toml
[branches]
prefix = "feat/"
```

A table rather than a top-level key. TOML puts a bare top-level scalar written
below `[themes.paper]` *inside that table*, so `prefix = "elt/"` appended to
the end of an existing config file would silently become a theme's property and
do nothing. In a file whose whole purpose is being hand-edited, that is a trap
worth one extra line to avoid.

Read per call, the way `load_themes` already is and for the reason stated at
`rpc.rs`'s `theme.list`: a few hundred bytes of TOML parsed a handful of times
a session, in exchange for edits taking effect without a daemon restart. This
also matters for what comes next — the settings editor in phase 2 writes this
file, and a value cached at startup would not reflect its own writes.

### Getting it to the clients

`Host` gains a nested message rather than a scalar:

```protobuf
message HostSettings {
  // Prepended to a branch name derived from a task description. Empty opts
  // out entirely.
  string branch_prefix = 1;
}

message Host {
  // ... 1-9 unchanged
  HostSettings settings = 10;
}
```

Nested, because phase 2 adds more settings to this same file and a nested
message grows by a field where a bare scalar would need a migration. It rides
on `Host`, which clients already fetch, and it stays distinguishable from the
live facts (`self_health`, `live_terminal_count`) sitting beside it.

macOS reads it for free: `workspace list --json`'s envelope already carries
`runtime_healthy` and `live_panes` from the same `host_get` call, so it gains
`branch_prefix` alongside them and arrives on every `refresh()` with no extra
subprocess. Decoded as an optional field, per the rule stated at
`Model.swift:26` — a client meeting an older daemon must not fail to decode
the whole fleet over one absent key.

### Applied client-side

The prefix is prepended by the client, not by the daemon, because QuickCreate
*shows you the branch it is about to make* under the composer. A daemon-side
prefix would make that preview a lie. The daemon still validates the finished
name through `validate::branch_name`, which is the check that actually protects
git.

Taken literally beyond trimming whitespace, so `elt-` works as well as
`elt/`. No slash is added or removed.

### The default changes QuickCreate

The two macOS paths disagree today: `NewWorkspaceSheet.suggestedBranch`
hardcodes `feat/`, and `QuickCreate` uses no prefix at all. One default has to
win, and `feat/` is the one already visible to users — so **QuickCreate's
branches become `feat/<slug>`**. `prefix = ""` opts out entirely.

This is a deliberate behavior change beyond the letter of the review, and it is
the point of the item: there cannot be a customizable default until there is a
single default to customize.

## 7. Deleting a worktree closes its terminals

`Service::remove_worktree` refuses with `DomainError::RunningProcesses` while
anything is alive, and `RemoveWorkspaceSheet` renders that as "Stop the
terminals in this workspace before removing it" with the confirm button
disabled. So the user is told to go do, by hand, the thing they just asked for.

Instead: stop and then remove each terminal in the workspace, then remove the
worktree. Two steps rather than one because that is the sequence that already
works — `stop_terminal` kills the pane and sets intent `Stopped`, which makes
the subsequent `remove_terminal` pass its own running check. `remove_root`
already deletes its workspaces' terminals through `remove_terminal` for the
stated reason that a hand-rolled deletion beside it would orphan the pane
`remain-on-exit` retains; this follows the same path for the same reason.

**Everything that protects files is kept:**

- Never the main checkout — both the `is_main_checkout` flag and the
  independent path comparison, and the comment explaining why there are two.
- Refused outright when the tmux inventory is unhealthy. This one must stay
  *and* stay first: `derive_terminal` reports every terminal as `Lost` when the
  inventory cannot be trusted, so a momentarily unreachable tmux server is
  exactly the condition under which "nothing is running here" is a lie.
- A dirty worktree still demands its name typed, decided by
  `removal_needs_confirmation`.
- The branch is still never deleted.

What changes is only that a *running* terminal stops being a refusal and
becomes something the operation does. The guard's real purpose — not deleting a
directory out from under a live process — is preserved by killing the process
first, which is different from skipping the check.

`RemoveWorkspaceSheet`'s callout stops being an instruction and becomes a
statement of what the button will do, naming the count: *"Closes 2 terminals
and deletes the working directory. The branch is kept, and nothing already
committed or pushed is touched."* The `hasRunningTerminals` gate on the confirm
button goes away.

## Testing

`crates/vt`:

- A URL under a cell is found; one that wraps across two rows is found whole
  from a cell on either row.
- An OSC 8 hyperlink wins over the text under it.
- A scheme outside the allowlist is not returned.
- `url_at` on a scrolled-back view returns what is under the pointer, not what
  is at that position on the live grid.
- OSC 52 copy reaches `Signals.clipboard` and drains exactly once.
- OSC 52 with `ClipboardType::Selection` produces nothing.
- **A program asking to read the clipboard gets no reply.** This is the
  security property, so it is a test rather than a comment.
- `farcooler_vt_take_clipboard` with a short buffer reports the needed size and
  drains nothing.
- `tests/header.rs` covers the new declarations; `ffi.rs`'s own layout test
  covers `VtUrlSpan`'s size and alignment, beside the one already guarding
  `VtCell` — renderers index these by raw offset.

`crates/core`:

- `[branches] prefix` parses; a file without one yields the default; a
  malformed file does not take the prefix down with it, matching the rule
  adapters and themes already follow.

`crates/daemon`:

- `workspace.create` with a `terminal_preset` produces a workspace with one
  terminal; with an empty one, none.
- A terminal that fails to start leaves the workspace created.
- `remove_worktree` with two running terminals removes all of it.
- `remove_worktree` still refuses the main checkout, still refuses on an
  unhealthy inventory, and still demands confirmation for a dirty worktree.

`apps/macos` — checked by running the app, since none of it is unit-testable:
⌘-hover underlines and ⌘-click opens; a program's OSC 52 copy lands on the
pasteboard; a collapsed repo stays collapsed across a relaunch; focus moves on
the click rather than after it, and a focus against an unreachable machine
snaps back rather than lying.

## Not doing

- **Shift-click for URLs**, for the reason in item 1. ⌘-click instead.
- **OSC 52 paste.** Item 2 says why.
- **Collapsing repos, or pane focus, on iOS and Android.** Neither concept
  exists on either platform.
- **A theme or adapter editor.** That is phase 2, below.
- **Pasting into a terminal on iOS.** Found while designing item 1 and left
  alone: iOS can put text on the clipboard but has no way to send it to a
  terminal, where Android long-presses to paste. Nothing in the review asks for
  it and OSC 52 does not depend on it, so it is recorded here rather than
  bundled in.

## What comes next

The review surfaced a second, larger request: a per-machine preferences UI, so
the daemon-side settings in `config.toml` can be changed without hand-editing
the file on every machine. Agreed scope, to be brainstormed on its own once
this lands:

- **Everything editable**, including a theme editor (nineteen colors, live
  preview, duplicate-a-built-in to start) and an adapter editor (`program`,
  `args`, `env`, and the four detection arrays).
- **All three apps** get the editing UI, not just macOS.
- **After this spec ships**, building on the read path item 6 establishes.

The pieces item 6 puts in place on purpose, because phase 2 needs them: a
`HostSettings` message that grows by a field, and a config file read per call
rather than cached, so an editor's writes are visible without a restart. What
phase 2 adds is the write half — format-preserving, because this file is
documented as dotfiles-tracked and hand-edited and a UI that eats its comments
would be worse than no UI.
