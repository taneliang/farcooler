//! `overnightd` — the host daemon.
//!
//! It opens no network listener. The only entry point is a mode-0600 Unix
//! socket under a user-only directory; reaching a host from elsewhere means
//! going through sshd, which already authenticates both directions. That is the
//! whole of Overnight's attack surface, and it is deliberately this small.
//!
//! The daemon owns the SQLite database. Nothing else may open it — two writers
//! would contend for the same file lock and, worse, two processes would each
//! believe they were the authority on durable intent. Runtime truth is a
//! different matter: that lives in tmux, which is safe for anything to read,
//! which is why streaming a terminal does not have to come through here.

use std::sync::Arc;

use overnight_daemon::{paths, rpc::Rpc, service::Service, watch::Watcher};
use overnight_protocol::v1::{Request, Response, Scope};
use overnight_transport::{HandshakeConfig, Handler, UnixListenerServer};

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "overnight=info,warn".into()),
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
    // `overnightd --stdio` is the remote entry point.
    //
    // sshd launches it, and it speaks the identical framing over stdin and
    // stdout that the socket speaks. That is the whole of Overnight's remote
    // transport: no listener, no port, no second authentication system. SSH has
    // already proved who the caller is, and the caller is the same Unix user
    // who owns the database and could read it directly anyway.
    if std::env::args().any(|a| a == "--stdio") {
        return serve_stdio_session().await;
    }

    // `overnightd --stream <terminal>` is the data plane, and it is deliberately
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
    // because that connection serialises calls: a stream sharing it would sit
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

    // `overnightd --fanout <pane>` is what tmux pipes a pane into, so that every
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
        return overnight_daemon::fanout::serve(pane).await.map_err(|e| {
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
    // is redundant and exits quietly. That is what makes two clients racing to
    // auto-start the daemon harmless.
    if tokio::net::UnixStream::connect(&socket).await.is_ok() {
        tracing::info!("a daemon is already listening; exiting");
        return Ok(());
    }

    let service = Arc::new(Service::open().await.map_err(|e| {
        eprintln!("cannot open the service: {e}");
        1
    })?);

    // Repair identity on any pane still carrying it at window level. See
    // `backfill_pane_tags`: those panes read correctly until something moves
    // them, and then stop being identifiable at all.
    service.backfill_pane_tags().await;

    // One watcher for the host, shared by every connection. Deriving activity
    // once and pushing it is the whole point: N clients must not mean N
    // processes reading the same screens.
    let watcher = Watcher::new(service.clone());
    tokio::spawn(watcher.clone().run());

    let server = UnixListenerServer::bind(&socket).map_err(|e| {
        eprintln!("cannot bind {}: {e}", socket.display());
        1
    })?;

    tracing::info!(socket = %socket.display(), "overnightd listening");

    let cfg = HandshakeConfig {
        daemon_version: env!("CARGO_PKG_VERSION").to_string(),
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
    let shutdown = async {
        let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("SIGTERM handler");
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = term.recv() => {}
        }
    };

    let result = tokio::select! {
        served = server.serve(cfg, RpcFactory { service, watcher }) => served.map_err(|e| {
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

async fn serve_stdio_session() -> Result<(), i32> {
    // Nothing may be printed to stdout: it is the wire.
    let service = Arc::new(Service::open().await.map_err(|e| {
        eprintln!("cannot open the service: {e}");
        1
    })?);

    let watcher = Watcher::new(service.clone());
    tokio::spawn(watcher.clone().run());

    let cfg = HandshakeConfig {
        daemon_version: env!("CARGO_PKG_VERSION").to_string(),
        granted_scope: Scope::HostAdmin,
    };

    overnight_transport::serve_stdio(cfg, RpcFactory { service, watcher }).await.map_err(|e| {
        eprintln!("stdio session ended: {e}");
        1
    })
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
}

impl Handler for RpcFactory {
    fn handle(&self, req: Request) -> impl std::future::Future<Output = Response> + Send {
        let rpc = Rpc::new(self.service.clone(), self.watcher.clone(), Scope::HostAdmin);
        async move { rpc.handle(req).await }
    }

    /// Every connection is subscribed to the push stream.
    ///
    /// No opt-in request: a client that connected wants to know when something
    /// changes, and making it ask would just be a round trip before the first
    /// event. Cost is zero on a quiet host, because only changes are sent.
    fn events(&self) -> Option<tokio::sync::broadcast::Receiver<overnight_protocol::v1::Event>> {
        Some(self.watcher.subscribe())
    }
}
