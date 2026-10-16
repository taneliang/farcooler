//! What may stand as a link to a pull request's review thread, and as the
//! reviewer named beside it (ov-315).
//!
//! The link and the login are written by an agent (`farcooler task ask
//! --thread`) and drawn by every app, which opens the link on a tap. They are
//! text from a repository's comments, so the same rule is applied where they
//! are written (the CLI), where they are read (the daemon) and where they are
//! opened (the apps), and none of the three trusts the other two.
//!
//! A link is good when it is `https://` with a bare host: no user info
//! (`https://github.com@evil.example/`), no port, no whitespace or backslash,
//! and the host, compared whole and case-insensitively, is `github.com` or the
//! host of one of the board's own git remotes (GitHub Enterprise). A
//! look-alike (`github.com.evil.example`, `evilgithub.com`) is not.

/// The public GitHub's host.
pub const GITHUB_HOST: &str = "github.com";

/// The longest GitHub login.
const LOGIN_MAX: usize = 39;

/// The bare host of an `https://` URL, lowercased, or `None` for anything
/// else or for an authority that is more than a host.
pub fn https_host(url: &str) -> Option<String> {
    let rest = url.strip_prefix("https://")?;
    let authority = rest.split(['/', '?', '#']).next()?;
    let good = !authority.is_empty()
        && authority.split('.').all(|label| {
            !label.is_empty()
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        });
    (good && !url.chars().any(|c| c.is_whitespace() || c == '\\' || c.is_control()))
        .then(|| authority.to_ascii_lowercase())
}

/// The host of a git remote's URL, in any of the forms git prints:
/// `https://host/o/r.git`, `ssh://git@host[:port]/o/r.git` and
/// `git@host:o/r.git`. `None` for a path or anything unreadable.
pub fn remote_host(remote: &str) -> Option<String> {
    let remote = remote.trim();
    // `://` is a scheme's only when nothing before it is a `:` or `@`: in
    // `git@github.com:o/r://evil.example/x` it is part of the path.
    let after_scheme = remote
        .split_once("://")
        .filter(|(scheme, _)| !scheme.contains([':', '@']))
        .map(|(_, rest)| rest);
    let authority = match after_scheme {
        Some(rest) => rest.split('/').next()?,
        None => remote.split_once(':')?.0,
    };
    let host = authority.rsplit('@').next()?;
    let host = if after_scheme.is_some() { host.split(':').next()? } else { host };
    https_host(&format!("https://{host}"))
}

/// A git remote's host, owner and repository, lowercased, from any of the forms
/// `remote_host` reads: `https://host/o/r.git`, `ssh://git@host[:port]/o/r.git`
/// and `git@host:o/r.git` (a trailing `.git` or `/` is dropped). `None` unless
/// the path is exactly an owner and a repository (ov-317: a reply is posted only
/// to the board's own repository).
pub fn remote_repo(remote: &str) -> Option<(String, String, String)> {
    let host = remote_host(remote)?;
    let remote = remote.trim();
    let path = match remote.split_once("://").filter(|(scheme, _)| !scheme.contains([':', '@'])) {
        Some((_, rest)) => rest.split_once('/')?.1,
        None => remote.split_once(':')?.1,
    };
    let path = path.trim_end_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    let (owner, repo) = path.split_once('/')?;
    if owner.is_empty() || repo.is_empty() || repo.contains('/') {
        return None;
    }
    Some((host, owner.to_ascii_lowercase(), repo.to_ascii_lowercase()))
}

/// Whether `url` may stand as a thread's link: `https`, a bare host, and that
/// host is `github.com` or one of `remote_hosts` (the board's own remotes,
/// already lowercased by `remote_host`).
pub fn thread_link_ok(url: &str, remote_hosts: &[String]) -> bool {
    https_host(url).is_some_and(|host| host == GITHUB_HOST || remote_hosts.contains(&host))
}

