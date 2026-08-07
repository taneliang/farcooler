# Pasting an image into a terminal

**Status:** approved
**Applies to:** `proto`, `crates/protocol`, `crates/tmux`, `crates/daemon`,
`crates/client`, `crates/cli`, `apps/macos`, `apps/ios`, `apps/android`

## The problem

A person looking at a Far Cooler terminal on their phone has a screenshot they
want the agent in that pane to look at. There is no way to give it to them.

Terminal paste today is text and only text: `TerminalRenderView.paste(_:)` reads
a string off the pasteboard, hands it to `VTCore.encode(paste:)`, and sends the
bytes. Agent panes already accept images — `AgentComposer.onPasteImage` builds an
`ImageBlock` and it rides in an `AgentPrompt` — but that path exists because ACP
has a slot for image bytes. A PTY does not. Claude Code and Codex both read an
image by absolute path in the prompt, and neither can see a phone's clipboard.

So the whole feature is: get the bytes onto the machine, write them to a file,
and type that file's path into the terminal.

## Where each piece lands

| Piece | Lands in |
|---|---|
| The transfer: chunking, retry, progress, cancel | `crates/client` — written once |
| Writing the file and typing the path | `crates/daemon`, new `pastes.rs` |
| Asking a pane whether it wants bracketed paste | `crates/tmux` |
| `terminal paste-image` | `crates/cli` |
| Entry points and the progress chip | all three apps |

`crates/client` is the shared client core for iOS, Android, macOS *and* the CLI —
its own doc comment states the bet: "put the part that must not differ between
platforms in one place, and give the platforms a narrow boundary." A chunk loop
with retries and a cancel is exactly that part. Each app writes a chip and an
entry point; none of them sees a chunk.

## The daemon types the path, not the client

This is the load-bearing decision, and it is worth being explicit about why,
because the obvious alternative — return the path, let the client paste it as
text through the input path it already has — looks simpler.

Three reasons it is not:

- **The bracketing is decided where the pane is.** Whether to wrap in
  `ESC[200~` depends on whether the program set bracketed paste mode, and the
  daemon can ask tmux. A client can also know, since it runs a VT over the
  output stream, but then the rule lives in three renderers and a CLI that has
  no VT at all.
- **The CLI gets it for free.** `farcooler terminal paste-image` is an agent
  pushing a chart into a sibling pane. Nothing on that side wants to link a
  terminal emulator to type a path.
- **It saves a round trip on the link that has none to spare.** A phone would
  otherwise wait for the path to come back before sending it as input, adding a
  full RTT after an upload it already waited through.

## 1. The wire

One new method, `terminal.paste_image`, at `CONTROL` scope. It writes to a
terminal, so it is exactly as privileged as `terminal.write` and no more.

```proto
message TerminalImagePut {
  bytes terminal_id = 1;
  // Client-generated. Names the partial file, so a second paste into the same
  // pane cannot append to the first one's bytes.
  bytes transfer_id = 2;
  string mime = 3;
  // Known up front, so an oversize send is refused before the first chunk
  // rather than after the last one.
  uint64 total_size = 4;
  // Must equal the bytes already stored. A retry that matches is a no-op; a
  // mismatch fails the transfer rather than writing at a guessed position.
  uint64 offset = 5;
  bytes chunk = 6;
}

message TerminalImagePutResult {
  uint64 stored = 1;
  // Final chunk only: the absolute path on the machine, already typed.
  optional string path = 2;
}
```

`TerminalImagePut` takes `Request.payload` tag 65, `TerminalImagePutResult`
takes `Result.value` tag 31.

Chunks are 128 KiB. This shares a connection with live terminal output, and
`attachment_get` already caps its reads at 256 KiB with a comment saying why:
one huge frame stalls panes mid-render. A ten-megabyte screenshot in a single
frame would do it for seconds.

Progress needs no server events. The client is the sender, so bytes acked over
bytes total is a number it already has.

A transfer does not survive a disconnect. The partial file is abandoned, the
sweep collects it, and the client starts over — resumable upload is machinery
for a file size this feature caps below 16 MB.

## 2. The file

