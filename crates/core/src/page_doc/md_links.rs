//! The links inside a `text` block's Markdown.
//!
//! The apps draw `md` with their own Markdown, which opens `http` and `mailto`
//! links and draws a link's label without where it goes. A page may hold only
//! `https` links, and the domain the owner reads must be the domain that opens:
//! a link whose words name one domain and whose target is another is the
//! spoof this refuses. Every way Markdown can write a link is read: inline
//! `[label](target)` (and an image's, which has the same parentheses),
//! reference definitions `[label]: target`, and `<autolinks>`.

use super::url_host;

/// Why `md` can't be drawn, or `Ok` when every link in it is a plain `https`
/// link that says where it goes.
pub(super) fn check(md: &str) -> Result<(), String> {
    for (label, target) in links(md) {
        check_target(&target)?;
        if let Some(label) = label {
            check_label(&label, &target)?;
        }
    }
    Ok(())
}

/// A target cut short and with control characters escaped, to sit in a sentence.
fn shown(text: &str) -> String {
    let head: String = text.chars().take(40).flat_map(char::escape_debug).collect();
    if text.chars().count() > 40 { format!("{head}...") } else { head }
}

fn check_target(target: &str) -> Result<(), String> {
    if !target.starts_with("https://") {
        let scheme = target.split_once(':').map(|(s, _)| s).filter(|s| !s.is_empty() && s.len() < 20);
        return Err(match scheme {
            Some(scheme) => format!(
                "a link in text has to be https, and this one is {} ({}). Links go in a links block, or in text as https.",
                shown(scheme),
                shown(target)
            ),
            None => format!("a link in text has to start with https:// ({}).", shown(target)),
        });
    }
    if url_host(target).is_none() {
        return Err(format!("the link {} has a domain a page can't show. Write the domain in plain letters, with no user name before it.", shown(target)));
    }
    Ok(())
}

/// A label's words must not name a domain other than the link's.
fn check_label(label: &str, target: &str) -> Result<(), String> {
    let goes_to = url_host(target).unwrap_or("").to_ascii_lowercase();
    let goes_to = goes_to.strip_prefix("www.").unwrap_or(&goes_to);
    for word in label.split_whitespace() {
        let word = word.trim_matches(|c: char| ".,;:!?()[]\"'`*_".contains(c));
        let word = word.strip_prefix("https://").or_else(|| word.strip_prefix("http://")).unwrap_or(word);
        let host = word.split(['/', '?', '#']).next().unwrap_or("");
        if !looks_like_a_domain(host) {
            continue;
        }
        let named = host.to_ascii_lowercase();
        if named.strip_prefix("www.").unwrap_or(&named) != goes_to {
            return Err(format!(
                "this link's words name {} but it goes to {}. Name the domain it goes to, or use words that aren't a domain.",
                shown(host),
                shown(goes_to)
            ));
        }
    }
    Ok(())
}

fn looks_like_a_domain(word: &str) -> bool {
    let labels: Vec<&str> = word.split('.').collect();
    labels.len() >= 2
        && labels.iter().all(|l| !l.is_empty() && l.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'))
        && labels.last().is_some_and(|tld| tld.len() >= 2 && tld.bytes().all(|b| b.is_ascii_alphabetic()))
}

/// Every link in `md`: its label when it has one, and its target.
fn links(md: &str) -> Vec<(Option<String>, String)> {
    let b = md.as_bytes();
    let mut out = Vec::new();
    for (i, &c) in b.iter().enumerate() {
        match c {
            // `[label](target "title")`, and `![alt](target)`.
            b']' if b.get(i + 1) == Some(&b'(') => {
                let open = b[..i].iter().rposition(|&x| x == b'[');
                let label = open.map(|o| md[o + 1..i].to_string());
                out.push((label, target_at(md, i + 2)));
            }
            // `<scheme:target>`.
            b'<' if b.get(i + 1).is_some_and(u8::is_ascii_alphabetic) => {
                let end = md[i + 1..].find(['>', ' ', '\n']).map(|e| i + 1 + e);
                if let Some(end) = end.filter(|&e| b[e] == b'>') {
                    let inner = &md[i + 1..end];
                    let scheme_len = inner.bytes().take_while(|x| x.is_ascii_alphanumeric() || b"+.-".contains(x)).count();
                    if inner.as_bytes().get(scheme_len) == Some(&b':') {
                        out.push((None, inner.to_string()));
                    }
                }
            }
            _ => {}
        }
    }
    // `[label]: target`, at the start of a line.
    for line in md.lines() {
        let line = line.trim_start_matches(' ');
        if let Some(rest) = line.strip_prefix('[')
            && let Some((label, after)) = rest.split_once("]:")
            && !label.is_empty()
        {
            out.push((Some(label.to_string()), target_at(after.trim_start(), 0)));
        }
    }
    out
}

/// The link target starting at `from`: optionally in `<>`, ending at a space or
/// the closing parenthesis, with balanced parentheses inside.
fn target_at(text: &str, from: usize) -> String {
    let rest = text[from..].trim_start();
    let rest = rest.strip_prefix('<').unwrap_or(rest);
    let mut depth = 0usize;
    let mut end = rest.len();
    for (i, c) in rest.char_indices() {
        match c {
            '(' => depth += 1,
            ')' if depth == 0 => {
                end = i;
                break;
            }
            ')' => depth -= 1,
            c if c.is_whitespace() || c == '>' => {
                end = i;
                break;
            }
            _ => {}
        }
    }
    rest[..end].to_string()
}
