//! A daemon that is up but must not serve.
//!
//! One reason today: the database was written by a newer Far Cooler than this
//! one (`DomainError::NewerData`), which is what a rollback to an older build
//! leaves behind. The store refuses to open it, so there is no `Service`.
//!
//! Exiting is the obvious answer and the wrong one. The LaunchAgent and the
//! systemd unit both restart a daemon that exits non-zero, so it would restart
//! into the same refusal every few seconds for as long as the older build is
//! installed; exiting zero would hide it instead. And either way the reason
//! goes to stderr, which nobody reads: the CLI starts the daemon with stderr
//! sent nowhere, and a client finding no socket says what it says for a runner
//! with nothing installed.
//!
//! So the daemon stays up, holding its lock and its socket, and answers every
//! session with the reason (`farcooler_transport::refuse`). The CLI, the Mac's
//! runner status and a phone's connect screen all show a runner's own words,
//! so the sentence reaches whoever is looking. Installing a build that can
//! read the database replaces this process the way it replaces any other
//! daemon from different source (`daemon_link::ensure_local`).

use std::path::Path;

use farcooler_core::DomainError;
use farcooler_transport::UnixListenerServer;

/// Whether a failure to open the service is one to stay up and report,
/// rather than exit on.
///
/// Only an error no restart can fix. Anything else may be transient, a disk
/// that was busy or a directory not there yet, and exiting into the
/// supervisor's restart is still the right way to retry it.
pub fn stays_up_for(err: &DomainError) -> bool {
    matches!(err, DomainError::NewerData)
}

/// Hold `socket` and refuse every session on it with `err` until `shutdown`
/// resolves, then remove the socket.
///
/// The caller holds the daemon lock across this, so a second start finds a
/// live socket and exits quietly, the same as beside a working daemon.
pub async fn hold(
    socket: &Path,
    err: DomainError,
    shutdown: impl std::future::Future<Output = ()>,
) -> std::io::Result<()> {
    let server = UnixListenerServer::bind(socket)?;
    tracing::error!(socket = %socket.display(), reason = %err, "not serving; refusing every session with the reason");
    let result = tokio::select! {
        served = server.refuse_every(err) => served,
        _ = shutdown => Ok(()),
    };
    let _ = std::fs::remove_file(socket);
    result
}

/// SIGTERM or Ctrl-C, the two ways a held daemon is told to go.
///
/// The working daemon has a third, `daemon.shutdown`, which a refusing one
/// cannot receive: no session gets far enough to ask. `ensure_local` signals
/// the socket's peer instead, as it does for a daemon too old to know the
/// method.
pub async fn signaled() {
    let Ok(mut term) = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
    else {
        let _ = tokio::signal::ctrl_c().await;
        return;
    };
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        _ = term.recv() => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use farcooler_transport::{Client, ClientError};

    const SENTENCE: &str =
        "This runner's data was written by a newer Far Cooler. Update Far Cooler to use it.";

    /// The daemon's start, up to the decision: a runtime directory whose
    /// database a newer build left behind fails to open with `NewerData`, and
    /// that is the error the daemon stays up for.
    #[tokio::test]
    async fn a_newer_database_stops_the_service_with_the_reason() {
        let dir = tempfile::tempdir().unwrap();
        drop(crate::service::Service::open_in(dir.path().to_path_buf()).await.unwrap());
        let conn = rusqlite::Connection::open(dir.path().join("farcooler.db")).unwrap();
        // One migration on, and one this build may not read past.
        conn.execute_batch(
            "UPDATE meta SET value = CAST(value AS INTEGER) + 1 WHERE key = 'schema_version';
             UPDATE meta SET value = (SELECT value FROM meta WHERE key = 'schema_version')
             WHERE key = 'compatible_down_to';",
        )
        .unwrap();
        drop(conn);

        let err = crate::service::Service::open_in(dir.path().to_path_buf())
            .await
            .err()
            .expect("a newer database must not open");
        assert!(stays_up_for(&err), "{err:?}");
        assert_eq!(err.to_string(), SENTENCE);
        assert!(!stays_up_for(&DomainError::OperationFailed));
    }

    /// And held, it tells whoever connects, then goes when told to and takes
    /// its socket with it.
    #[tokio::test]
    async fn a_held_daemon_says_why_to_every_client_and_leaves_when_told() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("farcooler.sock");
        let stop = std::sync::Arc::new(tokio::sync::Notify::new());
        let held = {
            let socket = socket.clone();
            let stop = stop.clone();
            tokio::spawn(async move { hold(&socket, DomainError::NewerData, stop.notified()).await })
        };

        let mut tries = 0;
        let err = loop {
            match Client::connect(&socket, "test", "0").await {
                Err(ClientError::Connect(_)) if tries < 100 => {
                    tries += 1;
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                }
                other => break other.err().expect("refused"),
            }
        };
        assert!(
            matches!(&err, ClientError::Daemon { message, .. } if message == SENTENCE),
            "{err:?}"
        );

        stop.notify_one();
        held.await.unwrap().unwrap();
        assert!(!socket.exists(), "the socket goes with the daemon");
    }
}
