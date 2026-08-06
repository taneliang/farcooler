# Machine settings

**Status:** approved
**Applies to:** `crates/core`, `crates/protocol`, `crates/daemon`, `crates/cli`,
`crates/client`, `apps/macos`, `apps/ios`, `apps/android`
**Follows:** `2026-08-06-user-review-fixes-design.md`, whose item 6 established
the read path and the `HostSettings` message this builds on.

## The request

> since we have a bunch of daemon-specific configs in config.toml, sounds like
> we need to build a per-machine preference UI so that we can change these
> without having to manually change the config.toml

Three tables live in `~/.config/farcooler/config.toml` and all three are
currently editable only by opening a text editor on the machine that owns them —
which for a remote host means an ssh session before you can change a colour.

| Table | What it holds |
|---|---|
| `[branches]` | `prefix`, the one scalar, added by the previous spec |
| `[themes.<name>]` | Nineteen colours and a `dark` flag |
| `[adapters.<name>]` | `program`, `args`, `env`, and four detection arrays |

All three become editable, from all three apps, per machine.

## The write path

This is the risky part of the feature, so it is settled first: the file is
documented as hand-edited and dotfiles-tracked, and a UI that rewrites it must
not eat what is already in it.

**`toml_edit::DocumentMut`, not `toml::to_string`.** Parse, mutate one table,
serialize. Comments, blank lines, key order and formatting survive everywhere the
edit does not touch — which for a file whose module doc explains why it lives
where it does, in prose, is the difference between a feature and a data-loss bug.
`toml_edit 0.22` is already in `Cargo.lock` as `toml`'s own dependency, so this
adds a direct dependency and no new tree.

**Atomic.** Write a sibling temp file in the same directory, then `rename`. A
crash or a full disk mid-write cannot leave a truncated dotfile behind, and
`rename` within a directory is atomic on every platform the daemon runs on.

**Creates what it needs.** An absent `~/.config/farcooler/` is the common case —
the previous spec says almost nobody has this file — so a first write creates the
directory and a file containing only the table being set.

**Default umask, deliberately not `0600`.** `push.json` is written owner-only
because it holds a bearer token. This file is the opposite: it is meant to be
readable, tracked in a dotfiles repository, and copied to other machines.

**Concurrency is per table, and that is enough.** Because a write touches exactly
one table, a hand edit to a different one survives untouched. Two edits to the
*same* table race, and the last writer wins. That is stated rather than guarded:
a precondition token would have to be threaded through three apps to protect
against a case that requires someone to be editing the same theme in a text
editor and in the app at the same moment.

New in `crates/core/src/config.rs`:

```rust
/// Set `[branches] prefix`, creating the file if it does not exist.
pub fn write_branch_prefix(path: &Path, prefix: &str) -> std::io::Result<()>;

/// Write one `[themes.<name>]` table, replacing it if it exists.
pub fn write_theme(path: &Path, theme: &crate::theme::Theme) -> std::io::Result<()>;

/// Remove one `[themes.<name>]` table. Absent is success, not an error.
pub fn delete_theme(path: &Path, name: &str) -> std::io::Result<()>;

/// Write one `[adapters.<name>]` table, replacing it if it exists.
pub fn write_adapter(path: &Path, name: &str, spec: &ConfigAdapter) -> std::io::Result<()>;

/// Remove one `[adapters.<name>]` table. Absent is success.
pub fn delete_adapter(path: &Path, name: &str) -> std::io::Result<()>;
```

Delete-is-idempotent matters because it is how **Revert to Default** works: the
table goes away and the compiled-in value takes over again. Writing the built-in's
values back instead would freeze today's defaults into the user's file and mean a
later shipped improvement never reached them.

## Reload

Themes and the branch prefix are already read on every call — the previous spec
made that so, naming this feature as the reason. They need nothing.

**Adapters do.** `Service.registry` is loaded once in `Service::open_in` and held
for the life of the process, with a comment explaining why: the file and the
environment that locates it are process-global, and a service that consulted them
on every call could be moved out from under itself mid-operation, exactly as
`root` could. That reasoning is correct and is not being reversed.

So the registry becomes **swappable, reloaded on an explicit write** rather than
consulted per call:

