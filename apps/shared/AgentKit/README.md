# AgentKit

The agent view's shared logic: decode the daemon's normalized `AgentEvent`
stream, reduce it into a renderable transcript, and parse the composer's `/`
and `@` tokens. Both the macOS and iOS apps depend on it, and neither contains
a second copy of any of it.

Shared rather than written twice for one reason. The two apps already duplicate
`Model.swift` and `VTCore.swift` by copy, and this is a larger body of logic
than either. Two copies would drift in exactly the way that makes a phone and a
Mac disagree about the same session — which is the failure the daemon-side
derivation model exists to prevent.

## Running the tests

```sh
DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer swift test
```

The `DEVELOPER_DIR` prefix is not optional on a machine whose active toolchain
is the Command Line Tools. Those do not ship `Testing.framework`, so a bare
`swift test` fails with `no such module 'Testing'` — which reads like a broken
package rather than a missing toolchain, hence this note. The same is true of
editor diagnostics: SourceKit under CommandLineTools reports the test files as
having no `Testing` module and then cascades into spurious "cannot find type"
errors across the target. `swift build` succeeding is the real signal.

Prefixing the invocation is deliberate rather than running `xcode-select -s`,
because changing the active toolchain is a machine-wide side effect that a test
run has no business having.

## What must not change

Three behaviors here are load-bearing, and each has a test whose name says so:

- **An unknown event decodes to `.gap(.unparsed)`** — never a throw, which
  would blank a whole transcript over one unrecognized event on a client a
  release behind its daemon, and never a silent skip, which produces a
  transcript that is wrong and looks complete.
- **A `Gap` row is never merged into a neighbour and never dropped.** If a gap
  could be swallowed by an adjacent message, a user would never learn that
  history is missing.
- **Message chunks of the same role coalesce into one row.** The agent streams
  a sentence as many chunks; one row each renders it as a column of one-word
  paragraphs.
