//! `farcoolerd` — the runner daemon.
//!
//! It opens no network listener. The only entry point is a mode-0600 Unix
//! socket under a user-only directory; reaching a runner from elsewhere means
//! going through sshd, which already authenticates both directions. That is the
//! whole of Far Cooler's attack surface, and it is deliberately this small.
//!
//! The daemon owns the SQLite database. Nothing else may open it — two writers
//! would contend for the same file lock and, worse, two processes would each
//! believe they were the authority on durable intent. Runtime truth is a
//! different matter: that lives in tmux, which is safe for anything to read,
//! which is why streaming a terminal does not have to come through here.

use std::sync::Arc;

// The scope words come from `fence` rather than from a pair of matches here,
// because that module WRITES the `--scope` word this one reads back off the
// command line sshd forced. Two spellings of the same vocabulary would mean an
// enrollment that grants nothing, and it would say so only on the runner.
use farcooler_fence::{scope_from_word, scope_word};
use farcooler_daemon::{
    paths,
    rpc::RpcFactory,
    service::Service,
    sessions::peer_from_preamble,
    watch::Watcher,
};
use farcooler_protocol::v1::Scope;
use farcooler_transport::{HandshakeConfig, Peer, UnixListenerServer};

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "farcooler=info,warn".into()),
        )
        // stderr always: in --stdio mode stdout IS the wire, and one log line
        // on it would corrupt the first frame.
        .with_writer(std::io::stderr)
        .init();

    if let Err(code) = run().await {
        std::process::exit(code);
    }
}

