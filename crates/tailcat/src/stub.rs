//! What this crate is when no Go archive was linked.
//!
//! Every entry point returns the same error, and none of them succeeds. A stub
//! that returned Ok would be the exact failure this repository is most careful
//! about: a check that cannot fail, reporting success for work never done.

use super::TunnelError;
use std::path::Path;

pub async fn dial(_: &str, _: &str, _: u16) -> Result<tokio::net::UnixStream, TunnelError> {
    Err(TunnelError::NoTailcatLinked)
}

pub fn serve(_: &Path, _: u16, _: &[String]) -> Result<(), TunnelError> {
    Err(TunnelError::NoTailcatLinked)
}

pub fn conn_blob() -> Result<String, TunnelError> {
    Err(TunnelError::NoTailcatLinked)
}

pub fn allow_add(_: &str) -> Result<(), TunnelError> {
    Err(TunnelError::NoTailcatLinked)
}

/// The one entry point that does not error, because it claims nothing.
/// Recording configuration is not reporting work done; every call that would
/// actually open a tunnel still fails above.
pub fn set_derp_map_url(_: &str) {}
