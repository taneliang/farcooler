//! Telling the relay that an agent needs its owner.
//!
//! The daemon holds NO user credential and does no sign-in. It has a bearer
//! token the relay issued when a signed-in phone paired this runner, and that
//! token names nothing but "this account". So there is no WorkOS here, no
//! browser to open on a headless Linux box, and no user identity sitting on a
//! server the user does not own.
//!
//! What that buys, concretely: a stolen daemon token can notify the phone of
//! the person it was stolen from and can do nothing else — it cannot enumerate
//! devices, cannot name a destination, and is revocable from the app without
//! touching this runner.

use std::path::{Path, PathBuf};

/// Where a paired runner keeps its token.
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

/// The relay this channel's clients talk to.
///
/// One relay per channel, the same partition the runtime directory and the
/// binary name follow. They are separate deployments with separate databases
/// and separate WorkOS environments, so a beta daemon cannot notify a release
/// app — which is correct, because it could not reach that app's machine
/// either.
///
/// Release's URL is unchanged and must stay that way: it is compiled into
/// binaries in the App Store, which cannot be told a new one for days.
pub fn default_relay() -> String {
    use farcooler_protocol::Channel;
    match farcooler_protocol::CHANNEL {
        Channel::Stable => "https://relay.farcooler.com",
        Channel::Preview => "https://relay-preview.farcooler.com",
        Channel::Canary => "https://relay-canary.farcooler.com",
        Channel::Local => "https://relay-local.farcooler.com",
    }
    .to_string()
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
        // `push status` says "not paired", and the runner is silently mute.
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
/// Activity needs.
///
/// **One line at a time, and never the transcript itself.** `subtitle` is one
/// composed line — the agent's question while it is blocked, the composed
/// signal rung while it works — and it is the same string the sidebar is
/// drawing: derived from the transcript and the tool stream, redacted at
/// `farcooler_core::feed`'s single choke point, and cut to `feed::WIDTH`.
/// Never the conversation behind it, never a command line, never raw output.
///
/// The REPETITION is the part worth stating outright, because it is new and it
/// is not what "a notification" sounds like. A `working` notice moves the live
/// card for the whole length of a run — one line at most every
/// `watch::CARD_REFRESH_MS`, ten seconds, for as long as the agent works —
/// where `blocked` and `done` cross perhaps twice between them. So the relay
/// sees a slow drip of an agent's headline rather than two lines and silence.
///
/// A wider exposure than it was, and still not a leak, because nothing is
/// kept: `/v1/notify` persists a `version` and nothing else off this body,
/// `live_activities` holds delivery metadata, and the worker refuses to log a
/// body at all. The rule has not been relaxed — the relay is a delivery
/// service, and a payload it does not keep is a payload it cannot leak — but
/// it has to be stated about what actually crosses.
#[derive(Debug, serde::Serialize)]
struct Notification<'a> {
    title: &'a str,
    subtitle: &'a str,
    /// `"working"`, `"blocked"` or `"done"` — the same three `Notice` in
    /// `watch.rs` states, and nothing else.
    ///
    /// What the phone does with each is not one axis but two. `blocked` and
    /// `done` raise or dismiss the live card on the lock screen and alert;
    /// `working` only ever moves a card — starting one silently when there is
    /// none, updating it in place when there is — so an agent can be reported
    /// for a whole run without buzzing anybody.
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
    /// Whether the turn this `"done"` is about ENDED BADLY.
    ///
    /// A second field rather than a fourth status, because it answers a
    /// different question. `status` tells the relay what to do with the lock
    /// screen card, and a failed turn is over exactly as a finished one is —
    /// see `watch::exit_notice`. This tells the phone which MARK to draw, which
    /// nothing downstream can work out for itself: the only other place the
    /// fact exists in this payload is inside `title`, as the word "failed" in a
    /// human sentence, and reading a verb back out of one is the second copy of
    /// a rule that `status` already exists to avoid.
    ///
    /// It is here because of what the notification service extension draws.
    /// That extension has a status word and nothing else, `feed::glyph`'s `✗`
    /// is unreachable from `"done"` alone, and `accessoryCircular` draws ONLY
    /// the glyph — so without this an agent whose turn died wore `✓` on a lock
    /// screen widget until the app next polled.
    ///
    /// Always sent, including as `false`. A relay or a phone too old to know
    /// the field ignores it and behaves exactly as it did; skipping it when
    /// false would save nothing and make "absent" and "false" two spellings a
    /// reader has to tell apart.
    failed: bool,
    terminal: &'a str,
    /// What this runner is running, so the devices screen can show which of
    /// someone's runners is behind without them going to each one to look.
    ///
    /// Sent here rather than on a route of its own because a daemon that
    /// notifies is a daemon that is running, which is exactly when the answer
    /// is worth recording — and it costs nothing on a request already being
    /// made.
    version: &'a str,
    /// When the turn began, in Unix MILLISECONDS, so the live card can run its
    /// own clock. `None` between turns.
    ///
    /// A timestamp and nothing else. It says WHEN a turn started, never what it
    /// is about, so it does not widen what a relay could leak — and it buys the
    /// one thing a card cannot compute for itself: how long this has been going
    /// on. The phone renders it as a native timer, which needs no push per tick
    /// and keeps counting with the device off the network entirely.
    ///
    /// Renamed on the wire because the other end is not Rust. The relay reads
    /// `body.startedAt` and copies it into the activity's attributes; a
    /// `started_at` key arrives there as `undefined`, nothing errors, and the
    /// card simply comes up with no timer — the exact silent nothing this field
    /// exists to fix.
    ///
    /// A NUMBER, which serde gives for free here and which is the whole
    /// contract: the phone's decoder in `AgentActivityAttributes` tells seconds
    /// from milliseconds apart by magnitude and reads the field with `try?`, so
    /// a value sent as a string costs the timer without costing the card. That
    /// is a failure nobody would report.
    ///
    /// Skipped when absent rather than sent as null or zero. Zero is a perfectly
    /// decodable instant, and a card counting up from January 1970 is worse than
    /// a card with no clock on it at all.
    #[serde(rename = "startedAt", skip_serializing_if = "Option::is_none")]
    started_at: Option<i64>,
}

