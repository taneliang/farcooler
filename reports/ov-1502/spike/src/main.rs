//! ov-1502 spike: an SSH byte stream inside a WebRTC data channel.
//!
//! Not product code. Two roles:
//!
//!   dcspike serve <listen> <ssh-target> [ice...]
//!       The "runner". Answers offers POSTed to http://<listen>/offer (standing in
//!       for the signaling worker), serves the browser test page at /, and for
//!       each data channel: label "ssh" is spliced to <ssh-target> over TCP,
//!       label "echo" echoes every message.
//!
//!   dcspike pipe <signal-url> [ice...]
//!       The "browser" side in Rust, usable as an ssh ProxyCommand: opens a
//!       data channel labelled "ssh" and splices it to stdin/stdout.
//!
//!   dcspike relay <ws-url> <runner-token> <ssh-target>
//!       ov-1534: the "runner" on the relay's signaling instead of HTTP. Keeps
//!       a WebSocket to <ws-url> (`/v1/rtc/runner`), takes each session's
//!       `ice_servers`, answers its offer, and trickles candidates both ways.
//!
//! An ice argument is `url` or `url,username,credential`. POLICY=relay in the
//! environment forces relayed candidates only.
use anyhow::{Context, Result, anyhow};
use bytes::BytesMut;
use std::sync::Arc;
use std::time::Instant;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use webrtc::data_channel::{DataChannel, DataChannelEvent};
use webrtc::peer_connection::{
    MediaEngine, PeerConnection, PeerConnectionBuilder, PeerConnectionEventHandler,
    RTCConfigurationBuilder, RTCIceGatheringState, RTCIceServer, RTCIceTransportPolicy,
    RTCPeerConnectionState, RTCSessionDescription, Registry, register_default_interceptors,
};
use webrtc::runtime::default_runtime;

/// Stand-in for trickle ICE: production sends candidates as they appear; the
/// spike waits for gathering to finish, capped at GATHER_MS (default: no cap).
async fn gather_wait(rx: &mut mpsc::Receiver<()>) {
    match std::env::var("GATHER_MS").ok().and_then(|v| v.parse::<u64>().ok()) {
        Some(ms) => { let _ = tokio::time::timeout(std::time::Duration::from_millis(ms), rx.recv()).await; }
        None => { let _ = rx.recv().await; }
    }
}

const PAGE: &str = include_str!("page.html");

fn ice_servers(args: &[String]) -> Vec<RTCIceServer> {
    args.iter()
        .map(|a| {
            let mut it = a.splitn(3, ',');
            RTCIceServer {
                urls: vec![it.next().unwrap().to_string()],
                username: it.next().unwrap_or("").to_string(),
                credential: it.next().unwrap_or("").to_string(),
                ..Default::default()
            }
        })
        .collect()
}

struct Handler {
    gathered: mpsc::Sender<()>,
    ssh_target: Option<String>,
    started: Instant,
    /// ov-1534: the session and the relay socket to trickle candidates to.
    trickle: Option<(String, mpsc::UnboundedSender<serde_json::Value>)>,
}

#[async_trait::async_trait]
impl PeerConnectionEventHandler for Handler {
    async fn on_ice_candidate(&self, event: webrtc::peer_connection::RTCPeerConnectionIceEvent) {
        let Some((session, out)) = &self.trickle else { return };
        match event.candidate.to_json() {
            Ok(init) => {
                eprintln!("[{:>6} ms] trickle {}", self.started.elapsed().as_millis(), init.candidate);
                let _ = out.send(serde_json::json!({
                    "session": session, "type": "candidate",
                    "candidate": init.candidate, "sdpMid": "0", "sdpMLineIndex": 0,
                }));
            }
            Err(e) => eprintln!("candidate to_json: {e}"),
        }
    }
    async fn on_ice_gathering_state_change(&self, state: RTCIceGatheringState) {
        if state == RTCIceGatheringState::Complete {
            let _ = self.gathered.try_send(());
        }
    }
    async fn on_connection_state_change(&self, state: RTCPeerConnectionState) {
        eprintln!("[{:>6} ms] peer connection {state}", self.started.elapsed().as_millis());
    }
    async fn on_data_channel(&self, dc: Arc<dyn DataChannel>) {
        let target = self.ssh_target.clone();
        tokio::spawn(async move {
            let label = dc.label().await.unwrap_or_default();
            match (label.as_str(), target) {
                ("ssh", Some(t)) => {
                    if let Err(e) = splice_tcp(dc, &t).await {
                        eprintln!("ssh splice ended: {e:#}");
                    }
                }
                _ => echo(dc).await,
            }
        });
    }
}

