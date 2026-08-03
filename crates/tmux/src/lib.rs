//! Private tmux server lifecycle, control mode, and the runtime inventory.
//!
//! tmux is a hard dependency, not one of two interchangeable backends. Runtime
//! state is derived from live exactly-tagged panes, and the shell escape hatch
//! is a tmux client, so there is no native-PTY fallback to switch to.
//!
//! This crate implements `farcooler_core::inventory::RuntimeInventory`. The
//! dependency points one way: `tmux` depends on `core`, never the reverse.

pub mod control;
pub mod inventory;
pub mod server;
pub mod windows;

pub use control::{Notification, parse_line};
pub use inventory::LiveInventory;
pub use server::{SESSION_NAME, TmuxServer};
pub use windows::ManagedWindow;