```rust
// Was: registry: Registry
registry: std::sync::RwLock<std::sync::Arc<Registry>>,
```

`Service::registry()` hands out an `Arc` clone, so any single operation holds one
consistent registry from start to finish — the invariant the original comment was
protecting. `Service::reload_registry()` replaces it, and only the four adapter
writes call it. Editing the file by hand still needs `farcooler daemon ensure`,
which is unchanged and still documented.

## The protocol

Granular methods, one per thing the editor does, so each write touches one table
and two people editing different things never collide.

```protobuf
// One `[adapters.<name>]` table, plus where it came from.
message Adapter {
  string preset = 1;
  string program = 2;
  repeated string args = 3;
  map<string, string> env = 4;
  // Detection. Wrong values here do not fail loudly — they stop an agent being
  // recognized, which reads as "chat mode broke" from somewhere else entirely.
  repeated string commands = 5;
  repeated string identity = 6;
  repeated string blocked = 7;
  repeated string working = 8;
  AdapterOrigin origin = 9;
}

enum AdapterOrigin {
  ADAPTER_ORIGIN_UNSPECIFIED = 0;
  // Compiled into the daemon; no table in config.toml.
  ADAPTER_ORIGIN_BUILT_IN = 1;
  // A table shadowing a built-in of the same name.
  ADAPTER_ORIGIN_OVERRIDE = 2;
  // A table naming an agent Far Cooler does not ship.
  ADAPTER_ORIGIN_USER = 3;
}

message AdapterList { repeated Adapter items = 1; }

message AdapterTestResult {
  bool ok = 1;
  // What the adapter reported, when it answered — an agent name and version.
  string reported = 2;
  // Why it did not, when it did not. Never a raw Rust error: this reaches a
  // form field.
  string failure = 3;
}
```

| Method | Payload | Scope |
|---|---|---|
| `settings.set_branch_prefix` | `HostSettings` | HostAdmin |
| `theme.upsert` | `Theme` | HostAdmin |
| `theme.delete` | `TypedConfirmation` (the name) | HostAdmin |
| `adapter.list` | — | HostAdmin |
| `adapter.upsert` | `Adapter` | HostAdmin |
| `adapter.delete` | `TypedConfirmation` (the preset) | HostAdmin |
| `adapter.test` | `Adapter` | HostAdmin |

**Every one is `HostAdmin`**, including the reads. These write to a file in the
user's home directory on a possibly-remote machine, and `adapter.list` reveals
`program`, `args` and `env` — which is to say local paths and, for an agent that
needs one, an API key. `Scope::Read` is for the shape of the fleet.

`theme.delete` and `adapter.delete` reuse `TypedConfirmation` rather than gaining
a message each: the payload is one string and the existing message already
carries exactly one string. The field is the name to delete, not a
confirmation of intent — deleting a theme touches no files and is undone by
picking it again, so it needs no typed gate.

**Themes need no new read.** `theme.list` already returns this host's themes, and
every client compiles in the eleven built-ins, so "this host theme shadows a
built-in" is a name comparison the client already has both halves of. Adapters
are the opposite: no client can currently see the registry at all — the agent
pickers are a hardcoded three-entry UI list — which is why `adapter.list` exists.

## `adapter.test`

`crates/core/tests/adapters.rs` already spawns an adapter, sends an ACP
`initialize`, and reads until it answers or a timeout elapses. That function moves
into `crates/core` as `activity::handshake`, and the test calls it — so the test
and the button are one implementation rather than two that agree today.

What comes with it, unchanged and for the reasons already written down there:

- **A 90-second bound.** A cold `npx` fetches a package on first use, and the same
  cost applies here against the same real packages.
- **The read happens on its own thread**, because `BufReader::lines().next()` has
  no timeout and an adapter that starts and goes silent is an observed failure
  mode (`Status::AdapterSilent`) that would otherwise hang the call forever.
- **Lines that are not the answer to id 1 are skipped**, because adapters log
  before they answer.

This is what makes the adapter form worth having. Without it you find out your
launch command is wrong by opening a pane, pressing `⌃B a`, and getting a blank
screen — a failure that lands nowhere near the form that caused it.

