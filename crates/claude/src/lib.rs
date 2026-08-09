//! The Claude Code stream-json backend.
//!
//! The `claude` CLI, run with stream-json in both directions, is what the
//! official Agent SDK itself drives — the SDK is a TypeScript wrapper around
//! this exact process. Speaking it directly is what keeps Node out of chat
//! mode, which matters because `docs/remote-hosts.md` promises an install that
//! is two static binaries and names only tmux, git and systemd.
//!
//! The protocol is documented nowhere in prose but fully typed in the
//! `sdk.d.ts` that ships with the SDK, pinned at `vendor/claude-sdk.d.ts`.
//! Every shape here was cross-checked against a live claude 2.1.226 rather
//! than read off the declarations alone.
//!
//! Only the handshake is implemented, which is what the seam spec asked for.

pub mod backend;
pub mod conn;
pub mod handshake;
pub mod normalize;
