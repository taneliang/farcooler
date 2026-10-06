//! What a pane is serving, read from the kernel rather than from its output.
//!
//! A pane running `python -m http.server 8099` either holds a listening socket
//! on 8099 or it does not; that is a fact about the host, available without
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

/// Every listening TCP port on this host, by owning process.
///
/// One call for the whole host, on the sampling loop's cadence, for the same
/// reason `ps` is: a fleet of thirty panes must not mean thirty processes a
/// second.
///
/// Failure is silently empty. A host without `lsof`, or one where it is
/// refused, loses a decoration — it must not lose the row.
pub fn listening_ports() -> HashMap<i32, Vec<u16>> {
    let out = std::process::Command::new("lsof")
        .args(["-nP", "-iTCP", "-sTCP:LISTEN", "-Fpn"])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output();
    let Ok(out) = out else { return HashMap::new() };
    parse_lsof(&String::from_utf8_lossy(&out.stdout))
}

/// Listening TCP ports held by these processes only, by owning process.
///
/// `lsof -p` over a handful of pids, where `listening_ports` walks every file
/// descriptor on the host. The sampling loop asks about the processes under its
/// panes and nothing else; a host with no panes never asks at all.
///
/// Same failure rule: empty, never an error. `lsof` exits non-zero when one of
/// the pids has gone, and still prints the rest, so the exit status is ignored.
pub fn listening_ports_of(pids: &[i32]) -> HashMap<i32, Vec<u16>> {
    if pids.is_empty() {
        return HashMap::new();
    }
    let list = pids.iter().map(i32::to_string).collect::<Vec<_>>().join(",");
    let out = std::process::Command::new("lsof")
        .args(["-nP", "-a", "-p", &list, "-iTCP", "-sTCP:LISTEN", "-Fpn"])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output();
    let Ok(out) = out else { return HashMap::new() };
    parse_lsof(&String::from_utf8_lossy(&out.stdout))
}

/// The wire's `Terminal.ports`: every port a pane serves, lowest first, each
/// once. Sorted because a client shows the first and compares the list to see
/// whether anything moved, and `lsof` names sockets in no stable order.
pub fn field(ports: &[u16]) -> Vec<u32> {
    let mut out: Vec<u32> = ports.iter().map(|&p| u32::from(p)).collect();
    out.sort_unstable();
    out.dedup();
    out
}

/// `lsof -Fpn` output, by owning process.
///
/// Split from [`listening_ports`] so a fixture can stand in for the kernel.
///
/// `-F` is a field-per-line format: `p<pid>` opens a process block, and each
/// `n<name>` under it is one of its sockets.
pub fn parse_lsof(output: &str) -> HashMap<i32, Vec<u16>> {
    let mut found: HashMap<i32, Vec<u16>> = HashMap::new();
    let mut pid: Option<i32> = None;
    for line in output.lines() {
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

    /// What `lsof -nP -iTCP -sTCP:LISTEN -Fpn` printed on a machine running a
    /// dev server that also holds a debugger port, a proxy on IPv6 loopback
    /// and an `*` listener. Two processes, one with the same port twice (IPv4
    /// and IPv6).
    const LSOF: &str = "p4121\nn*:5173\nn[::1]:5173\nn127.0.0.1:9229\np777\nn[::1]:8080\nn*:bogus\n";

    #[test]
    fn ports_are_parsed_from_lsof_output_by_process() {
        let found = parse_lsof(LSOF);
        assert_eq!(found[&4121], vec![5173, 9229], "the same port on two addresses counts once");
        assert_eq!(found[&777], vec![8080], "a name that isn't a port is skipped");
        assert_eq!(found.len(), 2);
    }

    /// The structured field is the parsed ports, lowest first, each once, so a
    /// client can show `:5173` and open it without reading the label.
    #[test]
    fn the_wire_field_is_sorted_and_deduplicated() {
        assert_eq!(field(&parse_lsof(LSOF)[&4121]), vec![5173, 9229]);
        assert_eq!(field(&[9229, 5173, 5173]), vec![5173, 9229]);
        assert!(field(&[]).is_empty());
    }

    #[test]
    fn nothing_listening_is_no_purpose() {
        assert_eq!(purpose(&[]), None);
    }
}
