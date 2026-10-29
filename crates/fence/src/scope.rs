//! A Key A line with another scope word (ov-856), in its own file: `lib.rs`
//! is at its size ceiling.

use farcooler_protocol::v1::Scope;

use super::{Entry, Grant, Rejected, key_a_line, key_a_options, passkey, split_options, usable_client_id};

/// One of our own Key A lines, rewritten to carry `scope` (ov-856).
///
/// The scope word and nothing else: the key material, the comment, the
/// client id and the node key come back as the entry has them, through the
/// same `key_a_line` that wrote the line. Rebuilt, never spliced, as
/// [`with_node_key`] is, and refusing the same lines it refuses (a plain
/// line, a foreign one, one whose scope word this build can't read).
///
/// **Refused unless the options are exactly the ones this build writes** for
/// that entry (the slice 1 review's SF3): a line with another program, an
/// extra option or another spelling would otherwise come back rewritten to
/// ours, which is more than the scope word changing. Such a line is left
/// alone by the caller.
///
/// It does not decide whether `scope` may be higher than the line's: that
/// is the caller's rule (a rebuild only lowers; a promotion names who).
pub fn with_scope(entry: &Entry, scope: Scope) -> Result<String, Rejected> {
    if entry.shell_access || !usable_client_id(&entry.client_id) {
        return Err(Rejected::NodeKey);
    }
    if matches!(entry.scope, Scope::Unspecified) || matches!(scope, Scope::Unspecified) {
        return Err(Rejected::Unscoped);
    }
    let (options, rest) = split_options(&entry.line).ok_or(Rejected::Unparseable)?;
    let node = (!entry.node_key.is_empty()).then_some(entry.node_key.as_str());
    if options != key_a_options(&entry.client_id, entry.scope, node) {
        return Err(Rejected::Unparseable);
    }
    let parsed = ssh_key::PublicKey::from_openssh(rest).map_err(|e| {
        tracing::debug!(error = %e, "a line in the fence did not parse back as a key");
        Rejected::Unparseable
    })?;
    if !passkey::admits(&parsed, Grant::FarCooler) {
        return Err(Rejected::Algorithm);
    }
    let key = ssh_key::PublicKey::new(parsed.key_data().clone(), parsed.comment().to_string());
    let body = key.to_openssh().map_err(|e| {
        tracing::debug!(error = %e, "a parsed key could not be re-encoded");
        Rejected::Unparseable
    })?;
    let line = key_a_line(&body, &entry.client_id, scope, node);
    debug_assert!(!line.contains(['\r', '\n']));
    Ok(line)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BEGIN, END, Grant, parse, render};

    const KEY: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAICIiIiIiIiIiIiIiIiIiIiIiIiIiIiIiIiIiIiIiIiIi";
    const NODE: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";

    fn entry(scope: Scope, node: Option<&str>) -> Entry {
        let line = render(KEY, "Bob's iPhone", "phone-b", scope, Grant::FarCooler, node).unwrap();
        parse(&format!("{BEGIN}\n{line}\n{END}\n")).unwrap().remove(0)
    }

    /// Only the scope word changes: the same bytes come back for the same
    /// scope, with or without a node key, and one word differs for another.
    #[test]
    fn only_the_scope_word_changes() {
        for node in [None, Some(NODE)] {
            let e = entry(Scope::Control, node);
            assert_eq!(with_scope(&e, Scope::Control).unwrap(), e.line);
            let read = with_scope(&e, Scope::Read).unwrap();
            assert_eq!(read, e.line.replace("--scope control", "--scope read"));
            let back = parse(&format!("{BEGIN}\n{read}\n{END}\n")).unwrap().remove(0);
            assert_eq!((back.scope, back.client_id, back.node_key, back.label), (Scope::Read, e.client_id, e.node_key, e.label));
        }
    }

    /// A line whose options aren't the ones this build writes (another program,
    /// an added option) is refused, not rewritten to ours.
    #[test]
    fn a_line_with_other_options_is_refused() {
        let e = entry(Scope::Control, None);
        let other = e.line.replace("restrict,", "restrict,no-pty,");
        let mut changed = e.clone();
        changed.line = other;
        assert!(with_scope(&changed, Scope::Read).is_err());
        let mut elsewhere = e.clone();
        elsewhere.line = e.line.replace("--stdio", "--stdio --extra");
        assert!(with_scope(&elsewhere, Scope::Read).is_err());
    }

    /// A plain line, a line this build can't read, and a scope it can't
    /// write are all refused, as `with_node_key` refuses them.
    #[test]
    fn lines_it_must_not_touch_are_refused() {
        let shell = render(KEY, "Mac", "mac", Scope::HostAdmin, Grant::Shell, None).unwrap();
        let shell = parse(&format!("{BEGIN}\n{shell}\n{END}\n")).unwrap().remove(0);
        assert!(with_scope(&shell, Scope::Read).is_err());
        let mut unknown = entry(Scope::Control, None);
        unknown.scope = Scope::Unspecified;
        assert!(with_scope(&unknown, Scope::Read).is_err());
        assert!(with_scope(&entry(Scope::Control, None), Scope::Unspecified).is_err());
    }
}
