//! The variables that make claude run on a credential instead of a config
//! directory's login, and the owner's warning that their shell's startup
//! files export one (ov-651).
//!
//! A member's own shell pane drops them before the shell starts, but the
//! owner's rc files run after that and can export them again, and a `claude`
//! typed there would use the owner's account. The daemon asks the login shell
//! which are set (`claude_key_exports`), never what they are set to. Only the
//! names reach this file, the wire and the apps.

/// The three variables, in the order the warning names them.
pub const NAMES: [&str; 3] = ["ANTHROPIC_API_KEY", "CLAUDE_CODE_OAUTH_TOKEN", "ANTHROPIC_AUTH_TOKEN"];

/// The owner's warning for the `names` their shell's startup files set, or
/// `None` when none are. One sentence, the same on every app: the daemon sends
/// names and each client calls this.
pub fn warning(names: &[String]) -> Option<String> {
    let listed = match names {
        [] => return None,
        [one] => one.clone(),
        [init @ .., last] => format!("{} and {last}", init.join(", ")),
    };
    Some(format!("Your shell’s startup files set {listed}, so a member typing claude in their own shell would use your account."))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(of: &[&str]) -> Vec<String> {
        of.iter().map(|n| n.to_string()).collect()
    }

    #[test]
    fn nothing_set_is_no_warning() {
        assert_eq!(warning(&[]), None);
    }

    #[test]
    fn one_name_is_named() {
        assert_eq!(
            warning(&names(&["ANTHROPIC_API_KEY"])).as_deref(),
            Some("Your shell’s startup files set ANTHROPIC_API_KEY, so a member typing claude in their own shell would use your account.")
        );
    }

    #[test]
    fn several_names_read_as_a_list() {
        let two = warning(&names(&["ANTHROPIC_API_KEY", "ANTHROPIC_AUTH_TOKEN"])).unwrap();
        assert!(two.contains("set ANTHROPIC_API_KEY and ANTHROPIC_AUTH_TOKEN,"), "{two}");
        let three = warning(&names(&NAMES)).unwrap();
        assert!(three.contains("ANTHROPIC_API_KEY, CLAUDE_CODE_OAUTH_TOKEN and ANTHROPIC_AUTH_TOKEN,"), "{three}");
    }
}
