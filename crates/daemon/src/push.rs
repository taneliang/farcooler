//! Telling the relay that an agent needs its owner.
//!
//! The daemon holds NO user credential and does no sign-in. It has a bearer
//! token the relay issued when a signed-in phone paired this machine, and that
//! token names nothing but "this account". So there is no WorkOS here, no
//! browser to open on a headless Linux box, and no user identity sitting on a
//! server the user does not own.
//!
//! What that buys, concretely: a stolen daemon token can notify the phone of
//! the person it was stolen from and can do nothing else — it cannot enumerate
//! devices, cannot name a destination, and is revocable from the app without
//! touching this machine.

use std::path::{Path, PathBuf};

/// Where a paired machine keeps its token.
///
/// Beside the database rather than in it: a token is a credential, not durable
/// intent, and keeping it out of the schema means a database copied for support
/// or debugging does not carry the ability to notify its owner.
pub fn config_path(runtime_dir: &Path) -> PathBuf {
    runtime_dir.join("push.json")
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Pairing {
    /// Overridable so a self-hosted relay is a setting rather than a fork.
    #[serde(default = "default_relay")]
    pub relay: String,
    pub token: String,
}

pub fn default_relay() -> String {
    "https://relay.overnight.sh".to_string()
}

impl Pairing {
    /// Load from wherever this daemon keeps its runtime state.
    pub fn load() -> Option<Self> {
        Self::load_in(&crate::paths::runtime_dir().ok()?)
    }

    pub fn load_in(runtime_dir: &Path) -> Option<Self> {
        let text = std::fs::read_to_string(config_path(runtime_dir)).ok()?;
        serde_json::from_str(&text).ok()
    }

    pub fn save_in(&self, runtime_dir: &Path) -> std::io::Result<()> {
        let path = config_path(runtime_dir);
        let text = serde_json::to_string_pretty(self).unwrap_or_default();
        std::fs::write(&path, text)?;
        // Owner-only: it is a bearer token, and a runtime directory is not
        // always as private as the machine's owner assumes.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
        }
        Ok(())
    }

    pub fn forget_in(runtime_dir: &Path) {
        let _ = std::fs::remove_file(config_path(runtime_dir));
    }
}

/// What the relay is told.
///
/// A title, a line under it, and which terminal to open. Never a transcript,
/// never a command, never output — the relay is a delivery service, and a
/// payload it cannot read is a payload it cannot leak.
#[derive(Debug, serde::Serialize)]
struct Notification<'a> {
    title: &'a str,
    subtitle: &'a str,
    terminal: &'a str,
}

/// Send one, or quietly do nothing if this machine was never paired.
///
/// Failure is logged and swallowed on purpose. A push that does not arrive is a
/// missed notification; a push that takes the watcher down with it is every
/// future notification missed as well, plus the fleet.
pub async fn notify(
    client: &reqwest::Client,
    pairing: &Pairing,
    title: &str,
    subtitle: &str,
    terminal: &str,
) {
    let url = format!("{}/v1/notify", pairing.relay.trim_end_matches('/'));
    let result = client
        .post(&url)
        .bearer_auth(&pairing.token)
        .json(&Notification { title, subtitle, terminal })
        .send()
        .await;

    match result {
        Ok(response) if response.status().is_success() => {}
        Ok(response) => {
            tracing::warn!(status = %response.status(), "relay refused a notification")
        }
        Err(e) => tracing::warn!(error = %e, "could not reach the relay"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pairing_round_trips_and_defaults_its_relay() {
        // The relay is optional in the file so an existing pairing keeps
        // working when the default moves, and a self-hoster can set it without
        // the daemon needing a different shape of config.
        let parsed: Pairing = serde_json::from_str(r#"{"token":"abc"}"#).expect("parse");
        assert_eq!(parsed.token, "abc");
        assert!(parsed.relay.starts_with("https://"));

        let explicit: Pairing =
            serde_json::from_str(r#"{"token":"abc","relay":"https://mine.example"}"#)
                .expect("parse");
        assert_eq!(explicit.relay, "https://mine.example");
    }
}
