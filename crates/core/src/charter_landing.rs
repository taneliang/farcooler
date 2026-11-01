//! A charter's `## Landing` section (ov-872, slice 5): how a workspace lands
//! its tickets, in a small fixed form the runner reads. In core so the CLI
//! reads it the same way, to say which line didn't.
//!
//! ```text
//! ## Landing
//!
//! - Route: push
//! - Route `services/site/**`: ask
//! - Gate: `scripts/landing-gate.sh`
//! - Base: main
//! - Remote: origin
//! - CI: github
//! ```
//!
//! - **Route** (once): the route a ticket takes when no path rule says
//!   otherwise: `push`, `pull request` or `ask`. Push when it isn't said.
//! - **Route \`glob\`** (any number, in order): the route for the files the
//!   glob matches. `*` is any run of characters within one folder, `**` any
//!   run across folders, `?` one character. A file takes the first rule that
//!   matches it, else the default. A ticket takes its most careful file's
//!   route: ask over pull request over push.
//! - **Gate** (one or more): a command run with `sh -c` at the top of the
//!   landing's worktree; every one must pass, in order. `Gate: none` says
//!   there is none on purpose. A section that can push (including ask after approval) needs one of the two.
//! - **Base** (once): the branch work lands on; `main` when not said.
//! - **Remote** (once): the remote it is pushed to; `origin` when not said.
//! - **CI** (once): `github` waits for GitHub Actions on the base and on the
//!   push, `none` lands once the gate passes. `github` when not said.
//!
//! Every line in the section that isn't blank or an HTML comment is one of
//! those bullets. Anything else refuses the charter on save
//! (`workspace.set_charter`), with the line that didn't read, so a typo never
//! quietly pushes. A charter with no `## Landing` section has said nothing,
//! and the runner's landing job refuses to start (`no_landing_rules`).

use std::fmt;

/// How a ticket gets into the product, as a charter says it. The store's
/// `landings::Route` by another name: core can't name the store.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Route {
    /// Fold onto the base, gate, push, and land on green CI.
    Push,
    /// A pull request per ticket, which a human merges.
    PullRequest,
    /// Needs you: it pushes once you say land it.
    Ask,
}

/// What a charter's `## Landing` section says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LandingRules {
    /// The route of a file no path rule matches.
    pub default_route: Route,
    /// Path rules, first match wins.
    pub rules: Vec<(String, Route)>,
    /// The gate's commands, in order. Empty with `Gate: none`.
    pub gate: Vec<String>,
    pub base: String,
    pub remote: String,
    /// Whether the job waits on GitHub Actions.
    pub ci: bool,
}

/// Why a section didn't read: the line, counted from 1 in the whole charter,
/// and what was wrong with it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SectionError {
    pub line: usize,
    pub why: String,
}

impl fmt::Display for SectionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}: {}", self.line, self.why)
    }
}

fn route_word(raw: &str) -> Option<Route> {
    match raw.trim().to_ascii_lowercase().replace(['_', '-'], " ").as_str() {
        "push" => Some(Route::Push),
        "pull request" | "pull requests" | "pr" => Some(Route::PullRequest),
        "ask" => Some(Route::Ask),
        _ => None,
    }
}

/// How careful a route is: a ticket takes its most careful file's.
fn care(route: Route) -> u8 {
    match route {
        Route::Push => 0,
        Route::PullRequest => 1,
        Route::Ask => 2,
    }
}

/// `value` without one pair of backticks around it.
fn unticked(value: &str) -> &str {
    let v = value.trim();
    v.strip_prefix('`').and_then(|v| v.strip_suffix('`')).unwrap_or(v)
}

/// A branch or remote name a setting can hold.
fn name_ok(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 200
        && !value.starts_with('-')
        && !value.contains("..")
        && !value.contains(|c: char| c.is_whitespace() || c.is_control() || matches!(c, '~' | '^' | ':' | '\\' | '`'))
}

