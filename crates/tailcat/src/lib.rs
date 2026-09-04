//! The tunnel, and the one place that knows whether this build has one.
//!
//! Rule 1 in `farcooler-transport` says the daemon opens no network listener.
//! That stays literally true here — nothing in this crate calls `bind` on an
//! interface — but its meaning changed when this crate arrived: a runner now
//! holds an OUTBOUND DERP connection through which enrolled devices reach
//! sshd. See `docs/superpowers/specs/2026-08-31-tailcat-transport-design.md`.

use std::path::Path;

// Three backends, and exactly one of them is compiled. `linked` is the Go
// c-archive, which is what iOS and macOS ship. `helper` is a standalone,
// cgo-free Go program the daemon spawns, which is what Linux ships — see
// `helper.rs` for the musl segfault that makes the archive unusable there.
// Neither is the default: a plain `cargo build` with no Go toolchain anywhere
// gets `stub`, which fails at the one call site with the one error naming what
// is missing.
#[cfg(all(feature = "linked", feature = "helper"))]
compile_error!(
    "farcooler-tailcat: `linked` and `helper` are two ways to reach the same \
     tunnel and a build has to pick one. Linking the archive AND spawning a \
     helper would put two Go runtimes on one runner, each with its own \
     allowlist and its own idea of who is admitted."
);

#[cfg(feature = "linked")]
mod linked;
#[cfg(feature = "linked")]
use linked as backend;

#[cfg(feature = "helper")]
mod helper;
#[cfg(feature = "helper")]
use helper as backend;

#[cfg(not(any(feature = "linked", feature = "helper")))]
mod stub;
#[cfg(not(any(feature = "linked", feature = "helper")))]
use stub as backend;

#[derive(Debug, thiserror::Error)]
pub enum TunnelError {
    #[error("this build has no tunnel it can reach")]
    NoTailcatLinked,
    #[error("cannot reach the rendezvous service")]
    Derp,
    /// Tailcat ignores an unrecognized client SILENTLY, so a device removed
    /// from the allowlist gets no refusal — it gets a timeout. This is that
    /// timeout, named, so the app can say the one true thing about it: the
    /// runner did not answer, and revocation is why it might not have.
    #[error("the runner did not answer")]
    NoAnswer,
    #[error("tunnel io: {0}")]
    Io(#[from] std::io::Error),
}

impl TunnelError {
    /// The stable word that crosses the FFI. The apps own the sentence a
    /// person reads; a Rust error string must never reach a screen.
    pub fn code(&self) -> &'static str {
        match self {
            Self::NoTailcatLinked => "no_tailcat",
            Self::Derp => "derp",
            Self::NoAnswer => "no_answer",
            Self::Io(_) => "io",
        }
    }
}

pub async fn dial(
    token: &str,
    client_key: &str,
    port: u16,
) -> Result<tokio::net::UnixStream, TunnelError> {
    // Tailcat's dial always asks for this port — a runner's `OnTCP` maps it
    // to whatever port its own sshd actually uses, on loopback, which is a
    // fact only the runner has (see the design doc's "the port number is
    // virtual"). A caller that gets this wrong is told ECONNREFUSED at the Go
    // layer, indistinguishable from a dead sshd, which sends whoever reads it
    // looking in the wrong place. Catching the mistake here, in debug builds,
    // is cheap and names the actual bug instead.
    debug_assert_eq!(port, 22, "tailcat only dials port 22; got {port}");
    backend::dial(token, client_key, port).await
}

pub fn serve(key_path: &Path, ssh_port: u16, allow: &[String]) -> Result<(), TunnelError> {
    backend::serve(key_path, ssh_port, allow)
}

pub fn conn_blob() -> Result<String, TunnelError> {
    backend::conn_blob()
}

pub fn allow_add(node_key: &str) -> Result<(), TunnelError> {
    backend::allow_add(node_key)
}

/// Point this process at a different DERP map.
///
/// Process-wide rather than a parameter on every call, because it is
/// deployment configuration and not a property of one connection — the same
/// shape as the relay URL, which the relay README already calls "a client
/// setting" so that "running your own is a deploy rather than a fork".
///
/// Empty means the library default, `https://tailcat.dev/derpmap.json`, which
/// is what every build ships with. This exists because that map is documented
/// as best-effort and revocable at any time, and DERP is the rendezvous for
/// every tunneled connection: without this, recovering from a revocation would
/// mean shipping three apps and every runner. With it, recovery is a setting.
///
/// Not a promise that anyone runs their own. It is one field, taken now
/// because taking it later costs a release.
pub fn set_derp_map_url(url: &str) {
    backend::set_derp_map_url(url)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The default build has no Go archive, and must say so rather than
    /// pretending. A stub that returned Ok, or that panicked, would each be a
    /// worse answer than an error naming the one thing that is missing.
    #[cfg(not(any(feature = "linked", feature = "helper")))]
    #[tokio::test]
    async fn a_build_without_the_archive_says_which_thing_is_missing() {
        let out = dial("tc-anything", "key", 22).await;
        assert!(matches!(out, Err(TunnelError::NoTailcatLinked)));
        assert_eq!(TunnelError::NoTailcatLinked.code(), "no_tailcat");
    }

    #[cfg(not(any(feature = "linked", feature = "helper")))]
    #[test]
    fn serving_without_the_archive_is_refused_too() {
        let out = serve(std::path::Path::new("/tmp/k"), 22, &["a".to_string()]);
        assert!(matches!(out, Err(TunnelError::NoTailcatLinked)));
    }

    /// Every variant crosses the FFI as a stable word. A rename is a breaking
    /// change for an app in the field, not a tidy-up.
    #[test]
    fn every_error_has_a_stable_word() {
        assert_eq!(TunnelError::Derp.code(), "derp");
        assert_eq!(TunnelError::NoAnswer.code(), "no_answer");
        assert_eq!(TunnelError::Io(std::io::Error::other("x")).code(), "io");
    }

    /// `dial`'s port must always be 22 — see the doc comment at its one call
    /// site. A caller that gets this wrong should hear about it from a panic
    /// in a debug build, not from an `ECONNREFUSED` that reads exactly like a
    /// dead sshd.
    #[tokio::test]
    #[should_panic(expected = "tailcat only dials port 22")]
    async fn dialing_a_port_other_than_22_is_refused_in_debug_builds() {
        let _ = dial("tc-anything", "key", 2222).await;
    }
}
