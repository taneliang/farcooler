//! The rep gate's table: what a rep may call, and each class of escape it
//! refuses, by class.

use std::collections::HashMap;
use std::path::Component;

use serde_json::json;

use super::*;

const CLI: &str = "/opt/fc/farcooler";

/// A file system that resolves the way `canonicalize` does: component by
/// component, following a link wherever it sits (a parent directory
/// included) before the next `..` is applied. The checkout is `/w/repo`.
/// In it, `out` links to a file outside, `docs` is a directory linked
/// outside, and `up` links to a sibling directory, so `up/..` leaves.
fn resolve(path: &Path) -> Option<PathBuf> {
    let links: HashMap<&str, &str> = [("/w/repo/out", "/etc/passwd"), ("/w/repo/docs", "/etc"), ("/w/repo/up", "/w/other/inner")].into();
    let real = [
        "/w/repo", "/w/repo/README.md", "/w/repo/src", "/w/repo/.git", "/w/repo/.git/config", "/w/repo/.env",
        "/w/repo/.env.local", "/w/repo/src/.envrc", "/etc", "/etc/passwd", "/home/u/.ssh/id", "/w/other",
        "/w/other/inner", "/w/other/secret",
    ];
    let mut at = PathBuf::from("/");
    for part in path.components() {
        match part {
            Component::RootDir | Component::CurDir => {}
            Component::ParentDir => {
                at.pop();
            }
            Component::Normal(name) => {
                at.push(name);
                if let Some(to) = links.get(at.to_str()?) {
                    at = PathBuf::from(to);
                }
            }
            Component::Prefix(_) => return None,
        }
    }
    real.contains(&at.to_str()?).then_some(at)
}

fn verdict(tool: &str, input: serde_json::Value) -> Verdict {
    let gate = Gate { checkout: Path::new("/w/repo"), cli: CLI };
    decide(&gate, tool, &input, &resolve)
}

fn bash(command: &str) -> Verdict {
    verdict("Bash", json!({ "command": command }))
}

/// What a rep does: read the board, message, comment, read and search its
/// own checkout, and load its skill.
#[test]
fn a_rep_reads_the_board_messages_comments_and_reads_its_checkout() {
    for command in [
        "/opt/fc/farcooler plan",
        "'/opt/fc/farcooler' plan --json",
        "/opt/fc/farcooler task list",
        "/opt/fc/farcooler task show ov-12",
        "/opt/fc/farcooler task show ov-12 --json",
        r#"'/opt/fc/farcooler' message orchestrator "Alice asks: dark mode & a toggle (by Friday)!""#,
        "/opt/fc/farcooler message orchestrator 'Bob answers: yes; ship it' --task ov-3",
        r#"/opt/fc/farcooler task note ov-3 --kind comment --body "The detail, at length: a > b | c""#,
        r#"/opt/fc/farcooler task note ov-3 --body "Reordered flags" --kind comment"#,
    ] {
        assert_eq!(bash(command), Verdict::Allow, "{command}");
    }
    assert_eq!(verdict("Read", json!({ "file_path": "/w/repo/README.md" })), Verdict::Allow);
    assert_eq!(verdict("Read", json!({ "file_path": "README.md" })), Verdict::Allow, "relative to the checkout");
    assert_eq!(verdict("Read", json!({ "file_path": "/w/repo/src/../README.md" })), Verdict::Allow, "`..` that stays in");
    assert_eq!(verdict("Grep", json!({ "pattern": "dark", "path": "/w/repo/src", "glob": "*.rs" })), Verdict::Allow);
    assert_eq!(verdict("Grep", json!({ "pattern": "dark", "output_mode": "files_with_matches" })), Verdict::Allow);
    assert_eq!(verdict("Grep", json!({ "pattern": "dark" })), Verdict::Allow, "the checkout itself");
    assert_eq!(verdict("Glob", json!({ "pattern": "src/**/*.rs" })), Verdict::Allow);
    assert_eq!(verdict("Skill", json!({ "skill": "farcooler:rep" })), Verdict::Allow);
}

