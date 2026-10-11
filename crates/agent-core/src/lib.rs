//! The vocabulary an agent's events are told in.
//!
//! `agent-hooks` folds an agent's lifecycle hooks into these events, the
//! daemon keeps them per terminal (`agent_supervisor`), and the apps decode
//! them from `terminal.agent_subscribe`. It was also what the three chat
//! backends produced, until chat mode was retired (ov-377), which is why some
//! variants are ones no hook produces: a client still decodes them.
//!
//! Few dependencies, deliberately: anything reachable from here is reachable
//! from every crate that speaks it.

pub mod event;
pub mod permission;
pub mod usage;
