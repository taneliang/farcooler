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

use overnight_daemon::{paths, rpc::Rpc, service::Service};
use overnight_protocol::v1::{Request, Response, Scope};
use overnight_transport::{HandshakeConfig, Handler, UnixListenerServer};

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "overnight=info,warn".into()),
        )
        .with_writer(std::io::stderr)
        .init();

    if let Err(code) = run().await {
        std::process::exit(code);
    }
}

async fn run() -> Result<(), i32> {
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
        served = server.serve(cfg, RpcFactory { service }) => served.map_err(|e| {
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

/// One `Rpc` per request, over one shared `Service`.
///
/// The scope belongs to the connection; the service does not, and must not — a
/// second `Service` would mean a second SQLite handle and a second, divergent
/// view of the tmux inventory.
#[derive(Clone)]
struct RpcFactory {
    service: Arc<Service>,
}

impl Handler for RpcFactory {
    fn handle(&self, req: Request) -> impl std::future::Future<Output = Response> + Send {
        let rpc = Rpc::new(self.service.clone(), Scope::HostAdmin);
        async move { rpc.handle(req).await }
    }
}
