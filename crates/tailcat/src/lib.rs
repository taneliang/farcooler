//! The tunnel, and the one place that knows whether this build has one.
//!
//! Rule 1 in `farcooler-transport` says the daemon accepts no unauthenticated
//! connection and binds no port on any interface. Nothing in this crate calls
//! `bind` on an interface, so the second half is untouched — but this crate is
//! why the rule had to be written that way rather than as "opens no network
//! listener": a runner now holds an OUTBOUND DERP connection through which
//! admitted devices reach sshd, and a rule phrased around `bind` alone would
//! have gone on reading as "unreachable" while that was no longer the case.
//! See `docs/superpowers/specs/2026-08-31-tailcat-transport-design.md`.

use std::path::Path;

// Three backends, and exactly one of them is compiled. `linked` is the Go
// build linked into this process, which is what iOS, macOS and Android ship:
// a static `c-archive` on the two Apple platforms, and a `c-shared`
// `libtailcat.so` on Android, because `go build -buildmode=c-archive` refuses
// GOOS=android outright. Both come from `go/exports_cgo.go` and export the
// same symbols, so `linked.rs` declares one set for both. `helper` is a
// standalone, cgo-free Go program the daemon spawns, which is what Linux
// ships — see `helper.rs` for the musl segfault that makes the archive
// unusable there.
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

/// A node key pair, both halves in memory and neither of them on disk.
///
/// The shape exists because this is the one thing in the crate that is a
/// DEVICE's rather than a runner's, and the difference is the whole point:
/// `serve` is handed a path and the runner's private half lives in a 0600
/// file it owns, while this is handed nothing and returns both halves by
/// value for its caller to put wherever that platform keeps secrets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeKeyPair {
    /// This device's node PRIVATE key, and the only secret in this crate that
    /// crosses a function boundary. 43 characters of unpadded base64-URL,
    /// which is what `dial`'s `client_key` argument takes.
    pub private_key: String,
    /// The public half, in the exact spelling `farcooler_fence::usable_node_key`
    /// accepts: 43 characters of unpadded base64-URL. This is what goes in a
    /// ceremony offer and, from there, into a runner's allowlist.
    ///
    /// Not a secret. A node public key names which peer a tunnel will admit,
    /// and holding it grants nothing.
    pub public_key: String,
}

/// Mint this device's tunnel identity: a fresh node key pair, returned.
///
/// **It takes no path, and that is a decision rather than an omission.** The
/// Go functions underneath are path-based — a runner has a home directory and
/// writes `tailcat.key` at 0600 — but iOS keeps private keys in the Keychain
/// on purpose, "not in UserDefaults and not in a file"
/// (`apps/ios/FarCooler/Store.swift:7-8`), because the Keychain is the only
/// iOS store that survives a backup restore. A path argument here would put a
/// node private key in a file and quietly reverse that. Returning by value is
/// also what the rest of this crate already expects: `dial` takes the client
/// key as a string.
///
/// Once per DEVICE, not once per runner. The node key is the device's
/// identity — `RunnerStore` (`crates/cli/src/runner_pipe.rs`) already holds it
/// that way for the desktop, once rather than inside each `Reach::Tailcat`.
///
/// Minting is not serving: this touches no server and needs none running.
pub fn mint_node_key() -> Result<NodeKeyPair, TunnelError> {
    backend::mint_node_key()
}

