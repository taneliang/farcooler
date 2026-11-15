//! The pure halves of the adapters linked into this build (docs/engine-pipeline.md 3.2). Each is chosen by the
//! version string a published layout names, and old versions stay linked so old logs replay.
pub mod coding;
pub mod coding_templates;
pub use coding::CodingV1;
use crate::pipeline::Adapters;

static CODING_V1: CodingV1 = CodingV1;
/// Every adapter linked into this build, for `pipeline::step`.
pub fn linked() -> Adapters<'static> { Adapters::new(&[&CODING_V1]) }
