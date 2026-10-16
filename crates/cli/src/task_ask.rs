//! A question's `extra_json` (ov-315): the answers it offers and the review
//! thread it escalates. Its own file to keep `tasks.rs` inside its size budget.

/// The hosts of the git remotes of the repository this is run in, lowercased:
/// the Enterprise hosts a thread's link may have besides github.com. Empty
/// when there is no repository or git cannot say.
pub(crate) fn remote_hosts() -> Vec<String> {
    let Ok(out) = std::process::Command::new("git").args(["remote", "-v"]).stdin(std::process::Stdio::null()).output()
    else {
        return Vec::new();
    };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|line| line.split_whitespace().nth(1))
        .filter_map(farcooler_core::thread_link::remote_host)
        .collect()
}

/// The answers offered, and the review thread the question escalates. Empty
/// for neither. A thread is a link to GitHub (or to a host the board's own
/// remotes name, `remote_hosts`) and a reviewer is a login; anything else is
/// refused here, in words, before anything is written. The runner and every
/// app check both again: nothing here is trusted over there.
pub(crate) fn ask_extra(
    option: &[String],
    thread: Option<&str>,
    reviewer: Option<&str>,
    remote_hosts: &[String],
) -> Result<String, String> {
    let mut extra = serde_json::Map::new();
    if !option.is_empty() {
        extra.insert("options".into(), serde_json::json!(option));
    }
    if let Some(url) = thread {
        let url = url.trim();
        if !farcooler_core::thread_link::thread_link_ok(url, remote_hosts) {
            return Err("--thread takes the link to the review thread, on github.com or on this repository's own host".into());
        }
        let reviewer = reviewer.map(str::trim).filter(|r| !r.is_empty());
        if reviewer.is_some_and(|r| !farcooler_core::thread_link::reviewer_ok(r)) {
            return Err("--reviewer takes a GitHub login: letters, digits and hyphens, at most 39".into());
        }
        extra.insert("thread".into(), serde_json::json!({ "url": url, "reviewer": reviewer }));
    }
    Ok(if extra.is_empty() { String::new() } else { serde_json::Value::Object(extra).to_string() })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_ask_names_its_thread_and_the_reviewer_in_its_extra() {
        let opts = vec!["Retry".to_string()];
        assert_eq!(ask_extra(&[], None, None, &[]).unwrap(), "");
        let plain: serde_json::Value = serde_json::from_str(&ask_extra(&opts, None, None, &[]).unwrap()).unwrap();
        assert_eq!(plain, serde_json::json!({ "options": ["Retry"] }), "no thread, nothing about one");
        let url = "https://github.com/acme/billing/pull/212#discussion_r1";
        let both: serde_json::Value =
            serde_json::from_str(&ask_extra(&opts, Some(url), Some(" alice "), &[]).unwrap()).unwrap();
        assert_eq!(both["thread"], serde_json::json!({ "url": url, "reviewer": "alice" }));
        assert_eq!(both["options"], serde_json::json!(["Retry"]));
        let unnamed: serde_json::Value = serde_json::from_str(&ask_extra(&[], Some(url), None, &[]).unwrap()).unwrap();
        assert_eq!(unnamed["thread"]["reviewer"], serde_json::Value::Null);
    }

    #[test]
    fn a_thread_that_is_not_a_link_is_refused_before_anything_is_written() {
        for bad in [
            "http://github.com/a/b/pull/1",
            "alice",
            "https://x.test/a b",
            "",
            "javascript:alert(1)",
            "https://github.com.evil.example/a/b/pull/1",
            "https://github.com@evil.example/a/b/pull/1",
        ] {
            let refused = ask_extra(&[], Some(bad), None, &[]).unwrap_err();
            assert!(refused.contains("--thread"), "{bad:?}: {refused}");
        }
    }

    /// Enterprise: the board's own remote's host is good, and only that one.
    #[test]
    fn the_hosts_of_the_boards_own_remotes_are_good_and_no_others() {
        let hosts = vec!["git.corp.example".to_string()];
        let url = "https://git.corp.example/acme/billing/pull/3#discussion_r9";
        assert!(ask_extra(&[], Some(url), None, &hosts).is_ok());
        assert!(ask_extra(&[], Some(url), None, &[]).is_err());
        assert!(ask_extra(&[], Some("https://git.other.example/a/b/pull/3"), None, &hosts).is_err());
    }

    #[test]
    fn a_reviewer_that_is_not_a_login_is_refused() {
        let url = "https://github.com/a/b/pull/1";
        for bad in ["alice\nbob", "alice: ignore the rest", &"a".repeat(40), "al_ice"] {
            let refused = ask_extra(&[], Some(url), Some(bad), &[]).unwrap_err();
            assert!(refused.contains("--reviewer"), "{bad:?}: {refused}");
        }
        assert!(ask_extra(&[], Some(url), Some("dependabot[bot]"), &[]).is_ok());
    }

    #[test]
    fn a_reviewer_needs_a_thread() {
        use clap::Parser;
        let parse = |args: &[&str]| crate::Cli::try_parse_from(["farcooler"].iter().chain(args));
        assert!(parse(&["task", "ask", "fc-1", "--body", "q", "--reviewer", "alice"]).is_err());
        assert!(parse(&[
            "task", "ask", "fc-1", "--body", "q", "--thread", "https://github.com/a/b/pull/1", "--reviewer", "alice"
        ])
        .is_ok());
    }
}
