//! [`names_client`]: whether an `authorized_keys` file names a device (ov-980), in its own file: `lib.rs` is
//! at its size ceiling.

use super::{SHELL_COMMENT, usable_client_id};

/// Whether any line of `text`, an `authorized_keys` file read whole, names
/// `client_id` as a line of ours does (ov-980): `--client <id>` in a forced
/// command, or a plain line's `farcooler-shell-<name>.<id>` comment. Read
/// loosely on purpose, past options a person added and outside the markers,
/// so a device whose line was edited by hand is still known: the daemon asks
/// only whether a device id has any line at all, and sshd, not this, decides
/// what the line grants.
pub fn names_client(text: &str, client_id: &str) -> bool {
    if !usable_client_id(client_id) {
        return false;
    }
    let forced = format!("--client {client_id}");
    text.lines().map(str::trim).filter(|line| !line.is_empty() && !line.starts_with('#')).any(|line| {
        let in_command = line.match_indices(&forced).any(|(at, _)| {
            line[at + forced.len()..].chars().next().is_none_or(|c| c.is_whitespace() || c == '"' || c == '\'')
        });
        in_command
            || line.split_whitespace().any(|field| field.starts_with(SHELL_COMMENT) && field.split_once('.').is_some_and(|(_, id)| id == client_id))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ov-980: a device is named by its forced command or its plain line's
    /// comment, edited options and all; a longer id, a label, or a commented
    /// line names nobody.
    #[test]
    fn a_line_names_its_client_however_it_was_edited() {
        let key = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB";
        let a = format!(r#"environment="HOME=/h",restrict,command="~/.local/bin/farcoolerd --stdio --scope control --client phone-7" {key} Phone"#);
        let b = format!(r#"environment="HOME=/h" {key} farcooler-shell-Mac-abcd1234.mac-1"#);
        let text = format!("# farcooler begin\n{a}\n{b}\n# --client gone-1\n# farcooler end\n");
        assert!(names_client(&text, "phone-7"), "a forced command");
        assert!(names_client(&text, "mac-1"), "a plain line's comment");
        for nobody in ["phone", "phone-77", "mac", "Mac-abcd1234", "gone-1", "made-up-1", ""] {
            assert!(!names_client(&text, nobody), "{nobody:?}");
        }
    }
}