async fn echo(dc: Arc<dyn DataChannel>) {
    while let Some(ev) = dc.poll().await {
        match ev {
            DataChannelEvent::OnMessage(m) => {
                let _ = dc.send(BytesMut::from(&m.data[..])).await;
            }
            DataChannelEvent::OnClose => break,
            _ => {}
        }
    }
}

/// Data channel <-> TCP, both directions, as the runner would hand bytes to sshd.
async fn splice_tcp(dc: Arc<dyn DataChannel>, target: &str) -> Result<()> {
    let tcp = TcpStream::connect(target).await?;
    tcp.set_nodelay(true)?;
    let (mut rd, mut wr) = tcp.into_split();
    let out = dc.clone();
    tokio::spawn(async move {
        let mut buf = vec![0u8; 16 * 1024];
        loop {
            match rd.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if out.send(BytesMut::from(&buf[..n])).await.is_err() {
                        break;
                    }
                }
            }
        }
        let _ = out.close().await;
    });
    while let Some(ev) = dc.poll().await {
        match ev {
            DataChannelEvent::OnMessage(m) => wr.write_all(&m.data).await?,
            DataChannelEvent::OnClose => break,
            _ => {}
        }
    }
    Ok(())
}

async fn new_pc(
    ice: &[String],
    ssh_target: Option<String>,
) -> Result<(Arc<dyn PeerConnection>, mpsc::Receiver<()>)> {
    new_pc_with(ice_servers(ice), ssh_target, None).await
}

async fn new_pc_with(
    servers: Vec<RTCIceServer>,
    ssh_target: Option<String>,
    trickle: Option<(String, mpsc::UnboundedSender<serde_json::Value>)>,
) -> Result<(Arc<dyn PeerConnection>, mpsc::Receiver<()>)> {
    let mut media = MediaEngine::default();
    let registry = register_default_interceptors(Registry::new(), &mut media)?;
    let mut cfg = RTCConfigurationBuilder::new().with_ice_servers(servers);
    if std::env::var("POLICY").as_deref() == Ok("relay") {
        cfg = cfg.with_ice_transport_policy(RTCIceTransportPolicy::Relay);
    }
    let (tx, rx) = mpsc::channel(1);
    let pc = PeerConnectionBuilder::new()
        .with_configuration(cfg.build())
        .with_media_engine(media)
        .with_interceptor_registry(registry)
        .with_handler(Arc::new(Handler { gathered: tx, ssh_target, started: Instant::now(), trickle }))
        .with_runtime(default_runtime().ok_or_else(|| anyhow!("no runtime"))?)
        .with_udp_addrs(vec![std::env::var("UDP_BIND").unwrap_or_else(|_| "0.0.0.0:0".into())])
        .build()
        .await?;
    Ok((Arc::new(pc), rx))
}

async fn answer(offer_json: &str, ice: &[String], target: &str) -> Result<String> {
    if std::env::var("SHOW_SDP").is_ok() { eprintln!("[serve] offer {offer_json}"); }
    let offer: RTCSessionDescription = serde_json::from_str(offer_json)?;
    let (pc, mut gathered) = new_pc(ice, Some(target.to_string())).await?;
    let sdp_text = offer.sdp.clone();
    pc.set_remote_description(offer).await?;
    // ADD_CANDS=1: also hand the offer's a=candidate lines to the agent, as
    // trickle ICE would, to test whether candidates in the SDP are used.
    if std::env::var("ADD_CANDS").is_ok() {
        for line in sdp_text.lines().filter_map(|l| l.trim().strip_prefix("a=")).filter(|l| l.starts_with("candidate:")) {
            let init = webrtc::peer_connection::RTCIceCandidateInit {
                candidate: line.to_string(), sdp_mid: Some("0".into()), sdp_mline_index: Some(0), ..Default::default()
            };
            if let Err(e) = pc.add_ice_candidate(init).await { eprintln!("add_ice_candidate: {e}"); }
        }
    }
    let ans = pc.create_answer(None).await?;
    pc.set_local_description(ans).await?;
    gather_wait(&mut gathered).await;
    let local = pc.local_description().await.context("no local description")?;
    if std::env::var("SHOW_SDP").is_ok() { eprintln!("[serve] answer {}", serde_json::to_string(&local)?); }
    // Keep the connection alive for the life of the process (spike).
    std::mem::forget(pc);
    Ok(serde_json::to_string(&local)?)
}

