# Vendor protocol artifacts

The protocol definitions the native backends are written against, committed
rather than fetched.

Refresh them with `scripts/regen-backend-types.sh`, which needs `codex`, `npm`
and `node` installed. Nothing in the build runs it: `cargo build` must not need
the network, and CI must not need npm to compile a Rust workspace. The versions
this tree was generated against are a fact about the tree, so they live in it.

| File | Source | Shape |
|---|---|---|
| `codex-app-server.schema.json` | `codex app-server generate-json-schema` | JSON Schema, generated from the exact binary installed |
| `claude-sdk.d.ts` | `sdk.d.ts` inside `@anthropic-ai/claude-agent-sdk` | TypeScript declarations |
| `PINNED` | both of the above | The versions the two files came from |

## The two are not equivalent, and that shapes the backends

**Codex publishes a schema.** `codex app-server generate-json-schema` emits it
from the binary you have, so there is no guessing and no drift between the
document and the process. That is what makes a native Codex backend cheap.

**Claude publishes no schema.** The stream-json control protocol is documented
nowhere in prose, but it *is* fully typed in `sdk.d.ts` — 205 exported types,
`SDKMessage` as a union of 39 variants, 46 `SDKControl*` types, 31 hook events —
in a file that ships with the SDK and versions with the CLI. So the protocol is
knowable; it is just not machine-generatable into Rust without writing a
TypeScript-declaration parser first.

For the handshake that needs four structs, hand-writing them against this file
is correct and a code generator would be a project of its own. When the
transcript work needs the other 35 message variants, that trade changes, and
the generator belongs to that spec rather than to this one.

## `PINNED` is what version skew is measured against

A native backend compares what the installed CLI reports at handshake against
the version recorded here. A mismatch it cannot cover is
`BackendError::Incompatible`, which leaves the pane a terminal and says so,
rather than substituting a backend the user did not choose.
