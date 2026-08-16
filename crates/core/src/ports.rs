//! What a pane is serving, read from the kernel rather than from its output.
//!
//! A pane running `python -m http.server 8099` either holds a listening socket
//! on 8099 or it does not; that is a fact about the machine, available without
//! interpreting a single character of what the program printed. Pattern
//! matching prose is the mistake the rest of this work exists to correct, and
//! it would be perverse to reintroduce it here.

use std::collections::HashMap;

/// What a pane serving these ports is for.
///
/// The lowest, because a dev server that also opens a debugger port (node's
/// 9229, for one) should read as the server a person started, not the debugger
/// they did not.
pub fn purpose(ports: &[u16]) -> Option<String> {
    ports.iter().min().map(|p| format!("web :{p}"))
}

/// Every listening TCP port on this machine, by owning process.
///
/// One call for the whole host, on the sampling loop's cadence, for the same
/// reason `ps` is: a fleet of thirty panes must not mean thirty processes a
/// second.
///
/// Failure is silently empty. A machine without `lsof`, or one where it is
/// refused, loses a decoration — it must not lose the row.
pub fn listening_ports() -> HashMap<i32, Vec<u16>> {
    let out = std::process::Command::new("lsof")
        .args(["-nP", "-iTCP", "-sTCP:LISTEN", "-Fpn"])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output();
    let Ok(out) = out else { return HashMap::new() };

    // `-F` is a field-per-line format: `p<pid>` opens a process block, and each
    // `n<name>` under it is one of its sockets.
    let mut found: HashMap<i32, Vec<u16>> = HashMap::new();
    let mut pid: Option<i32> = None;
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        let (tag, value) = line.split_at(1.min(line.len()));
        match tag {
            "p" => pid = value.trim().parse().ok(),
            "n" => {
                let Some(pid) = pid else { continue };
                // `*:8099`, `127.0.0.1:8099`, `[::1]:8099`.
                let Some(port) = value.rsplit(':').next().and_then(|p| p.trim().parse::<u16>().ok())
                else {
                    continue;
                };
                let ports = found.entry(pid).or_default();
                if !ports.contains(&port) {
                    ports.push(port);
                }
            }
            _ => {}
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_served_port_reads_as_a_purpose() {
        assert_eq!(purpose(&[8099]), Some("web :8099".to_string()));
    }

    /// Several ports is a fact about the process, not a label. The lowest is
    /// almost always the one a person typed.
    #[test]
    fn many_ports_report_the_lowest() {
        assert_eq!(purpose(&[9229, 5173]), Some("web :5173".to_string()));
    }

    #[test]
    fn nothing_listening_is_no_purpose() {
        assert_eq!(purpose(&[]), None);
    }
}
