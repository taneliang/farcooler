//! A test daemon's gh reads no account and writes no state (ov-552).
//!
//! The daemon runs `gh` for PR and CI reads. A daemon a test started, with its
//! `HOME` a `TempDir`, ran the real gh anyway: gh wrote its device id to
//! `$HOME/.local/state/gh` after the `TempDir` was gone and made the directory
//! again, and a test run left `.tmpXXXX` behind in `TMPDIR`. It also used
//! whichever GitHub account the person running the tests is signed into.
//!
//! Under `cfg(test)` and under `FARCOOLER_TEST_STUB_AGENTS`, gh gets a config
//! and a state directory that cannot exist, so it reads as signed out (the same
//! answer a stub gives, which the daemon already treats as "unknown") and has
//! nowhere to write. A stand-in `gh` on `PATH` ignores both.

use std::process::Command;

/// Where gh's configuration and state would be: under a file, so neither can be
/// read or made.
const NOWHERE: &str = "/dev/null/farcooler-test-gh";

/// Whether this process is a test's, and its gh must be inert.
pub fn applies() -> bool {
    cfg!(test) || crate::service::test_stub_agents()
}

/// Point `gh` at nothing, and take away any token the environment carries.
pub fn apply(gh: &mut Command) {
    gh.env("GH_CONFIG_DIR", NOWHERE)
        .env("XDG_STATE_HOME", NOWHERE)
        .env("XDG_CONFIG_HOME", NOWHERE)
        .env("GH_NO_UPDATE_NOTIFIER", "1")
        .env_remove("GH_TOKEN")
        .env_remove("GITHUB_TOKEN")
        .env_remove("GH_ENTERPRISE_TOKEN")
        .env_remove("GITHUB_ENTERPRISE_TOKEN");
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsStr;

    fn env_of(cmd: &Command, key: &str) -> Option<Option<String>> {
        cmd.get_envs().find(|(k, _)| *k == OsStr::new(key)).map(|(_, v)| v.map(|v| v.to_string_lossy().into_owned()))
    }

    #[test]
    fn gh_is_pointed_at_nothing_and_loses_its_token() {
        let mut cmd = Command::new("gh");
        apply(&mut cmd);
        assert_eq!(env_of(&cmd, "GH_CONFIG_DIR"), Some(Some(NOWHERE.into())));
        assert_eq!(env_of(&cmd, "XDG_STATE_HOME"), Some(Some(NOWHERE.into())));
        assert_eq!(env_of(&cmd, "GH_TOKEN"), Some(None), "removed, not left to inherit");
    }

    #[tokio::test]
    async fn the_daemons_gh_command_is_inert_under_test() {
        // The call site: `stack::gh` applies it to every gh the daemon starts.
        // Skipped where there is no gh or git to find.
        let dir = farcooler_core::scratch::dir("gh-inert");
        if !Command::new("git").arg("init").arg("-q").arg(&dir).status().is_ok_and(|s| s.success()) {
            return;
        }
        let Ok(cmd) = crate::stack::gh(&dir).await else { return };
        let cmd = cmd.as_std();
        assert_eq!(env_of(cmd, "GH_CONFIG_DIR"), Some(Some(NOWHERE.into())));
        assert_eq!(env_of(cmd, "XDG_STATE_HOME"), Some(Some(NOWHERE.into())));
    }
}
