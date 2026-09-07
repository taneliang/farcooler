//! Which DERP rendezvous this runner's tunnel uses.
//!
//! One decision, and it is here rather than in `main.rs` for the reason
//! `allowlist::tunnel_plan` and `service::respawn_command` are where they are:
//! a line inside `#[tokio::main] async fn main()` is reachable by no test in
//! this workspace, so deleting it leaves every test green. `configured_derp_map`
//! used to be tested — the test read the variable and then called
//! `farcooler_tailcat::set_derp_map_url` ITSELF, which proved that `std::env`
//! and the tunnel crate both work and said nothing about whether the daemon
//! ever connects them. The two lines that did connect them were in `main`, and
//! deleting them broke nothing.
//!
//! So the whole decision — read the variable, say so in the log, tell the
//! tunnel — is `apply_configured_derp_map`, and `main` is one call. Both halves
//! of that are guarded: `a_runner_installed_with_a_derp_map_uses_it` below goes
//! red if the `set_derp_map_url` call is deleted, and
//! `crates/daemon/tests/a_configured_derp_map_reaches_the_tunnel.rs` goes red if
//! the call in `main` is deleted, because it spawns a real `farcoolerd` and asks
//! the tunnel helper what it was told.

/// The environment variable a runner's DERP map is set through.
///
/// tailcat's default map is documented as best-effort and revocable at any
/// time, and DERP is the rendezvous for every tunneled connection rather than
/// a fallback for the ones that could not go direct. So the day it is revoked,
/// a fleet with no way to be pointed elsewhere needs three app releases and a
/// visit to every runner. This is one string, and it is here so that day costs
/// a setting instead.
const DERP_MAP_ENV: &str = "FARCOOLER_DERP_MAP";

/// The DERP map this runner was INSTALLED with, if any.
///
/// **An environment variable and not a protocol field, deliberately.** This is
/// deployment configuration — it is set by whoever installed this runner — and
/// a client that could tell a runner which DERP map to use could tell it a map
/// the client controls, which is a rendezvous the client controls. Nothing
/// about this belongs on the wire. `FARCOOLER_HOME` sets the precedent for
/// reading a deployment fact here.
///
/// Unset and empty are the same answer, and that answer is `None` rather than
/// the empty string: empty means the LIBRARY's default, and a runner whose
/// installer wrote `FARCOOLER_DERP_MAP=` into a unit file must land on exactly
/// the same rendezvous as one that never mentioned it.
fn configured_derp_map() -> Option<String> {
    std::env::var(DERP_MAP_ENV).ok().filter(|url| !url.is_empty())
}

/// Point this process's tunnel at the rendezvous this runner was installed
/// with, and report what was applied.
///
/// Must run before anything serves. The URL is read when the tunnel's server is
/// built — `helper::spawn` sends `derpmap` to a helper it has just started and
/// before the `serve` that follows it — so a runner that came up on the wrong
/// rendezvous stays there until it is restarted.
///
/// The return value is not decoration either. `Option<String>` is what makes
/// "this runner was configured, and with what" observable to a caller, so the
/// unit tests below can assert on the decision as well as on its effect; `main`
/// discards it, which is what a thin wiring line looks like.
pub fn apply_configured_derp_map() -> Option<String> {
    let url = configured_derp_map()?;
    tracing::info!(%url, "using a configured DERP map");
    farcooler_tailcat::set_derp_map_url(&url);
    Some(url)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Serializes the tests that move `FARCOOLER_DERP_MAP`. `set_var` is only
    /// sound while no other thread reads the environment, and `cargo test`
    /// runs this crate's tests on many threads in one process.
    ///
    /// It also serializes `farcooler_tailcat`'s process-wide record of the
    /// URL, which these tests both write and read back. Nothing else in this
    /// library touches either.
    static DERP_MAP_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// A runner installed with a DERP map uses it, and the tunnel library is
    /// actually told — read back through the library rather than asserted
    /// against the string this test just set, which would prove only that
    /// `std::env` works.
    ///
    /// **The library is reset to its default first.** Without that line the
    /// assertion could be satisfied by a URL some earlier test left behind,
    /// and deleting the `set_derp_map_url` call in `apply_configured_derp_map`
    /// would not necessarily be caught — which is the exact defect this
    /// function was extracted to make catchable.
    ///
    /// **Spelled out rather than written as `DERP_MAP_ENV`.** The name is the
    /// interface: it is what somebody's unit file, launchd plist or
    /// `runner install` invocation already says, so a rename here is a
    /// breaking change for every runner in the field and not a tidy-up. A test
    /// that referred to the constant would go on passing through exactly that
    /// rename, which is the shape of a check that cannot fail.
    ///
    /// SAFETY for the `set_var`s: every test that touches this variable holds
    /// `DERP_MAP_ENV_LOCK`, and nothing else in this library reads the
    /// environment while they run.
    #[test]
    fn a_runner_installed_with_a_derp_map_uses_it() {
        let _serial = DERP_MAP_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        farcooler_tailcat::set_derp_map_url("");
        unsafe { std::env::set_var("FARCOOLER_DERP_MAP", "https://derp.example/derpmap.json") };

        let applied = apply_configured_derp_map();

        assert_eq!(applied.as_deref(), Some("https://derp.example/derpmap.json"));
        assert_eq!(
            farcooler_tailcat::derp_map_url(),
            "https://derp.example/derpmap.json",
            "the runner's DERP map never reached the tunnel"
        );
        farcooler_tailcat::set_derp_map_url("");
        unsafe { std::env::remove_var("FARCOOLER_DERP_MAP") };
    }

    /// A runner nobody configured is left on the library's own default, and
    /// so is one whose installer wrote the variable with nothing after the
    /// `=`. The second half is the one that would rot quietly: a unit file
    /// with an empty value is a normal thing to write, and a runner that took
    /// it as a URL would be a runner nobody could reach.
    ///
    /// Both halves assert on the LIBRARY as well as on the answer, so a
    /// version of this function that applied some default of its own would go
    /// red here rather than pass for returning `None`.
    #[test]
    fn an_unconfigured_runner_is_left_on_the_library_default() {
        let _serial = DERP_MAP_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        farcooler_tailcat::set_derp_map_url("");

        unsafe { std::env::remove_var("FARCOOLER_DERP_MAP") };
        assert_eq!(apply_configured_derp_map(), None, "an unset variable named a DERP map");
        assert_eq!(
            farcooler_tailcat::derp_map_url(),
            "",
            "an unconfigured runner was moved off the library's default"
        );

        unsafe { std::env::set_var("FARCOOLER_DERP_MAP", "") };
        assert_eq!(apply_configured_derp_map(), None, "an empty variable became a URL");
        assert_eq!(
            farcooler_tailcat::derp_map_url(),
            "",
            "an empty setting was applied as if it were a URL"
        );

        unsafe { std::env::remove_var("FARCOOLER_DERP_MAP") };
    }
}
