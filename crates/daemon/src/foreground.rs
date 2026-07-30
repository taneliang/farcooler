//! What is actually running in each terminal, arguments included.
//!
//! `pane_current_command` is a process NAME, which is the wrong grain for a label
//! people read: `pnpm dev` shows as `node`, `cargo build` as `cargo`, and every
//! idle shell as `zsh`. What distinguishes one pane from another is usually the
//! arguments — `pnpm dev` from `pnpm test`, `cargo build` from `cargo test`.
//!
//! So the foreground process group of each pane's tty is read from `ps`, which
//! has the argv. One call for the whole host per sample, not one per pane: this
//! sits on the watcher's loop, and a fleet of thirty panes must not mean thirty
//! processes a second.

use std::collections::HashMap;

/// Foreground command lines, keyed by tty name (`ttys162`).
pub type Foreground = HashMap<String, String>;

/// Read every tty's foreground command.
///
/// `stat` carries `+` for a process in its terminal's foreground group, which is
/// exactly "the thing you are looking at" — the shell itself is `Ss` and gets
/// skipped, so an idle pane reports nothing and keeps whatever tmux called it.
pub async fn read() -> Foreground {
    let out = tokio::process::Command::new("ps")
        .args(["-axo", "tty=,stat=,args="])
        .stdin(std::process::Stdio::null())
        .output()
        .await;
    let Ok(out) = out else { return Foreground::new() };

    let mut found = Foreground::new();
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        let Some((tty, rest)) = line.trim_start().split_once(char::is_whitespace) else {
            continue;
        };
        let rest = rest.trim_start();
        let Some((stat, args)) = rest.split_once(char::is_whitespace) else { continue };
        if !stat.contains('+') || tty == "??" {
            continue;
        }
        let args = args.trim();
        if args.is_empty() {
            continue;
        }
        // First wins. `ps` lists a foreground pipeline's members in order, and the
        // first is the one that was typed — `rg foo | less` should read as `rg`.
        found.entry(tty.to_string()).or_insert_with(|| summarise(args));
    }
    found
}

/// A command line short enough to be a label.
///
/// The program plus one argument. Two is where a pane header stops being
/// readable, and the first argument is nearly always the distinguishing part:
/// `pnpm dev` against `pnpm test`, `cargo build` against `cargo test`.
fn summarise(args: &str) -> String {
    let mut parts = args.split_whitespace();
    let Some(program) = parts.next() else { return String::new() };
    // A path tells you where a binary lives, which is not what the pane is doing.
    let program = program.rsplit('/').next().unwrap_or(program);

    match parts.next() {
        // A flag is rarely the point — `npm --silent run dev` should not read as
        // `npm --silent` — but a subcommand almost always is.
        Some(arg) if !arg.starts_with('-') => {
            let arg = arg.rsplit('/').next().unwrap_or(arg);
            let joined = format!("{program} {arg}");
            if joined.chars().count() <= 24 { joined } else { program.to_string() }
        }
        _ => program.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_subcommand_survives_because_it_is_the_distinguishing_part() {
        assert_eq!(summarise("pnpm dev"), "pnpm dev");
        assert_eq!(summarise("cargo build --release"), "cargo build");
        assert_eq!(summarise("/opt/homebrew/bin/rg pattern"), "rg pattern");
    }

    #[test]
    fn a_flag_does_not() {
        assert_eq!(summarise("npm --silent run dev"), "npm");
        assert_eq!(summarise("tail -f log"), "tail");
    }

    #[test]
    fn a_long_argument_is_dropped_rather_than_truncated() {
        // Half a path is worse than none: it looks like a name and is not one.
        assert_eq!(summarise("vim src/some/deeply/nested/module.rs"), "vim module.rs");
        assert_eq!(
            summarise("python a_very_long_script_name_indeed_here.py"),
            "python"
        );
    }

    #[test]
    fn nothing_in_means_nothing_out() {
        assert_eq!(summarise(""), "");
    }
}