async fn serve(listen: String, target: String, ice: Vec<String>) -> Result<()> {
    let l = TcpListener::bind(&listen).await?;
    eprintln!("signaling on http://{listen}/ ; ssh channel -> {target}");
    loop {
        let (mut s, _) = l.accept().await?;
        let ice = ice.clone();
        let target = target.clone();
        tokio::spawn(async move {
            let mut buf = Vec::new();
            let mut tmp = [0u8; 8192];
            // read headers + body (Content-Length)
            let (head_end, len) = loop {
                let n = s.read(&mut tmp).await.unwrap_or(0);
                if n == 0 {
                    return;
                }
                buf.extend_from_slice(&tmp[..n]);
                if let Some(p) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                    let head = String::from_utf8_lossy(&buf[..p]).to_lowercase();
                    let len = head
                        .lines()
                        .find_map(|l| l.strip_prefix("content-length:"))
                        .map(|v| v.trim().parse::<usize>().unwrap_or(0))
                        .unwrap_or(0);
                    break (p + 4, len);
                }
            };
            while buf.len() < head_end + len {
                let n = s.read(&mut tmp).await.unwrap_or(0);
                if n == 0 {
                    break;
                }
                buf.extend_from_slice(&tmp[..n]);
            }
            let first = String::from_utf8_lossy(&buf[..buf.iter().position(|&b| b == b'\r').unwrap_or(0)]).to_string();
            let (ctype, body) = if first.starts_with("POST /offer") {
                let offer = String::from_utf8_lossy(&buf[head_end..]).to_string();
                match answer(&offer, &ice, &target).await {
                    Ok(a) => ("application/json", a),
                    Err(e) => ("text/plain", format!("error: {e:#}")),
                }
            } else {
                ("text/html", PAGE.to_string())
            };
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nAccess-Control-Allow-Origin: *\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = s.write_all(resp.as_bytes()).await;
        });
    }
}

async fn pipe(url: String, ice: Vec<String>) -> Result<()> {
    let t0 = Instant::now();
    let (pc, mut gathered) = new_pc(&ice, None).await?;
    let dc = pc.create_data_channel("ssh", None).await?;
    let offer = pc.create_offer(None).await?;
    pc.set_local_description(offer).await?;
    gather_wait(&mut gathered).await;
    eprintln!("[pipe] offer gathered after {} ms", t0.elapsed().as_millis());
    let local = serde_json::to_string(&pc.local_description().await.context("no offer")?)?;
    if std::env::var("SHOW_SDP").is_ok() { eprintln!("{local}"); }
    let hostport = url.trim_start_matches("http://").trim_end_matches('/').to_string();
    let mut s = TcpStream::connect(&hostport).await?;
    let req = format!(
        "POST /offer HTTP/1.1\r\nHost: {hostport}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{local}",
        local.len()
    );
    s.write_all(req.as_bytes()).await?;
    let mut resp = String::new();
    s.read_to_string(&mut resp).await?;
    let body = &resp[resp.find("\r\n\r\n").context("bad response")? + 4..];
    let ans: RTCSessionDescription = serde_json::from_str(body).with_context(|| body.to_string())?;
    eprintln!("[pipe] answer received after {} ms", t0.elapsed().as_millis());
    if std::env::var("SHOW_SDP").is_ok() { eprintln!("{body}"); }
    pc.set_remote_description(ans).await?;

    // Wait for open, then splice stdin/stdout.
    loop {
        match dc.poll().await {
            Some(DataChannelEvent::OnOpen) => break,
            Some(_) => continue,
            None => return Err(anyhow!("channel ended before open")),
        }
    }
    eprintln!("[pipe] data channel open after {} ms", t0.elapsed().as_millis());
    let out = dc.clone();
    tokio::spawn(async move {
        let mut stdin = tokio::io::stdin();
        let mut buf = vec![0u8; 16 * 1024];
        loop {
            match stdin.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if out.send(BytesMut::from(&buf[..n])).await.is_err() {
                        break;
                    }
                }
            }
        }
        let _ = out.close().await;
    });
    let mut stdout = tokio::io::stdout();
    while let Some(ev) = dc.poll().await {
        match ev {
            DataChannelEvent::OnMessage(m) => {
                stdout.write_all(&m.data).await?;
                stdout.flush().await?;
            }
            DataChannelEvent::OnClose => break,
            _ => {}
        }
    }
    let _ = pc.close().await;
    Ok(())
}

