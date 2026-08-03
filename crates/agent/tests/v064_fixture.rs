#[test]
fn the_new_adapter_produces_no_spurious_gaps() {
    use farcooler_agent::acp::{normalize::update_to_events, wire};
    use farcooler_agent::event::AgentEvent;
    let raw = std::fs::read_to_string("tests/fixtures/session_v064.jsonl").expect("fixture");
    let mut bad = Vec::new();
    for line in raw.lines().filter(|l| !l.trim().is_empty()) {
        let rpc: wire::Rpc = serde_json::from_str(line).expect("frame parses");
        if rpc.method.as_deref() != Some("session/update") { continue; }
        match rpc.session_notification() {
            None => bad.push(format!("UNPARSED NOTIFICATION: {}", &line[..line.len().min(160)])),
            Some(n) => {
                for e in update_to_events(&n.update) {
                    if matches!(e, AgentEvent::Gap { .. }) {
                        bad.push(format!("GAP FROM: {}", &line[..line.len().min(160)]));
                    }
                }
            }
        }
    }
    assert!(bad.is_empty(), "{} problem frames:\n{}", bad.len(), bad.join("\n"));
}
