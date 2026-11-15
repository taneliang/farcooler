//! The pipeline core (docs/engine-pipeline.md, schema 3): one pipeline that any kind of work flows through.
//!
//! Pure and neutral. It knows stations, queues, capacity, pools, holds, batches, visits, decisions, outward
//! steps and crossings; what a token is about (a ticket, a mail thread, an article) belongs to an adapter,
//! whose data the core stores and passes but never reads. A test (`tests/pipeline_vocabulary.rs`) keeps
//! every coding word out of this module. Nothing in the daemon uses it yet (ov-1266).
mod adapter;
mod batch;
mod control;
mod flow;
mod layout;
mod publish;
mod step;
mod types;

pub use adapter::*;
pub use layout::*;
pub use step::*;
pub use types::*;
