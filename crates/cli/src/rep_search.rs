//! `farcooler search '<pattern>' [--path <dir>]` (ov-555): a rep looks for
//! where something is in its checkout.
//!
//! The runner searches (`farcooler_daemon::repo_grep`), over the files git
//! tracks and never a `.env*` file, in the checkout of the pane this runs in.
//! The CLI sends the pattern and the path and prints the lines. It never
//! names a checkout itself.

use clap::Args;
use farcooler_protocol::capability;
use farcooler_protocol::v1::{self as pb, request, result};
use farcooler_transport::ClientError;

use crate::tasks::{self, DispatchLink, Refused};
use crate::{Fallible, connect_to, expect_value, req, with};

const NEEDS_UPDATE: &str = "This runner needs an update to search.";
const CUT: &str = "The results were cut short. Narrow the search with --path.";
const NOTHING: &str = "No matches.";

/// `farcooler search`'s arguments.
#[derive(Debug, Clone, Args)]
pub(crate) struct SearchArgs {
    /// An extended regular expression, matched line by line. One that starts
    /// with a dash can't be given: start it with `[-]`.
    pattern: String,
    /// Search only under this directory (or in this file), relative to your
    /// checkout.
    #[arg(long)]
    path: Option<String>,
}

/// `farcooler search`, on a connected runner.
pub(crate) async fn search(runner: Option<&str>, args: SearchArgs, json: bool) -> Fallible {
    let mut link = connect_to(runner).await?;
    println!("{}", run(&mut link, args, json).await?);
    Ok(())
}

/// The search on `link`, answering what to print.
pub(crate) async fn run<L: DispatchLink>(
    link: &mut L,
    args: SearchArgs,
    json: bool,
) -> Result<String, Box<dyn std::error::Error>> {
    if !link.capabilities().iter().any(|c| c == capability::REP_SEARCH) {
        return Err(Box::new(Refused::new(NEEDS_UPDATE.to_string(), Some(pb::ErrorCode::CapabilityUnsupported as i32))));
    }
    let mut r = with(
        req("worktree.grep"),
        request::Payload::WorktreeGrep(pb::WorktreeGrep { pattern: args.pattern, path: args.path.unwrap_or_default() }),
    );
    r.required_capabilities.push(capability::REP_SEARCH.to_string());
    let answer = link.call(r).await.map_err(refused)?;
    let result::Value::WorktreeGrepResult(found) = expect_value(answer.value)? else {
        return Err(crate::daemon_link::UNREADABLE.into());
    };
    Ok(render(&found, json))
}

/// The lines, then a word when the runner cut them; or why there are none.
fn render(found: &pb::WorktreeGrepResult, json: bool) -> String {
    if json {
        return serde_json::json!({ "text": found.text, "cut": found.cut }).to_string();
    }
    let mut out = found.text.trim_end().to_string();
    if out.is_empty() {
        return NOTHING.to_string();
    }
    if found.cut {
        out.push('\n');
        out.push_str(CUT);
    }
    out
}

/// A refusal in this command's own words; any other in the board's.
fn refused(err: ClientError) -> Box<dyn std::error::Error> {
    if let ClientError::Daemon { code, what, .. } = &err {
        let said = match what.as_str() {
            "pattern" => Some("Give a pattern of at most 1,000 characters, on one line."),
            "path" => Some("Search a directory or file inside your checkout, never .git or a .env file."),
            "rep_only" => Some("Search works from a rep's pane."),
            _ => None,
        };
        if let Some(said) = said {
            return Box::new(Refused::naming(said.to_string(), *code, what.clone()));
        }
    }
    tasks::refused(err, "The runner couldn't search.")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(text: &str, cut: bool) -> pb::WorktreeGrepResult {
        pb::WorktreeGrepResult { text: text.into(), cut }
    }

    #[test]
    fn the_lines_are_printed_and_a_cut_says_so() {
        assert_eq!(render(&found("a.rs:3:fn x()\n", false), false), "a.rs:3:fn x()");
        assert_eq!(render(&found("a.rs:3:fn x()\n", true), false), format!("a.rs:3:fn x()\n{CUT}"));
        assert_eq!(render(&found("", false), false), NOTHING);
        assert_eq!(render(&found("a:1:b\n", true), true), r#"{"cut":true,"text":"a:1:b\n"}"#);
    }

    #[test]
    fn a_refusal_is_said_in_plain_words() {
        for what in ["pattern", "path", "rep_only"] {
            let err = ClientError::Daemon { code: 1, retryable: false, what: what.into(), message: String::new() };
            let said = refused(err).to_string();
            assert!(said.ends_with('.') && !said.contains('_'), "{said}");
        }
    }
}
