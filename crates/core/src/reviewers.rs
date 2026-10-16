//! Whom to ask to review a pull request (ov-580, ov-318 rank 5): the pure half,
//! shared by `farcooler plan lane pr-body` and the runner's plan. Nothing here
//! reads a file or runs git; the callers hand over text.
//!
//! **Three sources, in order, and the author never.** The people CODEOWNERS
//! names for the files the change touches, then whoever's commits last touched
//! those files, then a reviewer map: a file of the same shape as CODEOWNERS,
//! kept at `.farcooler/reviewers` in the repository, for a team that wants
//! its asks written down apart from what GitHub enforces. A source is used to
//! fill up to `MAX` before the next is asked, so a repository whose
//! CODEOWNERS answers never hears from history.
//!
//! **CODEOWNERS** is read the way GitHub reads it, as far as a suggestion
//! needs: `#` comments, a pattern then its owners (`@login`, `@org/team`; an
//! email address owns nothing a login can be asked of), the last matching rule
//! winning, a rule with no owners clearing the file's owners, `*` and `?`
//! within a path segment, `**` across segments, a leading `/` anchoring to the
//! root, a trailing `/` meaning a directory, a pattern with no slash matching
//! at any depth, and (unlike gitignore) a last segment with a wildcard naming
//! only files: `docs/*` owns `docs/a.md` and not `docs/build/a.md`. Not read: `[...]` classes, `!` negation (GitHub
//! rejects it too) and escapes. Ranked by how many touched files each owns.
//!
//! **History** knows people by commit email, and a commit email names a
//! GitHub login only when it is GitHub's own noreply form
//! (`12345+alice@users.noreply.github.com`). Any other address (a work email) is
//! left out rather than guessed at: a wrong login would ask a stranger. A
//! bot's address (`dependabot[bot]`) is never a login here.
//!
//! **The author is out**, whatever the source says, compared without regard to
//! case, and so is anyone who wrote the change's commits: an owner who opens their own PR is never asked to review it, and the
//! next source speaks instead.

/// How many people a suggestion names.
pub const MAX: usize = 3;

/// Where a suggestion came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Codeowners,
    History,
    Map,
}

impl Source {
    pub fn words(self) -> &'static str {
        match self {
            Source::Codeowners => "CODEOWNERS",
            Source::History => "recent commits",
            Source::Map => "the reviewer map",
        }
    }
}

/// One person (a login, or `org/team`) and why they are asked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suggestion {
    pub reviewer: String,
    pub source: Source,
}

/// Everything a suggestion is drawn from.
#[derive(Debug, Clone, Copy, Default)]
pub struct Evidence<'a> {
    /// The pull request's author and anyone else who wrote it: GitHub logins.
    /// Never suggested.
    pub authors: &'a [String],
    /// The paths the change touches, relative to the repository's root.
    pub paths: &'a [String],
    /// The text of the repository's CODEOWNERS, when it has one.
    pub codeowners: Option<&'a str>,
    /// Commit counts by GitHub login over the touched files (`History::of`).
    pub history: &'a [(String, u32)],
    /// The text of `.farcooler/reviewers`, when the repository has one.
    pub map: Option<&'a str>,
}

/// Up to `MAX` people to ask, best first, none of them the author.
pub fn suggest(e: &Evidence) -> Vec<Suggestion> {
    let mut out: Vec<Suggestion> = Vec::new();
    let mut take = |people: Vec<String>, source: Source| {
        for reviewer in people {
            let new = !e.authors.iter().any(|a| a.trim().eq_ignore_ascii_case(&reviewer))
                && !out.iter().any(|s| s.reviewer.eq_ignore_ascii_case(&reviewer));
            if out.len() < MAX && new {
                out.push(Suggestion { reviewer, source });
            }
        }
    };
    if let Some(text) = e.codeowners {
        take(owners_by_files(text, e.paths), Source::Codeowners);
    }
    take(by_commits(e.history), Source::History);
    if let Some(text) = e.map {
        take(owners_by_files(text, e.paths), Source::Map);
    }
    out
}

/// Who `rules` (CODEOWNERS text) names for `paths`, the owner of the most
/// files first, ties by name.
fn owners_by_files(rules: &str, paths: &[String]) -> Vec<String> {
    let rules = parse(rules);
    let mut count: Vec<(String, usize)> = Vec::new();
    for path in paths {
        for owner in owners_of(&rules, path) {
            match count.iter_mut().find(|(o, _)| o.eq_ignore_ascii_case(owner)) {
                Some((_, n)) => *n += 1,
                None => count.push((owner.to_string(), 1)),
            }
        }
    }
    count.sort_by(|(a, x), (b, y)| y.cmp(x).then_with(|| a.to_lowercase().cmp(&b.to_lowercase())));
    count.into_iter().map(|(o, _)| o).collect()
}

fn by_commits(history: &[(String, u32)]) -> Vec<String> {
    let mut all: Vec<&(String, u32)> = history.iter().collect();
    all.sort_by(|(a, x), (b, y)| y.cmp(x).then_with(|| a.to_lowercase().cmp(&b.to_lowercase())));
    all.into_iter().map(|(login, _)| login.clone()).collect()
}

// ---- CODEOWNERS -----------------------------------------------------------

/// One rule: a pattern and the owners it names (none clears them).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rule {
    pattern: String,
    owners: Vec<String>,
}