**What it cannot prove**, and the UI must not imply otherwise: a successful
handshake says the adapter *starts and speaks ACP*. It says nothing about
`commands`, `identity`, `blocked` or `working`, which is where a wrong value
silently stops an agent being recognized. So the form groups its fields as
**Launch** (provable) and **Detection** (not), and a new adapter duplicated from a
built-in inherits that built-in's detection arrays rather than starting empty.

## The editors

Per machine throughout. A machine's settings live on that machine, and the fleet
routinely spans several.

### macOS

`HostsSettings` — already the surface where machines are added, probed and
installed onto — gains a disclosure on each machine's row:

- **Branch prefix**, a text field with the effective value as its prompt.
- **Themes**, a list of built-ins and host themes merged by name, each row
  marked. `+` duplicates the selected one; a row that shadows a built-in offers
  **Revert to Default**.
- **Adapters**, the same shape over `adapter.list`.

**The theme editor** is nineteen swatches over a **live terminal preview**: a
real `TerminalRenderView` fed a fixed fixture of coloured output, repainted from
the palette being edited. The preview is the point — nineteen hex values tell you
nothing, and the app already resolves cell colours inside the VT core, so
previewing means setting a palette on a throwaway core rather than reimplementing
colour resolution in a settings pane.

**The adapter editor** is a form — program, args, env, then the four detection
arrays under a heading that says they are not what Test checks — plus **Test**,
which reports the reported version or the failure.

### iOS and Android

Settings → Machines → a machine → the same three sections, and **all nineteen
colours**, as asked. On a phone the swatches are a scrolling grid, and the live
preview matters *more* there than on the Mac because the grid is small and a
swatch is smaller.

Both platforms already have a Settings screen with a Machines section and a theme
picker reading host themes, so this extends a screen rather than inventing one.

## Testing

`crates/core`:

- A write into a file with comments, blank lines and three tables leaves every
  byte outside the edited table identical. **This is the test that makes the
  feature safe**, so it is written first and asserts on the whole file rather
  than on a parse of it.
- A write to an absent path creates the directory and a file containing only that
  table.
- `delete_theme` on a name that is not there succeeds.
- Deleting the last theme leaves a file with no empty `[themes]` husk.
- A write into a file that is *malformed* is refused rather than replacing it —
  the reader already tolerates a broken file by ignoring it, and a writer that
  overwrote one would turn a typo into lost work.
- Round trip: write a theme, read it back with `themes_from`, get the same
  nineteen colours.
- `handshake` against a fake adapter that answers, one that goes silent (bounded
  by a short timeout, not 90s), and one whose program does not exist.

`crates/daemon`:

- Every new method is `HostAdmin` in the scope table, asserted the way the
  existing scope tests do — including `adapter.list`, with `theme.list` beside it
  as `Read` to make the distinction explicit.
- `adapter_origin` reports `BUILT_IN` for a name with no table, `OVERRIDE` for one
  shadowing a shipped adapter, and `USER` for one Far Cooler does not ship.

**Not socket tests, and this is a real limitation rather than an omission.**
`config_path()` reads process-global environment, and the socket harness runs its
tests in parallel — a test that pointed it at a scratch file would move it out
from under every other test in the same binary, which is the exact failure that
harness's own doc comment says it was built to avoid. So the origin decision is a
**pure function of two name sets**, tested hermetically, and the parts that
genuinely need a file — the write, the reload, and the round trip — are verified
against a scratch daemon by hand, the way the previous spec's items 5 to 7 were.
What that leaves uncovered by CI is the wiring between them.

Closing that properly means threading a config path through `Service` instead of
reading the global, which is a larger change than this feature justifies and is
recorded here rather than pretended away.

The three apps are checked by building and driving them; none of this is
unit-testable.

## Not doing

- **A precondition token for concurrent writes.** Per-table writes make the
  realistic case safe; see the write path above.
- **Editing `[adapters.*]` detection with a validator.** There is nothing to
  validate against — the strings are matched against agent output that only that
  agent produces. `Test` covers launch; detection is verified by using it.
- **A worktrees-directory setting.** Offered and not chosen when this scope was
  agreed.
- **Reading the config file per call for adapters.** The reload is explicit, for
  the reason `Service`'s existing comment gives.
