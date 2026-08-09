//! The Codex app-server backend.
//!
//! `codex app-server` is a stateful, long-lived process speaking JSON-RPC 2.0
//! over newline-delimited JSON — the same backend that powers OpenAI's own
//! editor extensions. It is marked `[experimental]` in the CLI's help output,
//! which is why `backend` defaults to `acp` for the codex preset and this is
//! opt-in.
//!
//! Only the handshake is implemented. That is deliberate and is what the seam
//! spec asked for: enough to prove the boundary is cut in the right place
//! against a real binary, before the transcript work commits to it.

pub mod backend;
pub mod conn;
pub mod handshake;
pub mod normalize;
