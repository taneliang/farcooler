//! Route and glob matching for workflow steps.

/// How a ticket gets into the product.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Route {
    /// Fold onto the base, gate, push, and land on green CI.
    Push,
    /// A pull request per ticket, which a human merges.
    PullRequest,
    /// Needs you: it pushes once you say land it.
    Ask,
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
