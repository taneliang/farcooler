//! Whether claude is between turns, as its own session registry says
//! (ov-367, ov-392). The screen can't always tell: after a long paste is
//! queued, claude 2.1.290's footer keeps `paste again to expand` where `esc
//! to interrupt` was. The registry, `<config>/sessions/<pid>.json`, says
//! `"status":"idle"` only between turns, and is read through
//! `claude_registry`, so a file a crashed claude left for a reused pid
//! (`procStart` not this process's start) counts for nothing.

use std::path::{Path, PathBuf};

use farcooler_core::session_log::projector::Activity;

use super::mid_turn;
use crate::claude_registry::{self, Kernel};

/// What claude's registry says of the live process `pid`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Said {
    /// `"status":"idle"`, and only that: between turns.
    Idle,
    /// Live, and anything else: `busy`, `shell` (a `!` command running), or
    /// no status at all. Never typed into as if between turns.
    NotIdle,
    /// No live entry: no file, one that doesn't parse, or a stale one.
    Nothing,
}

/// `pid`'s entry in the registry under `config`.
pub(crate) fn said(config: &Path, pid: i32) -> Said {
    let Ok(bytes) = std::fs::read(config.join("sessions").join(format!("{pid}.json"))) else { return Said::Nothing };
    match claude_registry::parse(&bytes) {
        Some(entry) if entry.pid == pid && claude_registry::is_live(&entry, &Kernel) => match entry.status {
            Some(Activity::Idle) => Said::Idle,
            _ => Said::NotIdle,
        },
        _ => Said::Nothing,
    }
}

/// What claude's registry says the live process `pid` is doing, for a key
/// that stops or steers a turn (`interrupt`, ov-368).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Status {
    /// `"status":"busy"`, and only that: a turn running, no dialog up.
    Busy,
    /// `idle`, or `shell` (a `!` command or a background shell running, the
    /// turn over): nothing to stop.
    Idle,
    /// Live, and anything else. claude 2.1.290 writes `"status":"waiting"`
    /// while a permission dialog is up (ov-368); no status at all is an
    /// older claude's, which can't say there's none.
    Waiting,
    /// No live entry.
    Nothing,
}

/// `pid`'s status in the registry under `config`, read as `said` reads it.
pub(crate) fn status(config: &Path, pid: i32) -> Status {
    let Ok(bytes) = std::fs::read(config.join("sessions").join(format!("{pid}.json"))) else { return Status::Nothing };
    match claude_registry::parse(&bytes) {
        Some(entry) if entry.pid == pid && claude_registry::is_live(&entry, &Kernel) => match entry.status {
            Some(Activity::Busy) => Status::Busy,
            Some(Activity::Idle | Activity::Shell) => Status::Idle,
            None => Status::Waiting,
        },
        _ => Status::Nothing,
    }
}

/// claude's config directory, from its process's environment (`mid_turn`).
pub(crate) async fn config_of(pid: i32) -> Option<PathBuf> {
    mid_turn::config_dir(&mid_turn::process_env(pid).await?)
}

/// `said`, for the process `pid`, its config found from its environment.
pub(crate) async fn said_of(pid: i32) -> Said {
    match config_of(pid).await {
        Some(config) => said(&config, pid),
        None => Said::Nothing,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Only an explicit `idle` from the live process is idle; a stale file,
    /// another pid's, or none says nothing.
    #[test]
    fn only_an_explicit_idle_is_idle() {
        let config = tempfile::tempdir().unwrap();
        let pid = std::process::id() as i32;
        let started = started_utc(pid);
        let write = |body: String| {
            std::fs::create_dir_all(config.path().join("sessions")).unwrap();
            std::fs::write(config.path().join("sessions").join(format!("{pid}.json")), body).unwrap();
        };
        assert_eq!(said(config.path(), pid), Said::Nothing, "no file");
        let entry = |status: &str| {
            format!(r#"{{"pid":{pid},"sessionId":"s","procStart":"{started}"{status}}}"#)
        };
        write(entry(r#","status":"idle""#));
        assert_eq!(said(config.path(), pid), Said::Idle);
        for status in [r#","status":"busy""#, r#","status":"shell""#, r#","status":"waiting""#, ""] {
            write(entry(status));
            assert_eq!(said(config.path(), pid), Said::NotIdle, "{status:?}");
        }
        write(entry(r#","status":"idle""#).replace(&started, "Sun Oct  4 18:06:13 2020"));
        assert_eq!(said(config.path(), pid), Said::Nothing, "a stale file");
    }

    /// Only an explicit `busy` from the live process is busy (ov-368): the
    /// `waiting` claude writes under a dialog, and no status, are neither
    /// busy nor idle.
    #[test]
    fn only_an_explicit_busy_is_busy() {
        let config = tempfile::tempdir().unwrap();
        let pid = std::process::id() as i32;
        let started = started_utc(pid);
        std::fs::create_dir_all(config.path().join("sessions")).unwrap();
        let file = config.path().join("sessions").join(format!("{pid}.json"));
        assert_eq!(status(config.path(), pid), Status::Nothing, "no file");
        for (field, want) in [
            (r#","status":"busy""#, Status::Busy),
            (r#","status":"idle""#, Status::Idle),
            (r#","status":"shell""#, Status::Idle),
            (r#","status":"waiting""#, Status::Waiting),
            ("", Status::Waiting),
        ] {
            std::fs::write(&file, format!(r#"{{"pid":{pid},"sessionId":"s","procStart":"{started}"{field}}}"#)).unwrap();
            assert_eq!(status(config.path(), pid), want, "{field:?}");
        }
        let stale = format!(r#"{{"pid":{pid},"sessionId":"s","procStart":"Sun Oct  4 18:06:13 2020","status":"busy"}}"#);
        std::fs::write(&file, stale).unwrap();
        assert_eq!(status(config.path(), pid), Status::Nothing, "a stale file");
    }

    /// This process's start, as claude writes `procStart`: UTC.
    fn started_utc(pid: i32) -> String {
        use crate::claude_registry::Processes;
        let secs = Kernel.started(pid).expect("this process started");
        let out = std::process::Command::new("date").args(["-u", "-r", &secs.to_string(), "+%a %b %e %H:%M:%S %Y"]).output();
        let out = match out {
            Ok(o) if o.status.success() => o,
            _ => std::process::Command::new("date").args(["-u", "-d", &format!("@{secs}"), "+%a %b %e %H:%M:%S %Y"]).output().unwrap(),
        };
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }
}
