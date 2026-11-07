//! One read-only admission check shared by every production entry that would
//! open the Store or start a writer: normal main (after the daemon lock),
//! `Service::open_selected`, `Service::open_in` (so stream and standalone stdio,
//! which call Service directly, inherit it). It writes nothing; a refusal never
//! implies recovery finished. Recovery uses `farcooler_store::engine_cutover`.
use std::path::Path;
use farcooler_core::{DomainError, Result};
use farcooler_store::engine_cutover::startup::{Admission, ordinary_admission};

pub const DATABASE: &str = "farcooler.db";

pub fn admit(root: &Path) -> Result<Admission> {
    ordinary_admission(&root.join(DATABASE)).map_err(|error| {
        tracing::error!(%error, "ordinary startup refused by the engine cutover gate");
        DomainError::OperationFailed
    })
}
