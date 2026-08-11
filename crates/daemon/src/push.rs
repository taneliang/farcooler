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
    "https://relay.farcooler.com".to_string()
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
        use std::io::Write;

        let path = config_path(runtime_dir);
        // Not `unwrap_or_default()`. That wrote an EMPTY file, returned Ok, and
        // let the CLI print "paired" — after which `load_in` parses nothing,
        // `push status` says "not paired", and the machine is silently mute.
        let text = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;

        // Owner-only from the moment it exists. Setting the mode after writing
        // leaves a window in which a bearer token sits on disk with whatever
        // the umask says — usually world-readable — and the fix costs one
        // `OpenOptions` call.
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        options.open(&path)?.write_all(text.as_bytes())
    }

    pub fn forget_in(runtime_dir: &Path) {
        let _ = std::fs::remove_file(config_path(runtime_dir));
    }
}

/// What the relay is told.
///
/// A title, a line under it, which terminal to open, and the two facts a Live
/// Activity needs. Never a transcript, never a command, never output — the
/// relay is a delivery service, and a payload it cannot read is a payload it
/// cannot leak.
#[derive(Debug, serde::Serialize)]
struct Notification<'a> {
    title: &'a str,
    subtitle: &'a str,
    /// `"blocked"` or `"done"`: whether the phone raises the live card on the
    /// lock screen or dismisses the one already there.
    ///
    /// Told rather than worked out, because the only other place this fact
    /// exists in the payload is inside `title`, which is a human sentence. A
    /// relay that read "needs you" out of one would be a second copy of the
    /// same rule, in another language, breaking the day the copy changed.
    ///
    /// A daemon too old to send this omits the field and the relay falls back
    /// to a plain notification — so an empty or invented status is worse than
    /// none, and this is never either.
    status: &'a str,
    /// What the agent is called, on its own: the resolved agent name — "claude",
    /// "codex", a harness preset — and not the worktree, which does not exist at
    /// the call site. Deliberately the same string already interpolated into
    /// `title`, so the live card and the notification under it cannot disagree
    /// about what they are naming.
    ///
    /// It is already inside `title` as a fragment, and that is exactly the
    /// problem: the live card puts the name and the status in separate places
    /// on the lock screen, and neither can be cut back out of a sentence.
    label: &'a str,
    terminal: &'a str,
    /// What this machine is running, so the devices screen can show which of
    /// someone's machines is behind without them going to each one to look.
    ///
    /// Sent here rather than on a route of its own because a daemon that
    /// notifies is a daemon that is running, which is exactly when the answer
    /// is worth recording — and it costs nothing on a request already being
    /// made.
    version: &'a str,
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
    status: &str,
    label: &str,
    terminal: &str,
) {
    let url = format!("{}/v1/notify", pairing.relay.trim_end_matches('/'));
    let result = client
        .post(&url)
        .bearer_auth(&pairing.token)
        .json(&Notification {
            title,
            subtitle,
            status,
            label,
            terminal,
            version: farcooler_protocol::BUILD,
        })
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
    fn a_saved_pairing_comes_back_and_can_be_forgotten() {
        // The whole contract of `farcooler push pair|status|forget`, which had
        // no test at all: whether a machine is paired is the difference between
        // being told an agent is stuck and finding out in the morning.
        let dir = tempfile::tempdir().expect("tempdir");
        assert!(Pairing::load_in(dir.path()).is_none(), "nothing is paired yet");

        Pairing { relay: "https://mine.example".into(), token: "t".into() }
            .save_in(dir.path())
            .expect("save");
        let back = Pairing::load_in(dir.path()).expect("saved");
        assert_eq!(back.token, "t");
        assert_eq!(back.relay, "https://mine.example");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(config_path(dir.path()))
                .expect("metadata")
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600, "a bearer token must not be readable by anyone else");
        }

        Pairing::forget_in(dir.path());
        assert!(Pairing::load_in(dir.path()).is_none(), "forget means forgotten");
    }

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