Bytes land in `$FARCOOLER_HOME/pastes/.incoming/<transfer_id hex>` and are
renamed into place only when the last chunk arrives. Nothing that reads the
directory — an agent, the sweep, a person with a Finder window — can ever see a
half-written PNG, because a half-written PNG never exists under a name that
looks finished.

The final name is `YYYY-MM-DD-HHMMSS.png`, with `-2`, `-3` appended on collision
within the same second. Legible was the point: this path gets echoed back by the
agent, read on a phone screen, and scrolled past in someone's history. A content
hash would deduplicate repeated pastes and would reuse `review::put_attachment`
nearly unchanged, but it produces a sixty-four character path that means nothing
to the person reading it.

The paste directory is deliberately **not** the review attachment store, despite
the resemblance. Different directory, different naming, different lifetime, and
review attachments are referenced by database rows that a seven-day sweep would
break. `pastes.rs` sits beside `review.rs` and shares nothing but a shape.

### What is allowed in

The daemon **sniffs the magic bytes and ignores the claimed mime**, deriving the
extension from what it actually found. PNG, JPEG, GIF and WebP pass; anything
else is refused. A client should not be able to name arbitrary content `.png` on
someone's machine, and the mime string is a claim by the sender. `transfer_id`
is formatted as hex before it becomes a filename, so it cannot be a path.

`review::image_dimensions` already sniffs PNG and JPEG headers for its own
reasons; the sniffing here is a separate, stricter function in `pastes.rs`
because refusing a file is a different job from measuring one.

### The sweep

On daemon start, and once a day after that: delete pastes with an mtime older
than seven days, and `.incoming/` entries older than an hour.

Seven days rather than one because conversations get resumed. An agent that read
an image yesterday and is asked about it this morning should still find the file
where its own transcript says it is.

## 3. Typing the path

The daemon writes `<absolute path>` followed by a single space, through
`vt::encode_paste`, via `Service::send_bytes`.

A trailing space so the next word does not glue to `.png`. **Never a newline** —
nothing is submitted on anyone's behalf, on any surface, ever.

`encode_paste` brackets only when the program asked, and its existing test
(`paste_is_bracketed_only_when_the_program_asked`) is the reason to reuse it
rather than reimplement the wrapping. It needs to be told the mode, and the
daemon cannot currently answer: `TmuxWindows::pane_modes` asks tmux for
`alternate_on`, the mouse flags, cursor and keypad and wrap — not for bracketed
paste.

tmux does expose it, as `#{bracket_paste_flag}`. So `crates/tmux` gains a small
separate query rather than a tenth field on `pane_modes`:

```rust
/// Whether the pane's program has asked for bracketed paste.
///
/// Separate from `pane_modes` on purpose: that returns the escape sequences
/// that RESTORE a pane's modes in a fresh emulator, and putting bracketing
/// into that string changes what every replaying client does. This is one
/// question asked at paste time by the one caller that needs it.
pub async fn pane_bracketed_paste(&self, pane_id: &str) -> Result<bool>;
```

The alternative was tmux's own `paste-buffer -p`, which inserts the brackets
itself "if the application has requested bracketed paste mode" — correct, and
one command instead of two. It was not chosen because it routes the path through
tmux's global buffer list, which is shared state visible to anything else on the
socket, and because it means this feature writes bytes to a pane by a mechanism
no other write in the daemon uses.

## 4. The entry points

### macOS

`TerminalRenderView.paste(_:)` gains a branch ahead of its existing string path,
under a deliberately conservative rule:

- The pasteboard has a string → **paste text, exactly as today.** Unchanged.
- No string, but an image → transfer.
- File URLs pointing at images → transfer their **contents**, not their paths.

The third case is the one worth stating. A path on the local Mac means nothing on
a remote machine, so pasting it as text would produce a file reference the agent
cannot open. Dragging an image onto the pane goes through the same door.

Even against a local daemon the bytes make the round trip and land in the paste
directory. Handing over the original path would be faster and would mean two
behaviors — one file the sweep owns and one that lives forever — differing by
something the user cannot see.

### iOS and Android