/// ov-1534: a runner on the relay's signaling WebSocket. One peer connection
/// per session; servers from the session's `ice_servers`; the answer sent as
/// soon as it is made and every candidate trickled as it is found.
async fn relay(ws_url: String, token: String, target: String) -> Result<()> {
    use futures_util::{SinkExt, StreamExt};
    use std::collections::HashMap;
    use tokio_tungstenite::tungstenite::{Message, client::IntoClientRequest};

    let started = Instant::now();
    let mut req = ws_url.as_str().into_client_request()?;
    req.headers_mut().insert("authorization", format!("Bearer {token}").parse()?);
    let (ws, _) = tokio_tungstenite::connect_async(req).await.context("relay websocket")?;
    eprintln!("[relay] connected to {ws_url}");
    let (mut sink, mut stream) = ws.split();
    let (out, mut out_rx) = mpsc::unbounded_channel::<serde_json::Value>();
    tokio::spawn(async move {
        while let Some(v) = out_rx.recv().await {
            if sink.send(Message::Text(v.to_string().into())).await.is_err() {
                break;
            }
        }
    });

    let mut servers: HashMap<String, Vec<RTCIceServer>> = HashMap::new();
    let mut pcs: HashMap<String, Arc<dyn PeerConnection>> = HashMap::new();
    while let Some(frame) = stream.next().await {
        let Message::Text(text) = frame? else { continue };
        let m: serde_json::Value = serde_json::from_str(&text)?;
        let Some(session) = m["session"].as_str().map(str::to_string) else { continue };
        let ms = started.elapsed().as_millis();
        match m["type"].as_str().unwrap_or("") {
            "ice_servers" => {
                // webrtc-rs takes TURN over UDP only (ov-1502 report §2).
                let urls: Vec<String> = m["urls"].as_array().into_iter().flatten()
                    .filter_map(|u| u.as_str())
                    .filter(|u| !u.starts_with("turns:") && !u.contains("transport=tcp"))
                    .map(str::to_string).collect();
                eprintln!("[{ms:>6} ms] {session}: ice_servers {urls:?} stun_only={}", m["stun_only"]);
                let entry = RTCIceServer {
                    urls,
                    username: m["username"].as_str().unwrap_or("").to_string(),
                    credential: m["credential"].as_str().unwrap_or("").to_string(),
                    ..Default::default()
                };
                servers.insert(session, vec![entry]);
            }
            "offer" => {
                eprintln!("[{ms:>6} ms] {session}: offer from device {}", m["device"]);
                let ice = servers.get(&session).cloned().unwrap_or_default();
                let (pc, _gathered) = new_pc_with(ice, Some(target.clone()), Some((session.clone(), out.clone()))).await?;
                pc.set_remote_description(RTCSessionDescription::offer(m["sdp"].as_str().unwrap_or("").to_string())?).await?;
                let ans = pc.create_answer(None).await?;
                pc.set_local_description(ans).await?;
                let local = pc.local_description().await.context("no local description")?;
                let _ = out.send(serde_json::json!({ "session": session, "type": "answer", "sdp": local.sdp }));
                eprintln!("[{:>6} ms] {session}: answer sent", started.elapsed().as_millis());
                pcs.insert(session, pc);
            }
            "candidate" => {
                let candidate = m["candidate"].as_str().unwrap_or("").to_string();
                if candidate.is_empty() { continue }
                if let Some(pc) = pcs.get(&session) {
                    let init = webrtc::peer_connection::RTCIceCandidateInit {
                        candidate: candidate.clone(),
                        sdp_mid: m["sdpMid"].as_str().map(str::to_string),
                        sdp_mline_index: m["sdpMLineIndex"].as_u64().map(|i| i as u16),
                        ..Default::default()
                    };
                    match pc.add_ice_candidate(init).await {
                        Ok(()) => eprintln!("[{ms:>6} ms] {session}: remote candidate {candidate}"),
                        Err(e) => eprintln!("[{ms:>6} ms] {session}: add_ice_candidate: {e}"),
                    }
                }
            }
            "end" => {
                eprintln!("[{ms:>6} ms] {session}: end {}", m["reason"]);
                if let Some(pc) = pcs.remove(&session) { let _ = pc.close().await; }
            }
            other => eprintln!("[{ms:>6} ms] {session}: unexpected {other}"),
        }
    }
    eprintln!("[relay] websocket closed");
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();
    let a: Vec<String> = std::env::args().collect();
    match a.get(1).map(String::as_str) {
        Some("serve") => serve(a[2].clone(), a[3].clone(), a[4..].to_vec()).await,
        Some("pipe") => pipe(a[2].clone(), a[3..].to_vec()).await,
        Some("relay") => relay(a[2].clone(), a[3].clone(), a[4].clone()).await,
        _ => Err(anyhow!("usage: dcspike serve <listen> <ssh-target> [ice..] | pipe <url> [ice..]")),
    }
}
