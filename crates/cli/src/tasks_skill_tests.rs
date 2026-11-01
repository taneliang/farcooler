//! The manager skill against the CLI: every `farcooler …` command the skill
//! names parses through the real clap tree, and its `--actor manager` rule
//! covers exactly the commands that take the flag. A child of `tasks.rs`,
//! moved out of its inline tests to keep that file inside its size budget.

use super::tests::refused_value;

/// Split a line the way a POSIX shell would for the simple cases the skill
/// uses: whitespace between words, and single or double quotes around one.
pub(super) fn shell_words(line: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut in_word = false;
    let mut quote: Option<char> = None;
    for c in line.chars() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), c) => word.push(c),
            (None, '\'' | '"') => {
                quote = Some(c);
                in_word = true;
            }
            (None, c) if c.is_whitespace() => {
                if in_word {
                    words.push(std::mem::take(&mut word));
                    in_word = false;
                }
            }
            (None, c) => {
                word.push(c);
                in_word = true;
            }
        }
    }
    assert!(quote.is_none(), "an unclosed quote: {line}");
    if in_word {
        words.push(word);
    }
    words
}

/// Every `farcooler …` line inside a fenced block of the skill, and every
/// inline `` `farcooler …` `` span outside one, with each `<placeholder>`
/// replaced by a word clap will take.
fn commands_the_skill_names() -> Vec<String> {
    use farcooler_daemon::skill_install::{Harness, render};
    let skill = render(Harness::Claude, "farcooler")
        .into_iter()
        .find(|f| f.relative.ends_with("SKILL.md"))
        .expect("claude's copy has a SKILL.md")
        .contents;
    let mut fenced = false;
    let mut found = Vec::new();
    for line in skill.lines() {
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
            continue;
        }
        let line = line.trim();
        let spans: Vec<&str> = if fenced {
            vec![line]
        } else {
            // A span may open on prose ("Read farcooler task show <key>,
            // then do the task."): the command is from the CLI's name to
            // the first comma.
            line.split('`')
                .skip(1)
                .step_by(2)
                .filter_map(|span| span.find("farcooler ").map(|at| &span[at..]))
                .map(|span| span.split(',').next().unwrap_or(span))
                .collect()
        };
        for line in spans.into_iter().filter(|l| l.starts_with("farcooler ")) {
            let mut filled = String::new();
            let mut rest = line;
            while let Some(open) = rest.find('<') {
                let close = rest[open..].find('>').expect("an unclosed placeholder") + open;
                filled.push_str(&rest[..open]);
                filled.push('x');
                rest = &rest[close + 1..];
            }
            filled.push_str(rest);
            found.push(filled.trim_end_matches(['.', ',']).to_string());
        }
    }
    found
}

/// The skill can't name a command the CLI doesn't have. An agent told
/// `task answer` finds out by failing, in front of the owner.
///
/// Reads the skill exactly as the daemon renders it, and parses every
/// `farcooler …` line in a fenced block through the real clap tree.
#[test]
fn every_command_the_manager_skill_names_parses() {
    use clap::Parser;
    // The check can fail: a verb the CLI doesn't have is refused.
    assert!(crate::Cli::try_parse_from(["farcooler", "task", "answer", "fc-1"]).is_err());

    let commands = commands_the_skill_names();
    assert!(commands.len() >= 8, "too few commands to be checking anything: {commands:?}");
    for line in &commands {
        if let Err(e) = crate::Cli::try_parse_from(shell_words(line)) {
            panic!("the manager skill names a command the CLI refuses:\n  {line}\n{e}");
        }
        // clap takes these as plain strings and they are refused only when
        // run, so the literal values the skill writes are checked here too.
        if let Some(e) = refused_value(&shell_words(line)) {
            panic!("the manager skill writes a value the CLI refuses:\n  {line}\n{e}");
        }
    }
    // Dispatch is the one verb that starts an agent, so the skill has to
    // name it, both ways into a worktree.
    let dispatches: Vec<&String> = commands
        .iter()
        .filter(|c| c.starts_with("farcooler task dispatch ") || c.starts_with("farcooler agent dispatch "))
        .collect();
    assert!(dispatches.iter().any(|c| c.contains("--new")), "{commands:?}");
    assert!(dispatches.iter().any(|c| c.contains("--worktree")), "{commands:?}");
    // Splitting a workstream off, and finding the board from the home,
    // are commands too, and are held to the same parse.
    for verb in [
        "farcooler workspace show ",
        "farcooler --json workspace show ",
        "farcooler workspace create ",
        "farcooler task move ",
        "farcooler worktree assign ",
        "farcooler workspace start-orchestrator ",
    ] {
        assert!(commands.iter().any(|c| c.starts_with(verb)), "the skill never names {verb:?}: {commands:?}");
    }
}

/// The skill's `--actor manager` rule names the commands it covers, and
/// they are exactly the ones the skill shows that take the flag. A rule
/// read as "every write" sends `--actor` to `workspace create`, `worktree
/// assign` and `workspace start-orchestrator`, which refuse it, and the
/// orchestrator is stopped partway through a split.
#[test]
fn the_actor_rule_covers_exactly_the_commands_that_take_it() {
    use clap::Parser;
    use farcooler_daemon::skill_install::{Harness, render};
    let skill = render(Harness::Claude, "farcooler")
        .into_iter()
        .find(|f| f.relative.ends_with("SKILL.md"))
        .expect("claude's copy has a SKILL.md")
        .contents;
    let rule = skill
        .split("\n\n")
        .find(|p| p.contains("carries `--actor manager`"))
        .expect("the skill states who carries --actor manager");
    let covered: Vec<&str> =
        rule.split('`').skip(1).step_by(2).filter(|s| !s.starts_with("--") && *s != "farcooler").collect();
    assert!(!covered.is_empty(), "the rule names no command, so it reads as every write: {rule}");

    let commands = commands_the_skill_names();
    let mut takes = 0;
    for line in &commands {
        let with_actor = if line.contains("--actor") { line.clone() } else { format!("{line} --actor manager") };
        let accepts = crate::Cli::try_parse_from(shell_words(&with_actor)).is_ok();
        // A line the skill already gives `--actor` has to take it.
        assert!(accepts || !line.contains("--actor"), "the skill gives --actor to a command that refuses it:\n  {line}");
        let named = covered.iter().any(|verb| line.starts_with(&format!("farcooler {verb} ")));
        assert_eq!(named, accepts, "the rule ({covered:?}) and the CLI disagree about --actor on:\n  {line}");
        takes += usize::from(accepts);
    }
    // A rule and a skill that both named nothing would agree.
    assert!(takes >= 5, "{takes} commands take --actor: {commands:?}");
}