async fn run() -> Result<(), i32> {
    // `farcoolerd --stdio` is the remote entry point.
    //
    // sshd launches it, and it speaks the identical framing over stdin and
    // stdout that the socket speaks. That is the whole of Far Cooler's remote
    // transport: no listener, no port, no second authentication system. SSH has
    // already proved who the caller is, and the caller is the same Unix user
    // who owns the database and could read it directly anyway.
    // `farcoolerd --version` answers instead of starting a daemon.
    //
    // It used to do neither: the flag was unrecognized, so the process started
    // up, logged, and exited — and `runner probe`, which runs
    // `farcoolerd --version` over ssh to find out what is installed, captured
    // that log line as the version. There was no way to ask this binary what it
    // was, which is a strange gap in a system whose components check each
    // other's build stamps.
    if std::env::args().any(|a| a == "--version" || a == "-V") {
        println!("farcoolerd {}", farcooler_protocol::BUILD);
        return Ok(());
    }

    if std::env::args().any(|a| a == "--stdio") {
        return serve_stdio_session().await;
    }

    // `farcoolerd --stream <terminal>` is the data plane, and it is deliberately
    // not the control plane.
    //
    // Screens used to reach a phone by polling a request/response method, which
    // put the latency floor at the poll interval — a second, where the capture
    // itself costs sixteen milliseconds. Everything else was already fast; the
    // waiting was ours.
    //
    // This is a separate ssh channel carrying nothing but the pane's bytes, so
    // the floor becomes the network's round trip and nothing else. It is a
    // second channel rather than frames multiplexed onto the control connection
    // because that connection serializes calls: a stream sharing it would sit
    // behind every fleet refresh, and a fleet refresh behind every byte. ssh
    // already multiplexes channels, so the simplest correct answer is to let it.
    let args: Vec<String> = std::env::args().collect();
    if let Some(index) = args.iter().position(|a| a == "--stream") {
        let Some(terminal) = args.get(index + 1) else {
            eprintln!("--stream needs a terminal id");
            return Err(2);
        };
        return stream_terminal(terminal).await;
    }

    // `farcoolerd --fanout <pane>` is what tmux pipes a pane into, so that every
    // watcher of that pane can have the same bytes.
    //
    // Not a mode a human runs. tmux allows one `pipe-pane` per pane, so before
    // this existed the second watcher of a terminal replaced the first one's
    // pipe and ended its stream without either of them being told. See `fanout`.
    if let Some(index) = args.iter().position(|a| a == "--fanout") {
        let Some(pane) = args.get(index + 1) else {
            eprintln!("--fanout needs a pane id");
            return Err(2);
        };
        // Which install's pane this is. A pane number is unique only within one
        // tmux server, so two daemons on this host both have a `%0`; without
        // this they share one socket and the second silently reads the first
        // one's pane. See `fanout::socket_path`.
        //
        // Told to us rather than worked out here, because this process is
        // spawned by tmux and the daemon that started the pipe is the one that
        // knows. Defaulted rather than required so a pipe command written by an
        // older daemon still serves its pane instead of dying: that pane's
        // watcher is looking for the old name too, and both go away with it.
        let install = args
            .iter()
            .position(|a| a == "--install")
            .and_then(|i| args.get(i + 1))
            .map(String::as_str)
            .unwrap_or("");
        return farcooler_daemon::fanout::serve(install, pane).await.map_err(|e| {
            eprintln!("cannot serve that pane: {e}");
            1
        });
    }

    let socket = paths::socket_path().map_err(|e| {
        eprintln!("cannot determine the socket path: {e}");
        1
    })?;

    // Refuse to start beside a live daemon rather than stealing its socket.
    //
    // `UnixListenerServer::bind` unlinks a stale socket, so a crashed daemon
    // cannot lock the user out forever. That is right for a dead socket and
    // wrong for a live one, so probe first: if something answers, this process
    // is redundant and exits quietly.
    if tokio::net::UnixStream::connect(&socket).await.is_ok() {
        tracing::info!("a daemon is already listening; exiting");
        return Ok(());
    }

    // And then take the lock, because the probe above is a check against a
    // condition this process is about to change, with a great deal of work in
    // between.
    //
    // That gap was wide — `Service::open` opens SQLite and inventories tmux,
    // `backfill_pane_tags` walks every pane, and only then does the bind
    // happen — so daemons started in the same moment all probed, all found
    // nothing, all did the setup, and all bound. The last one unlinked the
    // others' socket and won. The losers did not exit: they went on running
    // forever, holding the database and sampling every pane once a second
    // through a `Watcher` no client could reach.
    //
    // It is not a rare race. Sixty-three of these were found alive on one
    // runner, in groups sharing a start time to the second, and they are the
    // likeliest reason `capture-pane` on that runner had slowed from
    // milliseconds to the better part of a second.
    //
    // An advisory `flock` closes it properly: the kernel hands it to exactly
    // one process, and releases it when that process exits — including when it
    // crashes, so there is no stale lock to clear and no pid file to disbelieve.
    let lock_path = socket.with_extension("lock");
    let _lock = match acquire_daemon_lock(&lock_path) {
        Ok(Some(lock)) => lock,
        Ok(None) => {
            tracing::info!("another daemon holds the lock for this runtime directory; exiting");
            return Ok(());
        }
        Err(e) => {
            eprintln!("cannot lock {}: {e}", lock_path.display());
            return Err(1);
        }
    };

    let service = Arc::new(Service::open().await.map_err(|e| {
        eprintln!("cannot open the service: {e}");
        1
    })?);

    // Repair identity on any pane still carrying it at window level. See
    // `backfill_pane_tags`: those panes read correctly until something moves
    // them, and then stop being identifiable at all.
    service.backfill_pane_tags().await;
    // Shims outlive a daemon restart; without this they dial a socket nobody
    // is listening on and every agent pane goes silent while looking healthy.
    service.resume_agent_listeners();

    // One watcher for the runner, shared by every connection. Deriving activity
    // once and pushing it is the whole point: N clients must not mean N
    // processes reading the same screens.
    let watcher = Watcher::new(service.clone());
    tokio::spawn(watcher.clone().run());

    // Expire pasted images. Once at startup and daily after that, because the
    // host this runs on is a laptop that is asleep more often than it is
    // up — an interval alone would let a directory grow for weeks between two
    // long-running sessions that never reached the next tick.
    let sweeping = service.clone();
    tokio::spawn(async move {
        loop {
            farcooler_daemon::pastes::sweep(sweeping.root_dir()).await;
            tokio::time::sleep(std::time::Duration::from_secs(24 * 60 * 60)).await;
        }
    });

    let server = UnixListenerServer::bind(&socket).map_err(|e| {
        eprintln!("cannot bind {}: {e}", socket.display());
        1
    })?;

    tracing::info!(socket = %socket.display(), "farcoolerd listening");

    // Stop on a signal rather than being killed, so the socket is unlinked and
    // the next start does not have to reason about whether it is stale.
    //
    // `daemon.shutdown` arrives on the third arm. A client that has just found
    // a daemon built from different source than itself asks for exactly this,
    // and it has to be the same orderly stop a signal gets — not a kill — or
    // the replacement would start by inheriting a socket nobody unlinked.
    let stop = std::sync::Arc::new(tokio::sync::Notify::new());
    let shutdown = {
        let stop = stop.clone();
        async move {
            let mut term =
                tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                    .expect("SIGTERM handler");
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {}
                _ = term.recv() => {}
                _ = stop.notified() => {}
            }
        }
    };

    // Who an accepted connection is, decided per connection.
    //
    // Almost every caller here is a local one and gets host_admin. The exception
    // is a `--stdio` process relaying an ssh session into this daemon: sshd gave
    // it a scope and a device id, it says so in one line before the first frame,
    // and both have to survive the relay. The scope, or a read-enrolled device
    // would hold host admin on every runner where a daemon happens to be
    // running — which is every runner in normal use. The device id, or nothing
    // in this daemon can say which connections belong to a device, and
    // `client.revoke` cannot close what it revoked.
    let sessions = {
        let service = service.clone();
        let watcher = watcher.clone();
        let stop = stop.clone();
        move |preamble: Option<farcooler_transport::SessionPreamble>| {
            // A local socket caller holds host_admin.
            //
            // Reaching this socket already requires being the owning user on
            // this host, who can read the database and the worktrees directly
            // anyway. Granting less here would protect nothing; it would only
            // stop the user's own Mac app from showing them their own paths.
            // Remote clients arrive over SSH and are scoped there, where the
            // distinction is real.
            //
            // So an absent preamble is not a lesser session: it is every client
            // installed before the preamble existed, and it means exactly what
            // it has always meant. It also names no device, which is what keeps
            // a revocation from ever closing the Mac app's own connection.
            //
            // Present but unreadable is a different matter, and refuses. See
            // `scope_from_word`.
            //
            // Both rules live in `peer_from_preamble` rather than here, because
            // this is a binary: a test cannot call this closure, and the copy it
            // would have to write instead is a second answer about who a caller
            // is — asserted against, while the daemon uses the first one.
            let peer = peer_from_preamble(preamble.as_ref())?;
            Some((
                HandshakeConfig { daemon_version: farcooler_protocol::BUILD.to_string() },
                RpcFactory::new(service.clone(), watcher.clone(), stop.clone(), peer),
            ))
        }
    };

    let result = tokio::select! {
        served = server.serve(sessions) => served.map_err(|e| {
            tracing::error!(error = %e, "listener stopped");
            1
        }),
        _ = shutdown => {
            tracing::info!("shutting down");
            Ok(())
        }
    };

    let _ = std::fs::remove_file(&socket);
    result
}

