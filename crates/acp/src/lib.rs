//! The Agent Client Protocol backend.
//!
//! One of three, and the only one that is not vendor-specific: an ACP adapter
//! is how any agent without a native backend reaches chat mode, including
//! every adapter a user adds to their own config file. That is why this crate
//! is not going away when the native backends land — it is the extension
//! point, not a legacy path.

pub mod backend;
pub mod conn;
pub mod handshake;
pub mod normalize;
pub mod session;
pub mod wire;
