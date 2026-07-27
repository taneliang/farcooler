//! Wire protocol: generated protobuf types plus length-delimited framing.
//!
//! Unix sockets and SSH stdio carry the exact same stream:
//! `envelope_length:u32` in network byte order followed by one serialized
//! `WireEnvelope`. The framing stays transport-portable.

pub mod framing;
pub mod ids;

/// Generated protobuf types.
pub mod v1 {
    include!(concat!(env!("OUT_DIR"), "/overnight.v1.rs"));
}

pub use v1::*;

/// Protocol major version this build speaks.
pub const PROTOCOL_VERSION: u32 = 1;

/// Control envelopes are capped at 1 MiB.
pub const MAX_CONTROL_ENVELOPE_BYTES: usize = 1024 * 1024;

/// `TerminalFrame.payload` is capped at 64 KiB.
pub const MAX_TERMINAL_PAYLOAD_BYTES: usize = 64 * 1024;

/// Each terminal retains the most recent 8 MiB of its current epoch.
pub const REPLAY_BUFFER_BYTES: u64 = 8 * 1024 * 1024;

/// Per-client terminal output window: unacknowledged data cap.
pub const MAX_UNACKED_TERMINAL_BYTES: u64 = 1024 * 1024;

/// Control channel ceiling: queued unwritten control envelopes per client.
pub const MAX_QUEUED_CONTROL_BYTES: u64 = 4 * 1024 * 1024;

/// Clients acknowledge at 256 KiB unacknowledged or 250 ms, whichever first,
/// and always within 1 s of output that leaves anything unacknowledged.
pub const FLOW_ACK_BYTES_THRESHOLD: u64 = 256 * 1024;
pub const FLOW_ACK_INTERVAL_MS: u64 = 250;
pub const FLOW_ACK_MAX_DELAY_MS: u64 = 1000;

/// A disconnected writer lease expires after 30 seconds.
pub const WRITER_LEASE_EXPIRY_SECS: u64 = 30;

/// Terminal dimensions are clamped to these ranges.
pub const MIN_COLUMNS: u32 = 20;
pub const MAX_COLUMNS: u32 = 500;
pub const MIN_ROWS: u32 = 5;
pub const MAX_ROWS: u32 = 200;