/// Serve exactly one session over stdin/stdout, then exit.
///
/// One session per process, because that is what sshd gives us: a connection is
/// a process. It opens its own `Service`, which is the one place a second
/// SQLite handle exists — brief, single-threaded, and gone when the ssh session
/// ends. Long-lived state stays in tmux, which is shared safely by design.
/// Write one terminal's live output to stdout until the pane goes away.
async fn stream_terminal(terminal: &str) -> Result<(), i32> {
    let Ok(id) = terminal.parse::<uuid::Uuid>() else {
        eprintln!("--stream needs a terminal uuid");
        return Err(2);
    };
    let service = Service::open().await.map_err(|e| {
        eprintln!("cannot open the service: {e}");
        1
    })?;
    // The inventory has to be fresh, or the pane this id names is looked up in a
    // snapshot taken before the process started.
    service.inventory.refresh().await;
    service.runtime().stream(id).await.map_err(|e| {
        eprintln!("cannot stream that terminal: {e}");
        1
    })
}

/// Pump bytes between this process's stdio and the running daemon.
///
/// Deliberately dumb: it parses nothing and knows nothing about the protocol,
/// because the two ends already agree about it and anything this understood
/// would be a third opinion to keep in step. Either direction closing ends the
/// session, which is what an ssh client disconnecting looks like from here.
async fn relay_stdio(stream: tokio::net::UnixStream) -> Result<(), i32> {
    use tokio::io::AsyncWriteExt;

    let (mut daemon_read, mut daemon_write) = stream.into_split();
    let mut stdin = tokio::io::stdin();
    let mut stdout = tokio::io::stdout();

    let up = async {
        let _ = tokio::io::copy(&mut stdin, &mut daemon_write).await;
        let _ = daemon_write.shutdown().await;
    };
    let down = async {
        let _ = tokio::io::copy(&mut daemon_read, &mut stdout).await;
        let _ = stdout.flush().await;
    };

    tokio::select! {
        _ = up => {}
        _ = down => {}
    }
    Ok(())
}

