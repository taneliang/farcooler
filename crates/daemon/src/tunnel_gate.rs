//! One lock over this runner's tunnel (ov-832, the ov-826 review's S1).
//!
//! Two things change the running tunnel. A rebuild (`allowlist::start_tunnel`)
//! replaces the server with one built from `authorized_keys`, and it is how a
//! revoke takes a device's node key back out. An admission (`allow_add`, from
//! `client.enroll` and `client.set_node_key`) adds one key to the server that
//! is running. They used to run unordered: an enroll that wrote its line just
//! before a revoke reached its `allow_add` after the fence lock was released,
//! and when the revoke's rebuild landed in that gap, the revoked device's key
//! went into the server that rebuild had just made without it.
//!
//! So every call into the tunnel goes through [`Held`], which only
//! [`Gate::hold`] hands out, and an admission decides, under the same hold,
//! whether the fence still holds its key. A revoke writes before it rebuilds,
//! so an admission either sees that write and admits nothing, or runs before
//! the rebuild, which then drops what it added.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use farcooler_tailcat::TunnelError;

/// What a tunnel does, as this daemon calls it: `farcooler_tailcat`, or a
/// test's recording. All synchronous, and all run on the blocking pool.
pub trait Backend: Send + Sync {
    fn serve(&self, key_path: &Path, ssh_port: u16, allow: &[String]) -> Result<(), TunnelError>;
    fn allow_add(&self, node_key: &str) -> Result<(), TunnelError>;
    fn conn_blob(&self) -> Result<String, TunnelError>;
    fn ensure_identity(&self, key_path: &Path) -> Result<(), TunnelError>;
}

/// The tunnel this build links, or the stub that refuses everything.
struct Tailcat;

impl Backend for Tailcat {
    fn serve(&self, key_path: &Path, ssh_port: u16, allow: &[String]) -> Result<(), TunnelError> {
        farcooler_tailcat::serve(key_path, ssh_port, allow)
    }
    fn allow_add(&self, node_key: &str) -> Result<(), TunnelError> {
        farcooler_tailcat::allow_add(node_key)
    }
    fn conn_blob(&self) -> Result<String, TunnelError> {
        farcooler_tailcat::conn_blob()
    }
    fn ensure_identity(&self, key_path: &Path) -> Result<(), TunnelError> {
        farcooler_tailcat::ensure_identity(key_path)
    }
}

/// The lock, and the tunnel behind it. One per `Service`.
pub struct Gate {
    lock: tokio::sync::Mutex<()>,
    backend: std::sync::RwLock<Arc<dyn Backend>>,
}

impl Default for Gate {
    fn default() -> Self {
        Gate { lock: tokio::sync::Mutex::new(()), backend: std::sync::RwLock::new(Arc::new(Tailcat)) }
    }
}

impl Gate {
    /// Wait for the tunnel, and hold it until the answer is dropped. A
    /// rebuild can hold it for as long as `serve` takes (30-50 seconds at
    /// worst), and an admission waits that long behind it.
    pub async fn hold(&self) -> Held<'_> {
        let guard = self.lock.lock().await;
        let backend = self.backend.read().unwrap_or_else(|e| e.into_inner()).clone();
        Held { _guard: guard, backend }
    }

    /// A test's tunnel in place of this build's.
    #[cfg(test)]
    pub fn use_backend(&self, backend: Arc<dyn Backend>) {
        *self.backend.write().unwrap() = backend;
    }
}

/// The tunnel, held. The only way to call into it.
pub struct Held<'a> {
    _guard: tokio::sync::MutexGuard<'a, ()>,
    backend: Arc<dyn Backend>,
}

impl Held<'_> {
    /// Each call on the blocking pool: the Go side holds a package-wide
    /// mutex, and `serve` can hold it for tens of seconds.
    async fn off<T: Send + 'static>(
        &self,
        work: impl FnOnce(&dyn Backend) -> Result<T, TunnelError> + Send + 'static,
    ) -> Result<Result<T, TunnelError>, tokio::task::JoinError> {
        let backend = self.backend.clone();
        tokio::task::spawn_blocking(move || work(&*backend)).await
    }

    pub async fn serve(&self, key_path: PathBuf, ssh_port: u16, allow: Vec<String>) -> Result<Result<(), TunnelError>, tokio::task::JoinError> {
        self.off(move |b| b.serve(&key_path, ssh_port, &allow)).await
    }

    pub async fn allow_add(&self, node_key: String) -> Result<Result<(), TunnelError>, tokio::task::JoinError> {
        self.off(move |b| b.allow_add(&node_key)).await
    }

    pub async fn conn_blob(&self) -> Result<Result<String, TunnelError>, tokio::task::JoinError> {
        self.off(|b| b.conn_blob()).await
    }

    pub async fn ensure_identity(&self, key_path: PathBuf) -> Result<Result<(), TunnelError>, tokio::task::JoinError> {
        self.off(move |b| b.ensure_identity(&key_path)).await
    }
}

/// A tunnel that keeps the set it admits, for a test to read.
#[cfg(test)]
pub mod recording {
    use super::*;
    use std::sync::Mutex;

    /// Serving `Some(keys)`, or nothing. `serve` replaces the set, as the
    /// real one does, and an empty allowlist leaves nothing serving.
    #[derive(Default)]
    pub struct Recording {
        serving: Mutex<Option<Vec<String>>>,
        /// Every `serve`'s allowlist, in order.
        pub served: Mutex<Vec<Vec<String>>>,
    }

    impl Recording {
        /// The node keys the running server admits, or `None` with none running.
        pub fn admits(&self) -> Option<Vec<String>> {
            self.serving.lock().unwrap().clone()
        }
    }

    impl Backend for Recording {
        fn serve(&self, _: &Path, _: u16, allow: &[String]) -> Result<(), TunnelError> {
            self.served.lock().unwrap().push(allow.to_vec());
            let mut serving = self.serving.lock().unwrap();
            if allow.is_empty() {
                *serving = None;
                return Err(TunnelError::NoAnswer);
            }
            *serving = Some(allow.to_vec());
            Ok(())
        }
        fn allow_add(&self, node_key: &str) -> Result<(), TunnelError> {
            match self.serving.lock().unwrap().as_mut() {
                Some(keys) => {
                    keys.push(node_key.to_string());
                    Ok(())
                }
                None => Err(TunnelError::NoAnswer),
            }
        }
        fn conn_blob(&self) -> Result<String, TunnelError> {
            self.serving.lock().unwrap().as_ref().map(|_| "token".to_string()).ok_or(TunnelError::NoAnswer)
        }
        fn ensure_identity(&self, key_path: &Path) -> Result<(), TunnelError> {
            std::fs::write(key_path, b"identity").map_err(TunnelError::Io)
        }
    }
}