/// The `## Landing` section of `charter`, read. `Ok(None)` when it has none.
pub fn parse(charter: &str) -> Result<Option<LandingRules>, SectionError> {
    let mut lines = charter.lines().enumerate();
    let mut found = false;
    for (_, line) in lines.by_ref() {
        if is_heading(line, 2) && heading_text(line).eq_ignore_ascii_case("landing") {
            found = true;
            break;
        }
    }
    if !found {
        return Ok(None);
    }
    let mut default_route = None;
    let mut rules = Vec::new();
    let mut gate = Vec::new();
    let mut no_gate = false;
    let (mut base, mut remote, mut ci) = (None, None, None);
    let mut in_comment = false;
    for (i, line) in lines {
        let n = i + 1;
        let fail = |why: &str| Err(SectionError { line: n, why: why.to_string() });
        let text = line.trim();
        if is_heading(line, 1) || is_heading(line, 2) {
            break;
        }
        if in_comment {
            in_comment = !text.contains("-->");
            continue;
        }
        if text.is_empty() {
            continue;
        }
        if text.starts_with("<!--") {
            in_comment = !text.contains("-->");
            continue;
        }
        let Some(item) = text.strip_prefix("- ").or_else(|| text.strip_prefix("* ")) else {
            return fail("each line is a bullet: `- Route: push`, `- Route `glob`: ask`, `- Gate: <command>`, `- Base:`, `- Remote:` or `- CI:`");
        };
        // The key ends at the first colon after any backticked glob.
        let split = match item.find('`') {
            Some(open) if item[..open].trim().eq_ignore_ascii_case("route") => {
                let Some(close) = item[open + 1..].find('`').map(|c| open + 1 + c) else {
                    return fail("a path rule's glob is closed with a backtick");
                };
                item[close..].find(':').map(|c| close + c)
            }
            _ => item.find(':'),
        };
        let Some(colon) = split else { return fail("a bullet is `Key: value`") };
        let (key, value) = (item[..colon].trim(), item[colon + 1..].trim());
        let lower = key.to_ascii_lowercase();
        let ticked = lower.strip_prefix("route").map(str::trim).filter(|g| g.len() >= 2 && g.starts_with('`') && g.ends_with('`'));
        if let Some(glob) = ticked {
            let glob = unticked(&key[key.len() - glob.len()..]);
            if glob.is_empty() || glob.starts_with('/') {
                return fail("a path rule's glob is a path from the top of the repository, such as `services/site/**`");
            }
            let Some(route) = route_word(value) else { return fail("a route is push, pull request or ask") };
            rules.push((glob.to_string(), route));
            continue;
        }
        match lower.as_str() {
            "route" => {
                let Some(route) = route_word(value) else { return fail("a route is push, pull request or ask") };
                if default_route.replace(route).is_some() {
                    return fail("say the default route once");
                }
            }
            "gate" => {
                let command = unticked(value);
                if command.is_empty() {
                    return fail("a gate is a command, or `none`");
                }
                if command.eq_ignore_ascii_case("none") {
                    no_gate = true;
                } else {
                    gate.push(command.to_string());
                }
            }
            "base" | "remote" => {
                let name = unticked(value);
                if !name_ok(name) {
                    return fail("that isn't a branch or remote name");
                }
                let slot = if lower == "base" { &mut base } else { &mut remote };
                if slot.replace(name.to_string()).is_some() {
                    return fail("say it once");
                }
            }
            "ci" => {
                let on = match value.to_ascii_lowercase().as_str() {
                    "github" | "github actions" => true,
                    "none" => false,
                    _ => return fail("CI is github or none"),
                };
                if ci.replace(on).is_some() {
                    return fail("say CI once");
                }
            }
            _ => return fail("the keys are Route, Gate, Base, Remote and CI"),
        }
    }
    if no_gate && !gate.is_empty() {
        return Err(SectionError { line: 0, why: "`Gate: none` and a gate command can't both be said".into() });
    }
    let default_route = default_route.unwrap_or(Route::Push);
    let pushes = default_route != Route::PullRequest || rules.iter().any(|(_, r)| *r != Route::PullRequest);
    if pushes && gate.is_empty() && !no_gate {
        return Err(SectionError { line: 0, why: "a section that pushes names its gate: `- Gate: <command>`, or `- Gate: none`".into() });
    }
    Ok(Some(LandingRules {
        default_route,
        rules,
        gate,
        base: base.unwrap_or_else(|| "main".into()),
        remote: remote.unwrap_or_else(|| "origin".into()),
        ci: ci.unwrap_or(true),
    }))
}

fn is_heading(line: &str, level: usize) -> bool {
    let hashes = "#".repeat(level);
    line.strip_prefix(&hashes).is_some_and(|rest| rest.starts_with(' '))
}

fn heading_text(line: &str) -> &str {
    line.trim_start_matches('#').trim()
}

impl LandingRules {
    /// The route of one file.
    pub fn route_of_file(&self, path: &str) -> Route {
        self.rules.iter().find(|(glob, _)| glob_matches(glob, path)).map_or(self.default_route, |(_, r)| *r)
    }

    /// The route of a ticket that changes `paths`, and the rule that chose it
    /// when it wasn't the default: its most careful file's.
    pub fn route_of<'a>(&'a self, paths: &[String]) -> (Route, Option<&'a str>) {
        let mut best = None;
        for path in paths {
            let rule = self.rules.iter().find(|(glob, _)| glob_matches(glob, path));
            let route = rule.map_or(self.default_route, |(_, r)| *r);
            if best.as_ref().is_none_or(|(chosen, _)| care(route) > care(*chosen)) {
                best = Some((route, rule.map(|(g, _)| g.as_str())));
            }
        }
        best.unwrap_or((self.default_route, None))
    }
}

/// Whether `glob` matches the whole of `path`: `**` any run of characters,
/// `*` any run without a `/`, `?` one character but `/`. `dir/**` matches
/// everything under `dir`, and `**/name` a `name` at any depth, the top too.
pub fn glob_matches(glob: &str, path: &str) -> bool {
    fn go(g: &[u8], p: &[u8]) -> bool {
        match g.split_first() {
            None => p.is_empty(),
            Some((b'*', rest)) if rest.first() == Some(&b'*') => {
                let rest = &rest[1..];
                // `**/` also matches no folder at all: `**/a` matches `a`.
                if rest.first() == Some(&b'/') && go(&rest[1..], p) {
                    return true;
                }
                (0..=p.len()).any(|i| go(rest, &p[i..]))
            }
            Some((b'*', rest)) => {
                for i in 0..=p.len() {
                    if go(rest, &p[i..]) {
                        return true;
                    }
                    if p.get(i) == Some(&b'/') {
                        return false;
                    }
                }
                false
            }
            Some((b'?', rest)) => p.first().is_some_and(|c| *c != b'/') && go(rest, &p[1..]),
            Some((c, rest)) => p.first() == Some(c) && go(rest, &p[1..]),
        }
    }
    go(glob.as_bytes(), path.as_bytes())
}

#[cfg(test)]
#[path = "charter_landing_tests.rs"]
mod tests;