/// What the forced command says this session is.
///
/// `authorized_keys` carries `command="... --scope read --client phone-7"`, and
/// sshd runs that instead of whatever the client asked for — so these are the
/// one part of this process's command line the connecting device cannot choose.
/// That is the whole mechanism: identity and scope are asserted by a file on
/// this runner, not by the connection.
struct Session {
    /// `None` when the command line said nothing about scope at all, which is
    /// every key enrolled before this existed.
    scope: Option<Scope>,
    /// Which enrolled device this is, as this runner's file says rather than as
    /// the connection claims.
    ///
    /// Carried the whole way: into the preamble when this session is relayed
    /// into a running daemon, and into `Peer` either way, which is what makes a
    /// live connection attributable to a device at all. `client.revoke` closing
    /// what it revoked is the first thing that depends on it; writer leases,
    /// per-client idempotency and auditing are the rest.
    client: Option<String>,
}

impl Session {
    /// What this session may do.
    ///
    /// Absent means host_admin, which is honest rather than lax. A key with no
    /// forced command lets the device write the entire command line, so it could
    /// pass any scope it liked; defaulting to less would protect nothing and
    /// would break every entry enrolled before this existed.
    fn granted(&self) -> Scope {
        self.scope.unwrap_or(Scope::HostAdmin)
    }

    /// The line to send ahead of a relayed session — and `None` when there is
    /// nothing to say, which is not the same as saying "host_admin".
    ///
    /// Silence is already what an unscoped session means to the daemon at the
    /// other end, so the line would add nothing. It would cost something,
    /// though: a daemon built before preambles reads it as a frame, finds a
    /// sixteen-megabyte length where a length should be, and closes. That is
    /// every relayed session on a runner whose binary has been upgraded but
    /// whose daemon process has not yet restarted — failing IN FRONT OF the
    /// handshake, which is where the version mismatch would otherwise have been
    /// explained and the restart asked for.
    ///
    /// A session that was actually given a scope does send it, and does fail
    /// against such a daemon. That is the right way round: a scope that cannot
    /// be delivered must not be silently traded for host admin.
    fn preamble(&self) -> Option<String> {
        if self.scope.is_none() && self.client.is_none() {
            return None;
        }
        let client = self.client.clone().unwrap_or_else(|| "-".into());
        Some(format!("farcooler-session {} {client}\n", scope_word(self.granted())))
    }
}

