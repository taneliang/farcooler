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

/// Refused, loudly, and never an empty pair.
///
/// The one entry point here where a quiet answer would be worse than an
/// error rather than merely different. Every other stub returns a failure the
/// caller cannot mistake for work done; a mint that answered with empty
/// strings would hand back something an app would store, offer, and believe —
/// producing a ceremony offer that looks valid and admits nobody. Tailcat
/// ignores an unrecognized client silently, so the symptom on the far side is
/// a tunnel that times out saying nothing, which is the failure minting was
/// built to end. Android is on this arm today, until its `.so` lands, and a
/// failed mint there is meant to degrade to an offer carrying NO node key —
/// which is the tested `v=1` path — rather than to an offer carrying a
/// worthless one.
pub fn mint_node_key() -> Result<super::NodeKeyPair, TunnelError> {
    Err(TunnelError::NoTailcatLinked)
}

/// Refused, and NOT quietly satisfied by touching a file.
///
/// The temptation here is real: creating a runner's identity looks like a file
/// write, and a stub that made an empty `tailcat.key` would let
/// `allowlist::tunnel_plan` past its `NoIdentity` guard on a build that can
/// serve nothing. That is the exact shape this repository is most careful
/// about — a check passing for work never done — and the runner would then be
/// one whose key file holds nothing a real archive could ever read.
pub fn ensure_identity(_: &Path) -> Result<(), TunnelError> {
    Err(TunnelError::NoTailcatLinked)
}

/// The one entry point that does not error, because it claims nothing.
/// Recording configuration is not reporting work done; every call that would
/// actually open a tunnel still fails above.
pub fn set_derp_map_url(_: &str) {}
