//! A fake runner on a TCP port, for the browser check
//! (`services/web/test/browser/csp-check.mjs --fake-runner`).
//!
//! It is `tests/support`'s runner: a russh server with a fixed test host key,
//! admitting any client key, behind a fake daemon with one terminal that
//! echoes what is typed. It prints one JSON line,
//! `{"port":…,"host_key":"SHA256:…"}`, then `typed <hex>` for every
//! `terminal.write`. Never a real runner: nothing here touches a shell.

#[path = "../tests/support/mod.rs"]
mod support;

use std::io::Write as _;
use std::sync::Arc;

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let port = listener.local_addr().expect("address").port();
    println!(r#"{{"port":{port},"host_key":"{}"}}"#, support::fingerprint(support::HOST_KEY));
    let _ = std::io::stdout().flush();

    let (typing, mut typed) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
    tokio::spawn(async move {
        while let Some(bytes) = typed.recv().await {
            let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
            println!("typed {hex}");
            let _ = std::io::stdout().flush();
        }
    });
    loop {
        let Ok((stream, _)) = listener.accept().await else { continue };
        let seen = Arc::new(support::Seen::default());
        *seen.typing.lock().unwrap() = Some(typing.clone());
        support::serve(stream, support::HOST_KEY, None, seen);
    }
}
