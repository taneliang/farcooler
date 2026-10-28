//! Far Cooler in a browser (ov-764, plan-763 slice 1).
//!
//! Three things, each the native client's rule rather than a second copy of
//! it:
//!
//! - **Enrollment** ([`enroll`]): the offer this browser shows and the reply it
//!   accepts, through `farcooler-ceremony` — the same rules a phone runs.
//! - **SSH** ([`ssh`]): russh on its ring backend over whatever byte stream the
//!   page hands it, with the runner's host key pinned from the reply, signing
//!   in with this browser's passkey ([`passkey`]).
//! - **The runner's protocol** ([`wire`]): the phones' own client core,
//!   `farcooler_client::session::Session`, built for the browser (ov-768).
//!
//! `js` is the wasm-bindgen surface the page calls, and only it knows there is
//! a page. Everything else builds natively too, which is how `tests/` drives it
//! against an in-process SSH server.

pub mod agent;
pub mod enroll;
pub mod error;
pub mod handshake;
pub mod liveness;
mod pages;
pub mod passkey;
pub mod register;
pub mod ssh;
pub mod wire;

#[cfg(target_arch = "wasm32")]
mod js;
