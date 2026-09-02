//! The real backend: the C symbols `crates/tailcat/go` exports.
//!
//! Every errno this file compares is compared against a `libc::` constant,
//! never a literal. The numbers are platform-dependent — darwin and linux do
//! not agree on them except for `EINVAL` and `EIO` — and a literal copied out
//! of a comment or a report would be silently wrong on the other platform.

use super::TunnelError;
use std::ffi::CString;
use std::os::fd::FromRawFd;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

unsafe extern "C" {
    fn fc_tailcat_dial(token: *const i8, client_key: *const i8, port: u16) -> i32;
    fn fc_tailcat_serve(key_path: *const i8, ssh_port: u16, allow: *const i8) -> i32;
    fn fc_tailcat_conn_blob(buf: *mut i8, len: usize) -> i32;
    fn fc_tailcat_allow_add(node_key: *const i8) -> i32;
    fn fc_tailcat_set_derp_map_url(url: *const i8) -> i32;
}

/// A string that cannot cross the FFI (it holds an embedded NUL) is a local,
/// caller-side mistake, not a network condition — it gets `Io`, never `Derp`
/// or `NoAnswer`, both of which would misname what actually happened.
fn embedded_nul(what: &str) -> TunnelError {
    TunnelError::Io(std::io::Error::new(
        std::io::ErrorKind::InvalidInput,
        format!("{what} contains a NUL byte"),
    ))
}

pub async fn dial(
    token: &str,
    client_key: &str,
    port: u16,
) -> Result<tokio::net::UnixStream, TunnelError> {
    let token = CString::new(token).map_err(|_| embedded_nul("the token"))?;
    let key = CString::new(client_key).map_err(|_| embedded_nul("the client key"))?;
    // The Go side blocks on DERP and a WireGuard handshake, which is seconds
    // and not microseconds. Blocking a tokio worker for that would stall every
    // other connection this process is serving.
    let fd = tokio::task::spawn_blocking(move || unsafe {
        fc_tailcat_dial(token.as_ptr(), key.as_ptr(), port)
    })
    .await
    .map_err(|_| TunnelError::Derp)?;

    match fd {
        fd if fd >= 0 => {
            let std = unsafe { std::os::unix::net::UnixStream::from_raw_fd(fd) };
            std.set_nonblocking(true)?;
            Ok(tokio::net::UnixStream::from_std(std)?)
        }
        // EHOSTUNREACH: the relay the token names does not answer. This is
        // the one leg that is actually the rendezvous service's fault.
        fd if fd == -libc::EHOSTUNREACH => Err(TunnelError::Derp),
        // ETIMEDOUT: the relay answers, the runner does not. Tailcat ignores
        // an unrecognized client SILENTLY, so this is also what a revoked
        // device gets — see `TunnelError::NoAnswer`'s doc comment.
        fd if fd == -libc::ETIMEDOUT => Err(TunnelError::NoAnswer),
        // Everything else is a local/IO-class failure, not a statement about
        // the relay or a silent runner, so none of it borrows Derp's or
        // NoAnswer's words:
        //   EINVAL        the token or the key is not one this can use — the
        //                  caller's own mistake, not the network.
        //   ENETDOWN       the local data plane would not come up — ours.
        //   ECONNREFUSED   the tunnel is up and the TCP connect through it to
        //                  sshd failed. Deliberately NOT worded as "SSH
        //                  refused": the tunnel reached the runner and
        //                  nothing was listening, which the ConnectionRefused
        //                  `io::ErrorKind` this carries already says exactly
        //                  as it would for a Direct runner's own refused
        //                  connect — "today's message, unchanged" per the
        //                  design's failure table.
        //   EMFILE         the socketpair could not be created.
        //   anything else  an errno this file does not yet have a name for.
        //                  `Io` is still an honest answer: it never claims
        //                  Derp or NoAnswer's more specific stories.
        fd => Err(TunnelError::Io(std::io::Error::from_raw_os_error(-fd))),
    }
}

pub fn serve(key_path: &Path, ssh_port: u16, allow: &[String]) -> Result<(), TunnelError> {
    let key_path =
        CString::new(key_path.as_os_str().as_bytes()).map_err(|_| embedded_nul("the key path"))?;
    // Newline-separated: what `parseAllowed` on the Go side splits on.
    let allow = CString::new(allow.join("\n")).map_err(|_| embedded_nul("an allowed node key"))?;
    let rc = unsafe { fc_tailcat_serve(key_path.as_ptr(), ssh_port, allow.as_ptr()) };
    if rc < 0 {
        // EINVAL (an allowlist that admits nobody, or unreadable arguments),
        // EACCES (the runner's own key file) and EIO (`Server.Start` failed,
        // or a panic was recovered) are all reported the same way here: none
        // of them is a sentence this crate's callers need split out further,
        // and `Io` never claims more than "serving did not work."
        return Err(TunnelError::Io(std::io::Error::from_raw_os_error(-rc)));
    }
    Ok(())
}

pub fn conn_blob() -> Result<String, TunnelError> {
    // A production ConnBlob measured at 184 bytes (Task 3's live test); this
    // leaves generous headroom rather than sizing the buffer to that one
    // measurement — a region with more nodes makes a longer token.
    let mut buf = vec![0u8; 1024];
    let rc = unsafe { fc_tailcat_conn_blob(buf.as_mut_ptr().cast::<i8>(), buf.len()) };
    if rc < 0 {
        // ERANGE (the buffer was too small) and ENOTCONN (no server running)
        // both land here; `Io` says truthfully that the read did not work
        // without inventing a distinction its one caller does not need.
        return Err(TunnelError::Io(std::io::Error::from_raw_os_error(-rc)));
    }
    buf.truncate(rc as usize);
    String::from_utf8(buf).map_err(|_| {
        TunnelError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "the runner's conn blob was not valid UTF-8",
        ))
    })
}

pub fn allow_add(node_key: &str) -> Result<(), TunnelError> {
    let node_key = CString::new(node_key).map_err(|_| embedded_nul("the node key"))?;
    let rc = unsafe { fc_tailcat_allow_add(node_key.as_ptr()) };
    if rc < 0 {
        // EINVAL (an unusable or all-zero node key) or ENOTCONN (no server
        // running to admit anyone to).
        return Err(TunnelError::Io(std::io::Error::from_raw_os_error(-rc)));
    }
    Ok(())
}

pub fn set_derp_map_url(url: &str) {
    let Ok(url) = CString::new(url) else {
        // Recording configuration is not a call anyone awaits an error from
        // (see `super::set_derp_map_url`'s signature), so an unusable value
        // is logged rather than lost silently.
        tracing::warn!("tailcat: DERP map URL contains a NUL byte; ignoring it");
        return;
    };
    let rc = unsafe { fc_tailcat_set_derp_map_url(url.as_ptr()) };
    if rc != 0 {
        tracing::warn!(rc, "tailcat: setting the DERP map URL failed");
    }
}