Paste when the clipboard holds an image, plus a picker: on iOS in the keyboard
accessory row `TerminalView` already hosts, on Android in the key row.

HEIC converts to JPEG q0.9 on the way out. Both agents refuse HEIC, so an
untouched iPhone photo would otherwise land as a file the agent will not open.
JPEG rather than PNG because HEIC comes from the camera — PNG would triple the
size of a twelve-megapixel photo for no gain. iOS screenshots are already PNG and
pass through untouched.

Nothing else is re-encoded. A screenshot of small text is usually the entire
reason someone is sending one.

### The chip

One per transfer, stacked when several are in flight: thumbnail, determinate
ring, Cancel. It disappears as the path is typed. On failure it stays, with the
reason and Retry.

Native UI in all three apps, overlaid on the pane. Nothing about a transfer is
ever written into the terminal — the only bytes that reach the PTY are the path.

Transfers are independent. Two images type their paths in completion order.

### Typing during a transfer

Keystrokes go through immediately and the path lands wherever the cursor is when
the upload completes, which on a slow link can be the middle of a word.

The alternative was queueing printable keys locally and replaying them after the
path, which produces a clean line but makes the keyboard feel dead for the length
of the transfer. Keys that do nothing are a worse failure than text in an odd
place: one is recoverable by looking at what happened, the other looks like the
app has hung. The chip is on screen throughout, so there is always something
saying why.

### Refusals

Above 16 MB the client refuses before sending anything, so a huge paste fails
instantly rather than after a minute of cellular upload.

## 5. The CLI

```
farcooler terminal paste-image <terminal> <file>
```

Beside `send`, `screen` and `agent-prompt --image`, which already takes image
paths. It reads the file, sniffs it, streams the same chunks, and prints the host
path on stdout — which `main.rs` reserves as the data channel, so an agent can
capture it.

Being an RPC rather than a byte stream, it needs no `proxy()`: `connect_to(host)`
already reaches a remote daemon over ssh, the same as `seen` and `stop`.

The target is explicit. Injecting `FARCOOLER_TERMINAL_ID` into every pane would
let an agent address its own, but an agent pasting into its own pane feeds itself
its own stdin, and the plumbing would have to answer for panes the daemon adopted
rather than spawned.

## 6. Failures

No raw error text reaches a person. The chip says one of:

| Cause | Copy |
|---|---|
| Over the cap | That image is too large to send. Images up to 16 MB work. |
| Sniff refused it | Far Cooler can send PNG, JPEG, GIF, and WebP images. |
| Connection lost | Couldn't reach this machine. |
| Write failed on the host | Couldn't save the image on this machine. |
| Terminal gone | That terminal isn't running anymore. |

Buttons are Retry and Cancel.

## 7. Tests

**`crates/daemon`**

- An offset that does not match what is stored is refused, not written.
- The sniffed type beats the claimed mime, and picks the extension.
- Two pastes in the same second produce `…-141233.png` and `…-141233-2.png`.
- The sweep deletes at seven days and leaves six.
- No file under a final name exists until the last chunk lands.
- Oversize is refused before a byte is written.
- The typed bytes are bracketed only when the pane asked, end in a space, and
  contain no newline.

**`crates/tmux`**

- `pane_bracketed_paste` parses tmux's flag, and a malformed reply is an error
  rather than a guessed `false` — `parse_modes` already sets this precedent, for
  the same reason.

**`crates/client`**

- Progress is monotonic.
- Cancel mid-transfer types nothing into the terminal.
- A disconnect aborts the transfer rather than silently resuming it.

**`crates/daemon/tests/rpc_over_socket.rs`**

- Put an image, read the screen, assert the path appears; assert the file on
  disk is byte-identical to what was sent.

## Assumptions worth revisiting

- **HEIC converts to JPEG q0.9.** If someone is sending HEIC screenshots rather
  than photos, PNG would be the better target.
- **16 MB.** Chosen to be larger than any screenshot and smaller than a ProRAW
  photo. It is a client-side check, so raising it costs nothing on the host.
- **Seven days.** If people resume week-old conversations and find broken paths,
  this is the number that was wrong.