/// What one call to `notify` is about.
///
/// A struct rather than seven positional arguments, and for the same reason
/// `watch::Notice` is one: `title`, `subtitle`, `status`, `label` and
/// `terminal` are all `&str`, so any two of them transposed still compiles and
/// shows up only as a wrong lock screen. Named fields make that a build error
/// instead of a bug report.
///
/// Borrowed throughout. Every field is already owned by the caller's stack for
/// the length of the await, and a payload that allocated five strings per
/// notification would be paying for a copy nothing keeps.
pub struct Outgoing<'a> {
    pub title: &'a str,
    pub subtitle: &'a str,
    pub status: &'a str,
    pub failed: bool,
    pub label: &'a str,
    pub terminal: &'a str,
    pub started_at: Option<i64>,
}

/// Send one, or quietly do nothing if this runner was never paired.
///
/// Failure is logged and swallowed on purpose. A push that does not arrive is a
/// missed notification; a push that takes the watcher down with it is every
/// future notification missed as well, plus the fleet.
pub async fn notify(client: &reqwest::Client, pairing: &Pairing, notice: Outgoing<'_>) {
    let Outgoing { title, subtitle, status, failed, label, terminal, started_at } = notice;
    let url = format!("{}/v1/notify", pairing.relay.trim_end_matches('/'));
    let result = client
        .post(&url)
        .bearer_auth(&pairing.token)
        .json(&Notification {
            title,
            subtitle,
            status,
            failed,
            label,
            terminal,
            version: farcooler_protocol::BUILD,
            started_at,
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
        // no test at all: whether a runner is paired is the difference between
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
        assert_eq!(parsed.relay, default_relay(), "an absent relay is this channel's own");
    }

    /// Build the body `notify` sends, without sending it.
    ///
    /// The request itself is a network call no test can reach through, and the
    /// part worth guarding is not the sending — it is the JSON, which three
    /// separate programs have to agree about.
    fn body(started_at: Option<i64>) -> serde_json::Value {
        serde_json::to_value(Notification {
            title: "claude",
            subtitle: "3/7 · Designing test matrix",
            status: "working",
            failed: false,
            label: "claude",
            terminal: "term-1",
            version: "test",
            started_at,
        })
        .expect("serialize")
    }

    #[test]
    fn a_turn_clock_goes_out_as_a_number_or_not_at_all() {
        // The seam this field exists to close, and the one place it can be
        // checked. The phone renders a native timer from `startedAt` and its
        // decoder takes a NUMBER — a string reads back as nil there, which
        // costs the timer and reports nothing at all. Nothing else in this
        // repository would notice.
        let sent = body(Some(1_755_000_000_000));
        assert_eq!(
            sent["startedAt"],
            serde_json::json!(1_755_000_000_000_i64),
            "the relay reads `startedAt`, not `started_at`"
        );
        assert!(sent["startedAt"].is_number(), "a string decodes to nil on the phone: {sent}");

        // Between turns there is no clock, and no key. A `null` would be
        // harmless and a `0` would not: it is a decodable instant, and the card
        // would come up counting the fifty-odd years since January 1970.
        let quiet = body(None);
        assert!(
            quiet.get("startedAt").is_none(),
            "no turn is running, so the card must be given no clock: {quiet}"
        );
    }

    #[test]
    fn each_channel_talks_to_its_own_relay() {
        // One deployment per channel: its own database and its own WorkOS
        // environment. Two channels sharing a relay would mean a preview
        // pairing could notify a stable app — an app that cannot reach the
        // runner that sent it, because they are different binaries at
        // different paths.
        use farcooler_protocol::Channel;
        let urls: Vec<_> = [Channel::Local, Channel::Canary, Channel::Preview, Channel::Stable]
            .iter()
            .map(|c| match c {
                Channel::Stable => "https://relay.farcooler.com",
                Channel::Preview => "https://relay-preview.farcooler.com",
                Channel::Canary => "https://relay-canary.farcooler.com",
                Channel::Local => "https://relay-local.farcooler.com",
            })
            .collect();
        let unique: std::collections::BTreeSet<_> = urls.iter().collect();
        assert_eq!(unique.len(), 4, "two channels cannot share a relay: {urls:?}");

        // Stable's is compiled into App Store binaries that cannot be told a
        // new one for days. It does not move.
        assert_eq!(urls[3], "https://relay.farcooler.com");

        let explicit: Pairing =
            serde_json::from_str(r#"{"token":"abc","relay":"https://mine.example"}"#)
                .expect("parse");
        assert_eq!(explicit.relay, "https://mine.example");
    }
}
