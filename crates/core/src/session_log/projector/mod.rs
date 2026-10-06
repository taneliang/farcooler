//! One session, projected into the rows a native view draws (ov-363).
//!
//! The design is option (b) of `.claude/agent/reports/agent-architecture/
//! report.md`: the CLI session running in tmux is the truth, and this reads
//! what it writes down. The transcript is the record; hooks put rows up early
//! and the transcript confirms them; claude's session registry says whether
//! the process is busy. The fold is a port of the spike's `project.py`
//! (branch agent-architecture, 27bd9f83).
//!
//! - `record`: one line, decoded leniently and without copying what no row
//!   needs (base64 images, file dumps, thinking text).
//! - `rows`: the row vocabulary, with ids that never change.
//! - `fold`: transcript records into rows.
//! - `hooks`: hook payloads into provisional rows.
//! - `files`: the session's files read as they grow, half-written lines held.
//!
//! Pure apart from `files`, and that only reads.

pub mod files;
pub mod fold;
pub mod hooks;
pub mod record;
pub mod rows;

pub use files::{LineReader, SessionProjector};
pub use fold::{FoldStats, Projection};
pub use hooks::HookEffect;
pub use record::SubagentMeta;
pub use rows::*;

#[cfg(test)]
mod fixtures;
#[cfg(test)]
mod fold_tests;
#[cfg(test)]
mod files_tests;
#[cfg(test)]
mod hooks_tests;
#[cfg(test)]
mod parity_tests;
#[cfg(test)]
mod bench_tests;
#[cfg(test)]
mod shapes_tests;