/// Read the session out of this process's own arguments.
fn requested_session() -> Result<Session, i32> {
    let args: Vec<String> = std::env::args().collect();
    let after = |flag: &str| {
        args.iter().position(|a| a == flag).map(|i| args.get(i + 1).cloned().unwrap_or_default())
    };

    let scope = match after("--scope") {
        None => None,
        Some(word) => Some(scope_from_word(&word).ok_or_else(|| {
            eprintln!("--scope must be read, control or host_admin, not {word:?}");
            2
        })?),
    };
    Ok(Session { scope, client: after("--client").filter(|c| !c.is_empty()) })
}

async fn serve_stdio_session() -> Result<(), i32> {
    // Before anything else, and before a socket is dialled: a session whose
    // scope is a typo must not be served at either end.
    let session = requested_session()?;
    let granted = session.granted();

    // A daemon already running here owns everything that is not in SQLite.
    //
    // This used to open a second `Service` unconditionally, and for anything
    // read from the database that was fine — workspaces and terminals came back
    // correct, so it looked like it worked. But an agent's TRANSCRIPT lives in
    // this process's memory (`AgentSupervisor`), and the shims that produce it
    // are connected to the sockets the FIRST daemon bound. A second service has
    // no shims, so it answers every `agent_subscribe` with epoch 0 and no
    // events — which a client cannot tell apart from a conversation nobody has
    // started.
    //
    // The effect was that agent chat could never work over ssh: not on the
    // phone, which reaches every runner this way, and not on any remote runner from
    // the Mac. The fleet listed perfectly and the chat was permanently empty.
    //
    // So when a daemon is listening, this becomes a pipe to it. One daemon per
    // runner owns the live state and every entry point reaches that one, which is
    // what the rest of the design already assumes.
    if let Ok(socket) = paths::socket_path() {
        if let Ok(mut stream) = tokio::net::UnixStream::connect(&socket).await {
            // Say what this session is before any frame.
            //
            // The pipe below is deliberately dumb and must stay that way, so the
            // scope cannot ride inside the protocol — this process would have to
            // parse a conversation the two ends already agree about, which is a
            // third opinion to keep in step. One line before the conversation
            // starts is the whole mechanism. See `Session::preamble` for when
            // there is nothing to say.
            //
            // Anything that can reach this socket is already the owning user,
            // who holds host_admin regardless, so a caller that lies here gains
            // nothing it did not already have.
            if let Some(preamble) = session.preamble() {
                use tokio::io::AsyncWriteExt;
                if stream.write_all(preamble.as_bytes()).await.is_err() {
                    eprintln!("cannot reach the daemon on this runner");
                    return Err(1);
                }
            }
            return relay_stdio(stream).await;
        }
    }

    // Nothing may be printed to stdout: it is the wire.
    let service = Arc::new(Service::open().await.map_err(|e| {
        eprintln!("cannot open the service: {e}");
        1
    })?);

    let watcher = Watcher::new(service.clone());
    tokio::spawn(watcher.clone().run());

    let cfg = HandshakeConfig { daemon_version: farcooler_protocol::BUILD.to_string() };

    // `daemon.shutdown` ends this session, which is the whole of what this
    // process is. Nothing else here is shared, so there is no other daemon for
    // it to stop — when one exists, the branch above made this a pipe to it and
    // the request went there instead.
    //
    // The session is registered in this process's own `Service`, which is the
    // honest scope of what a revocation here can close: exactly this session.
    // A device with a session against the daemon on the socket is closed by
    // that daemon, and this process is only ever reached when there is none.
    let stop = Arc::new(tokio::sync::Notify::new());
    let served = farcooler_transport::serve_stdio(
        cfg,
        RpcFactory::new(
            service,
            watcher,
            stop.clone(),
            Peer { client_id: session.client.clone(), scope: granted },
        ),
    );

    tokio::select! {
        result = served => result.map_err(|e| {
            eprintln!("stdio session ended: {e}");
            1
        }),
        _ = stop.notified() => Ok(()),
    }
}