/// Give this runner a tunnel identity, if it does not already have one.
///
/// **The runner-side sibling of [`mint_node_key`], and the difference is the
/// point of having both.** A mint returns a pair by value and writes nothing,
/// because a phone keeps its private key in the Keychain; this takes a path and
/// writes a file at 0600, because a runner has a home directory and must be the
/// SAME node after a restart — every token already sitting in a device's
/// manifest names this runner's node and its DERP region.
///
/// Idempotent, and that is what makes it safe on every pairing: a runner that
/// already has the file is untouched, keeps its key, and keeps its pinned
/// region.
///
/// **Why it exists as its own call rather than being left to `serve`.**
/// `serve` would create the file — it is the same Go function underneath — but
/// `farcooler_daemon::allowlist::tunnel_plan` refuses a runner with no key file
/// BEFORE `serve` is reached, and that guard is deliberately the only admission
/// check a build with no archive can prove anything about. So the identity is
/// created first and the guard then passes on a runner that genuinely has one,
/// rather than being bypassed for one that does not. See
/// `crates/daemon/src/enrollment.rs`.
///
/// Creating an identity is not serving: this starts nothing, admits nobody, and
/// reaches no network.
pub fn ensure_identity(key_path: &Path) -> Result<(), TunnelError> {
    backend::ensure_identity(key_path)
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
///
/// A URL with whitespace in it is refused here rather than in each backend,
/// because it is not a URL on any of them and because one of them would be
/// actively harmed by it: `helper.rs` sends this to a subprocess over a line
/// protocol that is one command per line and one reply per line. A space makes
/// a second FIELD, which would arrive as a command the helper never meant to be
/// given; a newline makes a second LINE, which is worse — the helper answers it
/// too, and every reply after that is one behind, so the `serve` that follows
/// reads the stray line's `err 22` and the runner serves no tunnel for a reason
/// nothing names.
///
/// **Any whitespace, not a field count.** `split_whitespace` counts fields, and
/// a value that merely ENDS in a newline has one of them — which is exactly the
/// shape a plist `<string>` or a systemd `Environment=` with the URL on its own
/// line produces, and it was being taken. Refused, not trimmed: a value
/// somebody typed wrong should stay unset rather than become a different URL
/// nobody chose.
pub fn set_derp_map_url(url: &str) {
    if url.chars().any(char::is_whitespace) {
        tracing::warn!("tailcat: the DERP map URL has whitespace in it; ignoring it");
        return;
    }
    *recorded_derp_map_url().lock().expect("the DERP map URL lock") = url.to_string();
    backend::set_derp_map_url(url)
}

/// What this process was last told to use, or empty for the library default.
///
/// **It answers what was recorded, not what a tunnel is currently using**, and
/// the difference is worth stating because a reader who assumed otherwise
/// would be reading a weaker guarantee than they thought. The value is read
/// when a `Server` or a `Client` is built, so a `set_derp_map_url` after a
/// `serve` changes the next one and not the running one. Three separate
/// things prove the value actually lands rather than merely being remembered:
/// `helper.rs` builds the helper's `derpmap` command out of this same record
/// (`the_configured_derp_map_reaches_a_helper_before_it_serves`), the Go
/// side's `TestServerTakesTheConfiguredDERPMap` and
/// `TestTheDialingClientTakesTheConfiguredDERPMap` assert it reaches both
/// constructors, and `a_real_tunnel_carries_the_scope.rs` carries a real
/// connection over a `derper` that only a non-default map names.
pub fn derp_map_url() -> String {
    recorded_derp_map_url().lock().expect("the DERP map URL lock").clone()
}

/// Serializes every test in this crate that moves the process-wide DERP map
/// setting.
///
/// Above `mod tests` rather than inside it because `helper.rs`'s tests move
/// the same static: `cargo test` runs one crate's tests on many threads in one
/// process, and two modules each serializing only against themselves would
/// still take turns failing for a reason neither test is about. Helper tests
/// take `helper::tests::SERIAL` first and then this, always in that order.
#[cfg(test)]
pub(crate) static DERP_MAP_SETTING: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// The DERP map this process was configured with.
///
/// Here rather than in a backend because every backend needs it and one of
/// them cannot answer for itself: `helper.rs` has to remember the URL for a
/// helper that does not exist yet when the setting is made, and `stub.rs` has
/// no Go to ask. Keeping one record above the seam also means the apps and the
/// daemon read the same string back whichever build they are.
fn recorded_derp_map_url() -> &'static std::sync::Mutex<String> {
    static URL: std::sync::OnceLock<std::sync::Mutex<String>> = std::sync::OnceLock::new();
    URL.get_or_init(Default::default)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Empty is the default and empty means the LIBRARY's default.
    ///
    /// The bug this exists to catch is a helpful one: something filling in
    /// tailcat.dev's URL when the setting is unset. That reads as a
    /// convenience and is the opposite — it hardcodes the exact map the
    /// setting exists to move off, in the place nobody would look, and it
    /// keeps working right up until the day it matters.
    #[test]
    fn an_unset_derp_map_leaves_the_library_default_alone() {
        let _serial = super::DERP_MAP_SETTING.lock().unwrap_or_else(|e| e.into_inner());
        set_derp_map_url("");
        assert_eq!(derp_map_url(), "", "an empty setting must not become a URL");
    }

    #[test]
    fn a_derp_map_setting_is_remembered() {
        let _serial = super::DERP_MAP_SETTING.lock().unwrap_or_else(|e| e.into_inner());
        set_derp_map_url("https://derp.example/derpmap.json");
        assert_eq!(derp_map_url(), "https://derp.example/derpmap.json");
        set_derp_map_url("");
    }

    /// A URL with a space in it is two fields, and one of the backends sends
    /// this over a line protocol that splits on spaces — so the second field
    /// would reach a helper as a command nobody typed. Refused rather than
    /// trimmed, and the PREVIOUS value is what survives: a setting somebody
    /// typed wrong must not silently become a different rendezvous.
    #[test]
    fn a_derp_map_url_with_whitespace_in_it_is_refused() {
        let _serial = super::DERP_MAP_SETTING.lock().unwrap_or_else(|e| e.into_inner());
        set_derp_map_url("https://derp.example/derpmap.json");
        set_derp_map_url("https://derp.example/map.json allow_add cccc");
        assert_eq!(
            derp_map_url(),
            "https://derp.example/derpmap.json",
            "a DERP map URL carrying a second field was taken"
        );
        set_derp_map_url("");
    }

    /// A URL with a NEWLINE in it is refused too, and it is NOT the same
    /// mistake as the one above.
    ///
    /// `split_whitespace` counts FIELDS, and a value that ends in a newline
    /// has exactly one — so `"https://derp.example/map.json\n"` counted as one
    /// field and was taken. A plist `<string>` or a systemd `Environment=`
    /// with the URL on its own line is all it takes to produce one, and
    /// nothing between there and here trims it.
    ///
    /// The cost is not a bad URL, which would at least fail visibly. The
    /// helper backend writes this as `derpmap <url>` followed by a newline,
    /// down a pipe whose whole contract is one reply per line: the newline
    /// INSIDE the value ends the line early, and what follows it arrives as a
    /// second command the helper answers with a second reply. Every reply
    /// after that is one behind. The `serve` that follows reads the stray
    /// line's `err 22` and reports `EINVAL`, so the runner serves no tunnel
    /// and nothing anywhere names the newline.
    #[test]
    fn a_derp_map_url_with_a_newline_in_it_is_refused() {
        let _serial = super::DERP_MAP_SETTING.lock().unwrap_or_else(|e| e.into_inner());
        set_derp_map_url("https://derp.example/derpmap.json");
        for spelling in [
            "https://derp.example/map.json\n",
            "\nhttps://derp.example/map.json",
            "https://derp.example/map.json\r\n",
            "https://derp.example/map.json\t",
        ] {
            set_derp_map_url(spelling);
            assert_eq!(
                derp_map_url(),
                "https://derp.example/derpmap.json",
                "{spelling:?} was taken; it would reach a helper as two lines"
            );
        }
        set_derp_map_url("");
    }

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

    /// The one that would be quiet if it were got wrong. A stub that answered
    /// `Ok(NodeKeyPair::default())` — or anything with an empty public half —
    /// would put a device into a ceremony offer carrying a node key that
    /// looks like a field somebody filled in, and the runner would then admit
    /// nobody. Tailcat ignores an unrecognized client silently, so the
    /// symptom is a tunnel that times out for no stated reason, which is the
    /// exact failure minting exists to fix.
    #[cfg(not(any(feature = "linked", feature = "helper")))]
    #[test]
    fn minting_without_the_archive_refuses_rather_than_returning_an_empty_key() {
        let out = mint_node_key();
        let Err(TunnelError::NoTailcatLinked) = out else {
            panic!("a build with no archive minted something: {out:?}");
        };
        assert_eq!(TunnelError::NoTailcatLinked.code(), "no_tailcat");
    }

    /// The one a stub could most plausibly get wrong by being HELPFUL.
    ///
    /// Creating a runner's identity is, from the outside, a file write — and a
    /// stub that made an empty `tailcat.key` would let
    /// `farcooler_daemon::allowlist::tunnel_plan` past its `NoIdentity` guard on
    /// a build that can serve nothing at all. That guard is the only admission
    /// check a build with no archive can prove anything about, so satisfying it
    /// with a file nothing could ever read is the exact shape of a check that
    /// cannot fail. The file's ABSENCE is asserted, not just the error.
    #[cfg(not(any(feature = "linked", feature = "helper")))]
    #[test]
    fn a_build_without_the_archive_creates_no_identity_file() {
        let dir = tempfile::tempdir().expect("a scratch directory");
        let path = dir.path().join("tailcat.key");
        let out = ensure_identity(&path);
        let Err(TunnelError::NoTailcatLinked) = out else {
            panic!("a build with no archive created an identity: {out:?}");
        };
        assert!(!path.exists(), "a build with no tunnel wrote a key file anyway");
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