/// Each class of escape, refused: chaining and substitution, redirection,
/// unusual whitespace and braces, another program or one found through
/// `PATH`, a flag that routes elsewhere or speaks as someone else, a write
/// to the board the grammar doesn't name, and a malformed call.
#[test]
fn each_class_of_escape_is_refused() {
    let classes: &[(&str, &[&str])] = &[
        ("chaining", &["/opt/fc/farcooler plan; /opt/fc/farcooler plan", "/opt/fc/farcooler plan && true", "/opt/fc/farcooler plan | cat", "/opt/fc/farcooler plan &", "/opt/fc/farcooler plan\n/opt/fc/farcooler plan"]),
        ("substitution and expansion", &["/opt/fc/farcooler message orchestrator \"$(id)\"", "/opt/fc/farcooler message orchestrator \"`id`\"", "/opt/fc/farcooler message orchestrator $HOME", "/opt/fc/farcooler message orchestrator \"${X}\"", "/opt/fc/farcooler task show ~"]),
        ("escapes, globs and braces", &["/opt/fc/farcooler message orchestrator \\x", "/opt/fc/farcooler task show *", "/opt/fc/farcooler task show ov-?", "/opt/fc/farcooler plan # x", "/opt/fc/farcooler task show {a,b}"]),
        ("unusual whitespace", &["/opt/fc/farcooler\u{a0}plan", "/opt/fc/farcooler\tplan", "/opt/fc/farcooler plan\r", "/opt/fc/farcooler message orchestrator 'a\rb'"]),
        ("redirection", &["/opt/fc/farcooler plan > x", "/opt/fc/farcooler plan >> x", "/opt/fc/farcooler plan < x", "/opt/fc/farcooler plan 2>x"]),
        ("another program", &["git log", "rg dark", "grep -r dark .", "gh pr view 1", "sh -c /opt/fc/farcooler", "env /opt/fc/farcooler plan", "X=1 /opt/fc/farcooler plan", "/tmp/farcooler plan", "./farcooler plan"]),
        ("a program found through PATH", &["farcooler plan", "'farcooler' plan", "farcooler-local plan"]),
        ("routing", &["/opt/fc/farcooler --runner other plan", "/opt/fc/farcooler plan --runner other", "/opt/fc/farcooler --runner=other plan", "/opt/fc/farcooler task list --workspace other", "/opt/fc/farcooler task show ov-1 --repo other", "/opt/fc/farcooler message rep:bob hi", "/opt/fc/farcooler message mac-ux hi"]),
        ("speaking as another", &["/opt/fc/farcooler message orchestrator hi --actor manager", "/opt/fc/farcooler task note ov-1 --kind comment --body x --actor user"]),
        ("other writes", &["/opt/fc/farcooler plan set a", "/opt/fc/farcooler task create x", "/opt/fc/farcooler task dispatch ov-1", "/opt/fc/farcooler worktree create x", "/opt/fc/farcooler task note ov-1 --kind answer --body yes", "/opt/fc/farcooler task note ov-1 --kind comment --body x --extra '{}'", "/opt/fc/farcooler task note ov-1 --kind comment --body x --supersedes 1", "/opt/fc/farcooler rep alice"]),
        ("flag smuggling", &["/opt/fc/farcooler message orchestrator --help", "/opt/fc/farcooler task show --json", "/opt/fc/farcooler message orchestrator ''", "/opt/fc/farcooler task note -x --kind comment --body y"]),
        ("malformed", &["", "   ", "'unclosed", "\"unclosed", "''"]),
    ];
    for (class, commands) in classes {
        for command in *commands {
            assert!(matches!(bash(command), Verdict::Deny(_)), "{class}: {command:?} was allowed");
        }
    }
    assert!(matches!(verdict("Bash", json!({})), Verdict::Deny(_)), "no command");
    // A gate told a CLI by name rather than by path runs nothing.
    let by_name = Gate { checkout: Path::new("/w/repo"), cli: "farcooler" };
    assert!(matches!(decide(&by_name, "Bash", &json!({ "command": "farcooler plan" }), &resolve), Verdict::Deny(_)));
}

