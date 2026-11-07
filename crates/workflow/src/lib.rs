//! Pure workflow authority. All clocks, identities and authenticated grants are inputs.
//! There are deliberately no process, filesystem, database or provider dependencies.
mod definition;
mod reducer;
mod state;
pub mod templates;
pub use definition::*;
pub use reducer::*;
pub use state::*;

use serde::Serialize;
use sha2::{Digest, Sha256};

/// Versioned JSON of closed structs/enums and sorted maps is the canonical encoding.
/// Changing a serialized shape requires a schema version change and replay review.
pub fn digest<T: Serialize>(value: &T) -> String {
    let bytes = serde_json::to_vec(value).expect("workflow values contain no failing serializers");
    format!("wf1:{:x}", Sha256::digest(bytes))
}