/// The daemon's exclusive claim on one runtime directory.
///
/// The lock lives as long as this value does, which is the whole process — the
/// kernel drops it when the file descriptor closes, so an ordinary exit, a
/// panic and a `kill -9` all release it identically. That is why this is a
/// `flock` and not a pid file: there is no stale state to detect, no pid to
/// check for reuse, and nothing to clean up after a crash.
struct DaemonLock {
    /// Never read, and load-bearing anyway: the lock is the open descriptor.
    ///
    /// `#[allow(dead_code)]` rather than leaving the warning, because "field is
    /// never read" is precisely the advice that would get this deleted — and
    /// deleting it closes the file, releases the lock, and restores the bug
    /// with nothing failing to say so.
    #[allow(dead_code)]
    file: std::fs::File,
}

/// Take the lock, or report that somebody else has it.
///
/// `Ok(None)` is not a failure — it is the answer "another daemon is already
/// the owner here", which is the ordinary outcome whenever two clients race to
/// auto-start one.
fn acquire_daemon_lock(path: &std::path::Path) -> std::io::Result<Option<DaemonLock>> {
    use std::os::unix::io::AsRawFd;

    let file = std::fs::OpenOptions::new().create(true).write(true).truncate(false).open(path)?;

    // SAFETY: `file` owns a valid descriptor for the duration of the call, and
    // `flock` only ever reads it.
    let taken = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0;
    if !taken {
        let error = std::io::Error::last_os_error();
        // `EWOULDBLOCK` is somebody else holding it, which is an answer.
        // Anything else — a read-only directory, a filesystem with no locking —
        // is a real failure and must not be mistaken for a healthy duplicate.
        if error.kind() != std::io::ErrorKind::WouldBlock {
            return Err(error);
        }
        return Ok(None);
    }
    Ok(Some(DaemonLock { file }))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The two directions have to agree, or a relayed session is granted one
    /// thing here and told another at the far end.
    #[test]
    fn every_scope_survives_the_round_trip_through_a_word() {
        for scope in [Scope::Read, Scope::Control, Scope::HostAdmin] {
            assert_eq!(scope_from_word(scope_word(scope)), Some(scope));
        }
    }

    #[test]
    fn a_word_this_daemon_does_not_have_is_refused_rather_than_rounded_up() {
        // The whole point: a typo in someone's authorized_keys must not become
        // privilege escalation on the quietest possible path.
        for word in ["reed", "admin", "HostAdmin", "host-admin", "", "read control"] {
            assert_eq!(scope_from_word(word), None, "{word:?} must not resolve to a scope");
        }
    }

    #[test]
    fn a_session_told_nothing_says_nothing_and_holds_host_admin() {
        let session = Session { scope: None, client: None };
        assert_eq!(session.granted(), Scope::HostAdmin);
        assert_eq!(session.preamble(), None, "silence already means host_admin at the far end");
    }

    #[test]
    fn a_scoped_session_says_so_in_one_line() {
        let session = Session { scope: Some(Scope::Read), client: Some("phone-7".into()) };
        assert_eq!(session.granted(), Scope::Read);
        assert_eq!(session.preamble().as_deref(), Some("farcooler-session read phone-7\n"));
    }

    /// A named device with no scope still speaks: the far end learns which
    /// device this is, and reads the same host_admin it would have defaulted to.
    #[test]
    fn a_named_device_is_relayed_even_at_host_admin() {
        let session = Session { scope: None, client: Some("mac-1".into()) };
        assert_eq!(session.preamble().as_deref(), Some("farcooler-session host_admin mac-1\n"));
    }

    /// A scope with no device is the enrollment shape this all exists for, and
    /// the dash is what tells the far end there is no name rather than a blank
    /// field it should try to parse.
    #[test]
    fn an_unnamed_device_relays_a_dash_rather_than_an_empty_field() {
        let session = Session { scope: Some(Scope::Control), client: None };
        assert_eq!(session.preamble().as_deref(), Some("farcooler-session control -\n"));
    }
}