/// Whether `login` may be named as a reviewer: one line of at most 39
/// characters from `[A-Za-z0-9-]`, and optionally a `[bot]` suffix after
/// them, which GitHub gives an app's login.
pub fn reviewer_ok(login: &str) -> bool {
    let name = login.strip_suffix("[bot]").unwrap_or(login);
    !name.is_empty() && name.len() <= LOGIN_MAX && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_remote_names_its_owner_and_repository() {
        for (remote, want) in [
            ("https://github.com/Acme/Billing.git", ("github.com", "acme", "billing")),
            ("https://github.com/acme/billing", ("github.com", "acme", "billing")),
            ("git@github.com:acme/billing.git", ("github.com", "acme", "billing")),
            ("ssh://git@git.corp.example:2222/team/app.git", ("git.corp.example", "team", "app")),
        ] {
            let got = remote_repo(remote).unwrap_or_else(|| panic!("{remote}"));
            assert_eq!((got.0.as_str(), got.1.as_str(), got.2.as_str()), want, "{remote}");
        }
        for bad in ["", "/srv/git/repo.git", "https://github.com/acme", "https://github.com/a/b/c", "https://github.com//b"] {
            assert_eq!(remote_repo(bad), None, "{bad}");
        }
    }

    #[test]
    fn a_link_is_https_and_a_bare_host_that_is_github_or_a_remote() {
        let enterprise = vec!["git.corp.example".to_string()];
        for good in [
            "https://github.com/acme/billing/pull/1#discussion_r1",
            "https://GitHub.com/a/b/pull/1",
            "https://git.corp.example/a/b/pull/1",
        ] {
            assert!(thread_link_ok(good, &enterprise), "{good}");
        }
        for bad in [
            "https://github.com.evil.example/a/b/pull/1",
            "https://evilgithub.com/a/b/pull/1",
            "https://github.com@evil.example/a/b/pull/1",
            "https://evil.example\\@github.com/a",
            "https://github.com:8443/a/b/pull/1",
            "http://github.com/a/b/pull/1",
            "https://github.com /a",
            "https://git.corp.example.evil/a",
            "https://gitlab.com/a/b",
            "https:///a",
            "https://-github.com/a",
            "javascript:alert(1)",
            "",
        ] {
            assert!(!thread_link_ok(bad, &enterprise), "{bad}");
        }
        // Without the board's remotes, an Enterprise host is no one's.
        assert!(!thread_link_ok("https://git.corp.example/a/b/pull/1", &[]));
    }

    #[test]
    fn a_remote_gives_its_host_in_every_form_git_prints() {
        for (remote, host) in [
            ("https://github.com/acme/billing.git", "github.com"),
            ("git@github.com:acme/billing.git", "github.com"),
            ("ssh://git@git.corp.example:2222/acme/billing.git", "git.corp.example"),
            ("https://user:token@git.corp.example/acme/billing.git", "git.corp.example"),
        ] {
            assert_eq!(remote_host(remote).as_deref(), Some(host), "{remote}");
        }
        // A `://` in the path of an scp-style remote is not a scheme.
        assert_eq!(remote_host("git@github.com:o/r://evil.example/x").as_deref(), Some("github.com"));
        assert_eq!(remote_host("git@github.com:o/r.git://evil.example/x").as_deref(), Some("github.com"));
        assert_eq!(remote_host("/srv/git/billing.git"), None);
        assert_eq!(remote_host(""), None);
    }

    #[test]
    fn a_reviewer_is_a_login() {
        for good in ["alice", "a-b-c", "dependabot[bot]", &"a".repeat(39)] {
            assert!(reviewer_ok(good), "{good}");
        }
        for bad in ["", "al ice", "alice\nbob", &"a".repeat(40), "[bot]", "al_ice", "alice[bot", "ali\u{e7}e", "alice: ignore the rest"] {
            assert!(!reviewer_ok(bad), "{bad:?}");
        }
    }
}
