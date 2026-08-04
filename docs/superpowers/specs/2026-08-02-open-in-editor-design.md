# Design: Open a worktree in an editor

Date: 2026-08-02
Status: APPROVED (design)
Extends `docs/farcooler-design.md`.

## Problem

Far Cooler knows where every worktree is — the path is right there in the
overview, selectable, in monospace — and does nothing with it. Opening one in an
editor means selecting that path, switching to a terminal or a Finder window,
and pasting it somewhere. For a worktree on a remote machine there is no gesture
at all; the path is real but it is real somewhere else, and the user has to know
their editor's remote syntax and type it themselves.

This is the one thing you do with a worktree that Far Cooler cannot do, and it
is the thing you do immediately after looking at what an agent wrote.

## What we are building

A control that opens the selected worktree in an editor, in the window's title
bar and in the sidebar row's `…` menu. It remembers which editor you used last.
It works for worktrees on other machines by using each editor's own SSH
remoting, and it says plainly when an editor has none.

## An editor

```swift
struct Editor: Identifiable, Hashable, Codable {
    var id: String        // "zed", "vscode", "custom:<uuid>"
    var name: String      // "Zed"
    var local: [String]   // argv; {path} substituted
    var remote: [String]? // argv; {path} and {host} substituted. nil = local only
}
```

An argv array rather than a shell command line. A worktree path containing a
space is ordinary — `~/Dev/My Project/feat-auth` — and a shell string makes
quoting the caller's problem in a place where getting it wrong silently opens
the wrong directory, or two. There is no shell here, so there is nothing to
quote.

`remote` being optional and not defaulted is the whole gate: an editor that
cannot reach another machine says so by having nothing to say.

## Finding editors

Through app bundles, never `PATH`.

`CLI.swift` already documents why, for the same reason in a different place: a
double-clicked app inherits no shell environment, so a `code` that works in your
terminal is a binary this process cannot find. `PATH` lookup would make the
feature work for people who launch the app from a shell and fail for everyone
else, which is the worst available outcome because it is invisible to whoever
built it.

So each built-in carries a bundle identifier and the path to its launcher
*inside* the bundle. Detection is
`NSWorkspace.shared.urlForApplication(withBundleIdentifier:)`, then
`isExecutableFile` on the launcher. This also finds editors installed outside
`/Applications`, which `PATH`-independent hardcoded paths would not.

A wrong bundle identifier fails silently — the editor simply never appears, and
nobody can tell whether it is a bug or a machine without that editor. Each entry
therefore also carries its application name, and detection falls back to scanning
`/Applications` and `~/Applications` for it.

## The table

| Editor | Local | Remote |
|---|---|---|
| Zed | `cli {path}` | `cli ssh://{host}{path}` |
| VS Code, Insiders, VSCodium, Cursor | `--folder-uri file://{encodedPath}` | `--folder-uri vscode-remote://ssh-remote+{host}{encodedPath}` |
| Sublime Text | `subl {path}` | — |
| Nova | `nova {path}` | — |
| BBEdit | `bbedit_tool {path}` | — |
| TextMate | `mate {path}` | — |
| Xcode | `xed {path}` | — |
| MacVim, Emacs | `open -na <App> --args {path}` | — |
| JetBrains: IDEA, PyCharm, WebStorm, GoLand, RustRover, CLion, RubyMine, PhpStorm, Android Studio | `open -na <App> --args {path}` | — |

The JetBrains family has remote development, but it is JetBrains Gateway: a
separate application with its own connection flow, its own backend install on
the host, and no documented one-shot "open this remote path" invocation. An
editor whose remote story is *go and use a different application* is one this
feature is honest about not covering.

### Two things the implementation found

**The VS Code family never gets a positional path**, local or remote. Two
independent faults have the same fix:

- Remotely, VS Code cannot stat the path to learn what it is, so it guesses from
  the name: a basename containing a dot opens as a FILE. A worktree called
  `api-v2.1` arrives as a text document.
- Locally, Cursor's launcher ends in `eval "$CURSOR_CLI" "$@"`, which
  word-splits its arguments a second time. Every worktree Far Cooler creates
  lives under `~/Library/Application Support/`, so a positional path opened as
  two empty untitled files — not an edge case here but the default case.

A percent-encoded `--folder-uri` has no spaces to split on and is
unconditionally a folder. The encoding is deliberately stricter than
`.urlPathAllowed`, which permits `$ & ' ( ) * ; !` — legal in a URI, and all
still live after an `eval`. Everything outside the unreserved set is encoded.

