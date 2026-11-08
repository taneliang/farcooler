//! Watching a running engine agent's terminal over the C ABI, read-only (ov-976).

use super::*;

/// Start watching a running engine agent's terminal, read-only (ov-976).
///
/// Chunks arrive through `poll` exactly as a terminal stream's do, under the stream name
/// `"watch:<workspace>/<item>"` (the name `farcooler_client_stream_stop` takes to stop it).
/// The first bytes are the pane's size, as the runner's own size marker. The stream ends with
/// `{"stream": …, "ended": true}` when the agent's pane was reaped, and with
/// `{"stream": …, "ended": true, "lost": true}` when the link dropped first.
///
/// There is no write path: the session is asked for one `workflow.watch` and then only read.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn farcooler_client_watch_start(
    handle: *mut c_void,
    workspace: *const c_char,
    item: *const c_char,
) -> bool {
    guarded(false, || {
        let Some(h) = (unsafe { as_handle(handle) }) else { return false };
        let Some(workspace) = (unsafe { read_str(workspace) }) else { return false };
        let Some(item) = (unsafe { read_str(item) }) else { return false };
        let Ok(id) = workspace.parse::<uuid::Uuid>() else { return false };
        let Some(session) = locked(&h.session).clone() else { return false };
        let finished = h.finished.clone();
        let streams = h.streams.clone();
        let key = watch_key(&workspace, &item);
        let name = key.clone();

        let task = h.runtime.spawn(async move {
            use tokio::io::AsyncReadExt;

            let reader = match session.watch_stream(id, &item).await {
                Ok(reader) => reader,
                Err(e) => {
                    // `denied`: the runner refused this person (only the owner may watch), which is not "not running".
                    let denied = matches!(&e, crate::session::SessionError::Refused { code, .. }
                        if *code == farcooler_protocol::v1::ErrorCode::ScopeDenied as i32);
                    push_line(&finished, json!({ "stream": name, "error": e.to_string(), "denied": denied }).to_string());
                    return;
                }
            };
            drop(session);

            let mut reader = reader;
            let mut buf = vec![0u8; 16 * 1024];
            loop {
                match reader.read(&mut buf).await {
                    Ok(0) => break,
                    Ok(n) => push_line(
                        &finished,
                        json!({ "stream": name, "chunk": farcooler_core::base64::encode(&buf[..n]) }).to_string(),
                    ),
                    Err(_) => {
                        push_line(&finished, json!({ "stream": name, "ended": true, "lost": true }).to_string());
                        return;
                    }
                }
            }
            push_line(&finished, json!({ "stream": name, "ended": true }).to_string());
        });

        let mut running = locked(&streams);
        if let Some(previous) = running.insert(key, task) {
            previous.abort();
        }
        true
    })
}

/// The stream name a watch runs under.
pub fn watch_key(workspace: &str, item: &str) -> String {
    format!("watch:{workspace}/{item}")
}
