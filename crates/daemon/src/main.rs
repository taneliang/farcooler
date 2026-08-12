//! `farcoolerd` — the host daemon.
//!
//! It opens no network listener. The only entry point is a mode-0600 Unix
//! socket under a user-only directory; reaching a host from elsewhere means
//! going through sshd, which already authenticates both directions. That is the
//! whole of Far Cooler's attack surface, and it is deliberately this small.
//!
//! The daemon owns the SQLite database. Nothing else may open it — two writers
//! would contend for the same file lock and, worse, two processes would each
//! believe they were the authority on durable intent. Runtime truth is a
//! different matter: that lives in tmux, which is safe for anything to read,
//! which is why streaming a terminal does not have to come through here.

use std::sync::Arc;

use farcooler_daemon::{paths, rpc::Rpc, service::Service, watch::Watcher};
use farcooler_protocol::v1::{Request, Response, Scope};
use farcooler_transport::{HandshakeConfig, Handler, UnixListenerServer};

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
    // up, logged, and exited — and `host probe`, which runs
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
        return farcooler_daemon::fanout::serve(pane).await.map_err(|e| {
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
    // machine, in groups sharing a start time to the second, and they are the
    // likeliest reason `capture-pane` on that machine had slowed from
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

    // One watcher for the host, shared by every connection. Deriving activity
    // once and pushing it is the whole point: N clients must not mean N
    // processes reading the same screens.
    let watcher = Watcher::new(service.clone());
    tokio::spawn(watcher.clone().run());

    // Expire pasted images. Once at startup and daily after that, because the
    // machine this runs on is a laptop that is asleep more often than it is
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

    let cfg = HandshakeConfig {
        daemon_version: farcooler_protocol::BUILD.to_string(),
        // A local socket caller holds host_admin.
        //
        // Reaching this socket already requires being the owning user on this
        // machine, who can read the database and the worktrees directly
        // anyway. Granting less here would protect nothing; it would only stop
        // the user's own Mac app from showing them their own paths. Remote
        // clients arrive over SSH and are scoped there, where the distinction
        // is real.
        granted_scope: Scope::HostAdmin,
    };

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

    let result = tokio::select! {
        served = server.serve(cfg, RpcFactory { service, watcher, stop: stop.clone() }) => served.map_err(|e| {
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

async fn serve_stdio_session() -> Result<(), i32> {
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
    // phone, which reaches every host this way, and not on any remote host from
    // the Mac. The fleet listed perfectly and the chat was permanently empty.
    //
    // So when a daemon is listening, this becomes a pipe to it. One daemon per
    // host owns the live state and every entry point reaches that one, which is
    // what the rest of the design already assumes.
    if let Ok(socket) = paths::socket_path() {
        if let Ok(stream) = tokio::net::UnixStream::connect(&socket).await {
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

    let cfg = HandshakeConfig {
        daemon_version: farcooler_protocol::BUILD.to_string(),
        granted_scope: Scope::HostAdmin,
    };

    // `daemon.shutdown` ends this session, which is the whole of what this
    // process is. Nothing else here is shared, so there is no other daemon for
    // it to stop — when one exists, the branch above made this a pipe to it and
    // the request went there instead.
    let stop = Arc::new(tokio::sync::Notify::new());
    let served = farcooler_transport::serve_stdio(
        cfg,
        RpcFactory { service, watcher, stop: stop.clone() },
    );

    tokio::select! {
        result = served => result.map_err(|e| {
            eprintln!("stdio session ended: {e}");
            1
        }),
        _ = stop.notified() => Ok(()),
    }
}

/// One `Rpc` per request, over one shared `Service`.
///
/// The scope belongs to the connection; the service does not, and must not — a
/// second `Service` would mean a second SQLite handle and a second, divergent
/// view of the tmux inventory.
#[derive(Clone)]
struct RpcFactory {
    service: Arc<Service>,
    watcher: Arc<Watcher>,
    /// Shared with whatever is waiting to stop this process. See `daemon.shutdown`.
    stop: Arc<tokio::sync::Notify>,
}

impl Handler for RpcFactory {
    fn handle(&self, req: Request) -> impl std::future::Future<Output = Response> + Send {
        let rpc =
            Rpc::new(self.service.clone(), self.watcher.clone(), Scope::HostAdmin, self.stop.clone());
        async move { rpc.handle(req).await }
    }

    /// Every connection is subscribed to the push stream.
    ///
    /// No opt-in request: a client that connected wants to know when something
    /// changes, and making it ask would just be a round trip before the first
    /// event. Cost is zero on a quiet host, because only changes are sent.
    fn events(&self) -> Option<tokio::sync::broadcast::Receiver<farcooler_protocol::v1::Event>> {
        Some(self.watcher.subscribe())
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