/// The rules of a CODEOWNERS file, in file order.
pub fn parse(text: &str) -> Vec<Rule> {
    text.lines()
        .filter_map(|line| {
            let line = line.split(" #").next().unwrap_or(line);
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                return None;
            }
            let mut words = line.split_whitespace();
            let pattern = words.next()?.to_string();
            let owners = words.filter_map(owner).collect();
            Some(Rule { pattern, owners })
        })
        .collect()
}

/// `@alice` is `alice`, `@org/team` is `org/team`; an email, or anything that
/// is not a GitHub name, is nobody.
fn owner(word: &str) -> Option<String> {
    let name = word.strip_prefix('@')?;
    let (org, team) = match name.split_once('/') {
        Some((org, team)) => (org, Some(team)),
        None => (name, None),
    };
    (login_ok(org) && team.is_none_or(login_ok)).then(|| name.to_string())
}

/// A GitHub login or team slug: letters, digits and hyphens (and underscores
/// in a team's), at most 39 long.
pub fn login_ok(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 39
        && !name.starts_with('-')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Who owns `path`: the last rule that matches it.
fn owners_of<'a>(rules: &'a [Rule], path: &str) -> &'a [String] {
    rules.iter().rev().find(|r| matches(&r.pattern, path)).map_or(&[], |r| r.owners.as_slice())
}

/// Whether CODEOWNERS pattern `pattern` covers `path` (see the module note).
pub fn matches(pattern: &str, path: &str) -> bool {
    let dir_only = pattern.ends_with('/');
    let p = pattern.trim_matches('/');
    if p.is_empty() {
        return false;
    }
    let anchored = pattern.starts_with('/') || p.contains('/');
    let mut pat: Vec<&str> = p.split('/').collect();
    if !anchored {
        pat.insert(0, "**");
    }
    let path: Vec<&str> = path.trim_start_matches('/').split('/').collect();
    // The pattern covers the path itself, or a directory above it, but a last
    // segment with a wildcard only the files it names: `docs/*` owns
    // `docs/a.md` and not `docs/build/a.md`, as GitHub reads it (unlike gitignore).
    let wild_last = !dir_only && pat.last().is_some_and(|last| last.contains(['*', '?']));
    (1..=path.len()).any(|k| (k == path.len() || !wild_last) && (k < path.len() || !dir_only) && segments(&pat, &path[..k]))
}

fn segments(pat: &[&str], path: &[&str]) -> bool {
    match pat.split_first() {
        None => path.is_empty(),
        Some((&"**", rest)) => (0..=path.len()).any(|skip| segments(rest, &path[skip..])),
        Some((one, rest)) => path.split_first().is_some_and(|(seg, tail)| glob(one, seg) && segments(rest, tail)),
    }
}

/// `*` (any run, within a segment) and `?` (one character).
fn glob(pat: &str, text: &str) -> bool {
    fn go(p: &[char], t: &[char]) -> bool {
        match p.split_first() {
            None => t.is_empty(),
            Some(('*', rest)) => (0..=t.len()).any(|n| go(rest, &t[n..])),
            Some(('?', rest)) => t.split_first().is_some_and(|(_, tail)| go(rest, tail)),
            Some((c, rest)) => t.split_first().is_some_and(|(x, tail)| x == c && go(rest, tail)),
        }
    }
    go(&pat.chars().collect::<Vec<_>>(), &text.chars().collect::<Vec<_>>())
}

// ---- history --------------------------------------------------------------

/// The GitHub login a commit email names, when it is GitHub's noreply form.
pub fn login_from_email(email: &str) -> Option<String> {
    let local = email.trim().strip_suffix("@users.noreply.github.com")?;
    let login = local.rsplit_once('+').map_or(local, |(_, login)| login);
    login_ok(login).then(|| login.to_string())
}

/// Commit counts by login, from the author emails of the commits that last
/// touched the files: one email per commit. Emails that name no login are
/// left out.
pub fn history_of<'a>(emails: impl IntoIterator<Item = &'a str>) -> Vec<(String, u32)> {
    let mut out: Vec<(String, u32)> = Vec::new();
    for login in emails.into_iter().filter_map(login_from_email) {
        match out.iter_mut().find(|(l, _)| l.eq_ignore_ascii_case(&login)) {
            Some((_, n)) => *n += 1,
            None => out.push((login, 1)),
        }
    }
    out
}

/// "Suggested reviewers: `@alice` (CODEOWNERS), `@bob` (recent commits)" for a
/// pull request's description, or `None` when nobody is. In code spans, so the
/// description pings nobody: asking is the author's call, or a nudge's. Every
/// name has passed `login_ok`, so none can end the span.
pub fn line(suggestions: &[Suggestion]) -> Option<String> {
    if suggestions.is_empty() {
        return None;
    }
    let names: Vec<String> = suggestions.iter().map(|s| format!("`@{}` ({})", s.reviewer, s.source.words())).collect();
    Some(format!("Suggested reviewers: {}", names.join(", ")))
}

/// The same for a terminal: "alice (CODEOWNERS), bob (recent commits)".
pub fn plain(suggestions: &[Suggestion]) -> String {
    suggestions.iter().map(|s| format!("{} ({})", s.reviewer, s.source.words())).collect::<Vec<_>>().join(", ")
}

#[cfg(test)]
#[path = "reviewers_tests.rs"]
mod tests;