/// Paths outside the checkout, by any spelling, are refused: absolute,
/// climbing, through a link to a file or through a linked parent
/// directory, `..` after a link, home, or not there at all; so are `.git`
/// and `.env` files inside it, and a search pattern that names outside.
#[test]
fn paths_outside_the_checkout_are_refused() {
    for path in [
        "/etc/passwd", "/w/repo/../../etc/passwd", "../../etc/passwd", "/w/repo/out", "/w/repo/docs/passwd",
        "/w/repo/up/../secret", "/home/u/.ssh/id", "~/.ssh/id", "/w/repo/missing", "",
    ] {
        assert!(matches!(verdict("Read", json!({ "file_path": path })), Verdict::Deny(_)), "Read {path:?}");
        assert!(matches!(verdict("Grep", json!({ "pattern": "x", "path": path })), Verdict::Deny(_)), "Grep {path:?}");
        assert!(matches!(verdict("Glob", json!({ "pattern": "*", "path": path })), Verdict::Deny(_)), "Glob {path:?}");
    }
    for private in ["/w/repo/.git/config", ".git/config", "/w/repo/.env", ".env.local", "src/.envrc", "/w/repo/.git"] {
        assert!(matches!(verdict("Read", json!({ "file_path": private })), Verdict::Deny(_)), "Read {private:?}");
    }
    for pattern in ["/etc/*", "../*", "src/../../*", "~/*", "{src,/etc}/*", "src/{a,b}.rs", ".git/**", "**/.env*"] {
        assert!(matches!(verdict("Glob", json!({ "pattern": pattern })), Verdict::Deny(_)), "Glob {pattern}");
        assert!(matches!(verdict("Grep", json!({ "pattern": "x", "glob": pattern })), Verdict::Deny(_)), "Grep glob {pattern}");
    }
    assert!(matches!(verdict("Grep", json!({ "pattern": "KEY", "output_mode": "content" })), Verdict::Deny(_)), "lines");
    assert!(matches!(verdict("Read", json!({ "file_path": 7 })), Verdict::Deny(_)), "not a path");
}

/// A path that isn't under the checkout as written is refused without the
/// file system being asked about it, so the gate never waits on one.
#[test]
fn a_path_outside_is_refused_before_it_is_looked_up() {
    let asked = std::cell::RefCell::new(Vec::<PathBuf>::new());
    let watching = |p: &Path| {
        asked.borrow_mut().push(p.to_path_buf());
        resolve(p)
    };
    let gate = Gate { checkout: Path::new("/w/repo"), cli: CLI };
    for path in ["/net/far/away", "/w/repo/../../net/far", "../elsewhere"] {
        asked.borrow_mut().clear();
        assert!(matches!(decide(&gate, "Read", &json!({ "file_path": path }), &watching), Verdict::Deny(_)), "{path}");
        assert!(asked.borrow().iter().all(|p| p == Path::new("/w/repo")), "{path} was looked up: {:?}", asked.borrow());
    }
}

/// Everything else is refused by default: the tools that write, fetch,
/// spawn agents, run commands or reach servers, a skill not the rep's, and
/// a tool nobody has named yet.
#[test]
fn every_other_tool_is_refused() {
    for tool in [
        "Edit", "Write", "MultiEdit", "NotebookEdit", "WebFetch", "WebSearch", "Task", "Agent", "TodoWrite", "SlashCommand",
        "mcp__x__y", "BashOutput", "KillShell", "SomeToolFromNextYear",
    ] {
        assert_eq!(verdict(tool, json!({ "file_path": "/w/repo/README.md" })), Verdict::Deny(NOT_A_MESSENGERS), "{tool}");
    }
    assert!(matches!(verdict("Skill", json!({ "skill": "farcooler:manager" })), Verdict::Deny(_)));
}
