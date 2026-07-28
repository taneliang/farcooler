//! The shared client core.
//!
//! One implementation of "talk to an Overnight host", used by every client that
//! is not the host itself. The Mac app can shell out to `ssh` and the CLI;
//! iOS and Android cannot, so the transport, the protocol and the shape of the
//! answers all live here in Rust and each platform writes only a UI.
//!
//! This is the same bet as `crates/vt`: put the part that must not differ
//! between platforms in one place, and give the platforms a narrow boundary.

pub mod ffi;
pub mod session;
pub mod ssh;
