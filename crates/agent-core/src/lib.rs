//! The vocabulary every backend produces, and the confinement they all need.
//!
//! A backend crate depends on this and on nothing else in the workspace. That
//! is what stops one protocol's types leaking into another's, and it is why
//! this crate has so few dependencies: anything reachable from here is
//! reachable from all three backends, so what goes in is worth being careful
//! about.

pub mod backend;
pub mod event;
pub mod fs_guard;
pub mod markup;