**Some applications are opened, not run.** `IntelliJ IDEA.app/Contents/MacOS/idea`
and `Emacs.app`'s executable do not fork; they *are* the editor. Running one
directly would leave Far Cooler holding a child process for as long as the
editor is open. Those go through `open -na`, which forks and returns — the same
thing the editors that ship a launcher script do for themselves.

## Remote

`{host}` is `Workspace.host`, which the CLI fills from its own `--host`
argument (`crates/cli/src/main.rs`). It is therefore exactly the string SSH
takes — `you@box`, or a `~/.ssh/config` alias — which is exactly what both
`ssh-remote+` and `ssh://` want. Nothing parses or reassembles it.

`Workspace.worktree` is absolute, so `ssh://{host}{path}` composes without a
separator.

When `host` is non-empty and the editor has no `remote` template, the menu item
is **disabled, with the reason underneath**: "Xcode cannot open remote
worktrees." The alternative — running the local template on a remote path — either
fails obscurely or, if a directory of that name happens to exist on this Mac,
opens the wrong code. That second outcome is bad enough to design against even
though it is rare.

## The last-used editor

`@AppStorage("editors.lastUsed")`, holding an editor id. Unset, it is the first
detected editor in table order.

**If the last-used editor cannot open remote worktrees and the selected worktree
is remote, the button falls back to the first detected editor that can, and does
not overwrite the stored preference.** Zed on this Mac and VS Code on the box is
a normal way to work, and it should not require a second setting, or a
preference that thrashes every time you switch machines.

## Custom editors

Name, local command, remote command. The two command fields are argv, split on
whitespace, with `{path}` and `{host}` substituted. An empty remote command
means local-only, which is the same gate the built-ins use rather than a second
mechanism.

This is a text field rather than an application picker because the picker's
answer — `open -a <App> <path>` — cannot express flags, and flags are the entire
reason someone needs a custom entry at all.

## Surfaces

**The window title bar.** The right side of it is empty, and the title already
names the worktree, so the action about that worktree belongs beside it.

Attached in `ContentView` where the detail column is placed, rather than beside
each of the four `.navigationTitle(workspace.windowTitle)` calls. Those four sit
in three different views — `WorkspaceDetail`, `TerminalPane` (twice, nested),
`TileView` — which would each need the failure channel threaded down to them,
and two of them are in the same view and would declare the toolbar twice. The
detail column is the one place that decides which worktree the window is
showing, which is exactly what the control acts on. It reads the selection
directly rather than `currentWorkspace`, whose fallback to the first worktree in
the fleet is right for the commands that use it and wrong here, where it would
offer to open something the window is not showing.

It is a `Menu(primaryAction:)`: click opens in the last-used editor, the chevron
lists the others. An icon with a tooltip rather than a name, because the name
changes width when you switch editors and a title bar control that resizes
itself is a title bar control that moves the ones next to it.

**The sidebar row's `…` menu**, above "New terminal": the last-used editor as
one item, then a submenu for the rest. This reaches a worktree you are not
currently looking at, which the title bar cannot.

**Settings → Editors**, a new tab: which editors were detected, which is the
default, and the custom list. Its own tab rather than a row in Behavior,
because a list with an editor form in it is not a checkbox.

## Failure

A launcher that will not start, or exits non-zero, raises a banner at the top of
the window — the same weight as quick-create, because this is information to act
on rather than a decision to be interrupted for.

Its own state, **not** `client.lastError`. The design originally said that
property was "already the app's banner". It is not: it is rendered in exactly
one place, `fleetPlaceholder`, and only while no fleet has loaded — the opposite
of when this control exists. Three messages already written there ("No pane to
switch — select a terminal first." and two others) are already invisible in
practice. Writing a fourth failure into a channel with no reader would have been
worse than not reporting it.

Custom templates are where this matters. A mistyped binary path should say what
it tried to run, not do nothing.

What it cannot catch: VS Code's CLI exits 0 for almost anything, including flags
it does not recognize, and reports a failed SSH connection or a missing
Remote-SSH extension inside its own window rather than to whoever spawned it. A
zero exit means "the launcher ran". Resolving the application and its launcher
on disk before spawning is the part that can be checked honestly, and that is
what gates the menu.

## What this deliberately does not do

- **Open a file, only a directory.** The unit of work here is the worktree.
- **Terminal editors as panes.** Opening `nvim` in a new Far Cooler terminal
  would work identically local and remote, since the daemon is already on both
  machines — but it is a different feature (a terminal preset) wearing this
  one's clothes.
- **Reveal in Finder.** Adjacent, cheap, and not what was asked for.
- **Per-host editor preferences.** The remote fallback above covers the case
  that motivated it.
