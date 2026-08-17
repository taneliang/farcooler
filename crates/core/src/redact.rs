//! Strip anything credential-shaped before it leaves the runner.
//!
//! A pane's feed and its blocked question are put on the wire and into push
//! payloads, so they reach a phone through the relay. The strings involved are
//! short, but one of them is a shell command, and a shell command is exactly
//! where a token lives: `curl -H "Authorization: Bearer …"`, an inline
//! `AWS_SECRET_ACCESS_KEY=`, a URL with a key in the query.
//!
//! Conservative on purpose. A redaction that fires on something harmless costs
//! a few characters of context; one that misses puts a live credential on a
//! lock screen.
//!
//! Recognizing a secret is structural, not lexical: `NAME=value` and
//! `--flag value` are shapes, so `TOKEN=` and `--password` are caught
//! wherever they appear, but a bare word requires the flag's leading `-` to
//! trigger the check. Without that requirement, an agent's own blocked
//! question — prose like "add auth token parsing" — would get mangled on
//! every mention of a security-flavored word. The trade-off is deliberate: a
//! secret typed directly into prose (`the password is hunter2`) is NOT
//! caught, because telling that apart from ordinary sentences needs natural
//! language understanding this module does not have, and a filter with a
//! false-positive rate high enough to annoy is a filter that gets turned off.
//!
//! # Shapes deliberately left alone
//!
//! Each of these is a real way to spell a secret that this module does not
//! catch. They are listed so the next reader can tell "nobody thought of it"
//! from "somebody decided against it":
//!
//! * **`-p secret`** (sshpass, mysql). `-p` is also `cargo test -p
//!   farcooler-core` and `cargo run -p api`, which scroll past constantly in
//!   this repo's own terminals. A filter that eats the package name off every
//!   cargo command is a filter that gets switched off, which costs more than
//!   this case. The word-flag spellings (`--password`) are caught.
//! * **A JSON body**, `{"password": "hunter2"}`. Finding it means parsing
//!   JSON — quoting, nesting, escapes — not scanning whitespace-separated
//!   tokens. It is a different tool, not a bigger version of this one.
//! * **A protocol-relative URL**, `//user:pw@host`. Treating a bare `//` as
//!   the start of an authority fires on ordinary absolute paths and on
//!   doubled slashes in comments; the shape does not appear in terminal
//!   output.
//! * **A password containing a literal `://`**. The scan would have to guess
//!   which `://` starts the real URI. Contrived enough not to pay for.
//! * **Over-redacting `let token=parse(x);`** — a line of source code where
//!   the whole variable name IS `token` reads as a secret assignment and
//!   comes back `token=…;`. Losing a few characters of quoted code in an
//!   agent's question is a small, visible cost; the alternative is not
//!   redacting `TOKEN=` at all.
//! * **Over-redacting a URL with a port and a later '@' in its path** —
//!   `https://cdn.example.com:8443/assets/logo@3x.svg` comes back
//!   `https://cdn.example.com:…@3x.svg`, losing the port and the whole path.
//!   That is [`redact_userinfo_past_a_slash`] doing its job on a string that
//!   happens to share the shape of a password containing a '/', and retina
//!   asset naming beside a dev port is plausible enough to be worth stating.
//!   A narrowing exists and is written down here rather than applied, because
//!   the trade is still the right way round — a mangled asset URL costs less
//!   than a leaked database password: in the fallback, if the text between the
//!   ':' and the '@' contains a '/' AND everything before that '/' is all
//!   digits, it is `host:port/path`, not userinfo, and can be skipped. That
//!   keeps `admin:p4ss/w0rd@` redacting and leaves `:8443/assets/logo@` alone.
//! * **Over-redacting `keys=3`** — a plain count comes back `keys=…`, because
//!   [`names_a_secret`] strips a plural "s" and `KEY` is a secret word (it has
//!   to be, to cover `API_KEY`, `ACCESS_KEY` and `PRIVATE_KEY` with one
//!   entry). No leak, and the same class of cosmetic cost as `let token=…`
//!   above.

/// Words that make the thing after them a secret.
///
/// Compared for EQUALITY against a name's last segment, so an entry here must
/// be a single segment: `KEY` covers `AWS_SECRET_ACCESS_KEY`, `api_key` and
/// `--api-key` on its own, and a multi-segment entry like `ACCESS_KEY` could
/// never match anything.
const SECRET_WORDS: &[&str] = &[
    "SECRET", "TOKEN", "PASSWORD", "PASSWD", "APIKEY", "KEY", "CREDENTIAL",
    "AUTH",
];

/// Auth schemes that make the rest of an `Authorization:` header a secret.
///
/// `bearer` is unmistakable, but the others are ordinary English words, so
/// they only count when an `Authorization:` header is actually there to
/// introduce them — see [`introduces_a_secret`].
const AUTH_SCHEMES: &[&str] = &["basic", "token", "apikey"];

/// `text` with anything credential-shaped replaced by an ellipsis.
///
/// Hand-written rather than regex-driven: this crate has no regex dependency,
/// the shapes are few, and a hand-rolled scan is easier to read than the
/// alternation that would replace it.
pub fn redact(text: &str) -> String {
    // A line at a time: the scan works on whitespace-separated tokens, and
    // `split_whitespace` treats a newline as just another space, so redacting
    // the whole blob at once would rebuild it as a single line. Captured
    // terminal output is many lines, and its shape is most of its meaning.
    let lines: Vec<String> = text.split('\n').map(redact_line).collect();
    lines.join("\n")
}

/// One newline-free line, redacted.
fn redact_line(text: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut redact_next = false;
    let mut previous_was_authorization = false;

    for token in text.split_whitespace() {
        // Read before any branch below can `continue` past it: whether THIS
        // token is the header decides how the NEXT one is read.
        let this_is_authorization = is_an_authorization_header(token);
        let after_authorization = previous_was_authorization;
        previous_was_authorization = this_is_authorization;

        if redact_next {
            redact_next = false;
            // A flag with no operand is followed by another flag, not a value.
            if !token.starts_with('-') {
                out.push("…".to_string());
                continue;
            }
        }

        if introduces_a_secret(token, after_authorization) {
            out.push(token.to_string());
            redact_next = true;
            continue;
        }

        out.push(redact_token(token));
    }

    // `split_whitespace` collapses runs of spaces, so a line that needed no
    // redaction would still come back subtly different. Returning the original
    // keeps `redact` a no-op on the overwhelmingly common case.
    let rebuilt = out.join(" ");
    if rebuilt == text.split_whitespace().collect::<Vec<_>>().join(" ") {
        return text.to_string();
    }
    rebuilt
}

/// Whether the token AFTER this one is the secret.
///
/// Three shapes, one rule: `--password hunter2`; the `Bearer sk-…` that an
/// `Authorization:` header splits into two words; and the other auth schemes
/// — `Basic`, `token`, `ApiKey` — which only count when an `Authorization:`
/// header is present to introduce them, either as the PREVIOUS token
/// (`after_authorization`, the spaced form) or glued onto the front of this
/// one. That restriction is load-bearing rather than tidy: "token" is an
/// ordinary English word, and firing on it unconditionally would eat the next
/// word of every sentence like "the token expired an hour ago".
fn introduces_a_secret(token: &str, after_authorization: bool) -> bool {
    let word = final_segment(token, ':');
    if word.eq_ignore_ascii_case("bearer") {
        return true;
    }
    let introduced_by_a_header = after_authorization || glues_an_authorization_header(token);
    if introduced_by_a_header && AUTH_SCHEMES.iter().any(|s| word.eq_ignore_ascii_case(s)) {
        return true;
    }

    // Require the token to look like a flag before checking the word list —
    // otherwise ordinary prose containing "token" or "auth" would eat the
    // word after it. See the module doc for why prose secrets go uncaught.
    if !token.starts_with('-') {
        return false;
    }
    names_a_secret(token.trim_start_matches('-'))
}

/// Whether `token` carries an `Authorization:` header glued to its scheme.
///
/// `-H 'Authorization:Basic dXNl…'` is what a shell hands over when the header
/// has no space after its colon: header and scheme arrive as ONE token, so
/// [`is_an_authorization_header`] never sees the header on its own and the
/// previous-token gate can never fire. `Bearer` survived that only because it
/// is matched unconditionally; `Basic`, `token` and `ApiKey` leaked.
///
/// The scheme is the token's last ':'-separated segment, so the header name is
/// the one before it — which is exactly the test, and exactly why an ordinary
/// sentence cannot trip it: a bare "token" has no second-to-last segment at
/// all.
fn glues_an_authorization_header(token: &str) -> bool {
    let quotes = |c: char| c == '"' || c == '\'';
    let trimmed = token.trim_matches(quotes).trim_end_matches(':');
    let mut segments = trimmed.rsplit(':');
    // The scheme, discarded: the caller has already read it as `word`.
    let _scheme = segments.next();
    segments
        .next()
        .is_some_and(|name| name.trim_matches(quotes).eq_ignore_ascii_case("authorization"))
}

/// Whether `token` is an `Authorization:` header field name.
///
/// `-H "Authorization: Basic …"` reaches here as the token `"Authorization:`,
/// so the wrapping quote and the header's own trailing colon come off before
/// the comparison.
fn is_an_authorization_header(token: &str) -> bool {
    final_segment(token, ':').eq_ignore_ascii_case("authorization")
}

/// The last `separator`-delimited piece of `token`, with wrapping quotes and
/// any trailing separators removed.
///
/// This is what a human reads as "the word": in the glued
/// `"Authorization:Bearer` a shell produces when there is no space after the
/// colon, the word is `Bearer`, and in `"Authorization:` it is
/// `Authorization`, not the empty string after the final colon.
fn final_segment(token: &str, separator: char) -> &str {
    let quotes = |c: char| c == '"' || c == '\'';
    let trimmed = token.trim_matches(quotes).trim_end_matches(separator);
    let last = trimmed.rsplit(separator).next().unwrap_or(trimmed);
    last.trim_matches(quotes)
}

/// Whether `name` names a secret.
///
/// Matched by SEGMENT, not by substring: a name is split on `_`, `-` and `.`,
/// and only its LAST segment — minus one trailing plural "s" — is compared,
/// for equality, against [`SECRET_WORDS`]. Substring matching read `AUTH`
/// inside `AUTHOR_NAME`, `SECRET` inside `SECRET_SANTA`, `KEY` inside
/// `API_KEYS_DIR` and `api_keyword`, and `TOKEN` inside `TOKEN_COUNT`, and
/// destroyed the value of every one of them.
///
/// The last segment is the right one because these names read as a noun
/// phrase with the important word at the end: `AWS_SECRET_ACCESS_KEY`,
/// `spring.datasource.password`, `--api-key`. Anything else in the name
/// qualifies which secret it is, not whether it is one.
fn names_a_secret(name: &str) -> bool {
    let last = name.rsplit(['_', '-', '.']).next().unwrap_or(name);
    let upper = last.to_ascii_uppercase();
    let singular = upper.strip_suffix('S').unwrap_or(&upper);
    SECRET_WORDS.contains(&singular)
}

/// One whitespace-free token, redacted if it carries a secret inside it.
fn redact_token(token: &str) -> String {
    // `scheme://user:password@host` carries its secret with no `=` and no
    // `?` in sight — a connection string like `postgres://user:hunter2@…`
    // would slip past the branches below, so it gets checked first.
    //
    // This has to CHAIN into the query branch below rather than return
    // early: a single URI can carry a password in its userinfo AND a key in
    // its query string at once (`postgres://user:pass@host/db?api_key=…`),
    // and an early return here catches only the first, shipping the second
    // in the clear. So the userinfo redaction's own output is what the query
    // branch then scans, not the original token.
    let token: String = match redact_userinfo(token) {
        Some(redacted) => redacted,
        None => token.to_string(),
    };
    let token: &str = &token;

    // A URL query is handled first and per-parameter, so a key in the query
    // does not take the rest of the URL down with it — `?api_key=…&page=2`
    // should still say which page.
    if token.contains('?') && token.contains('=') {
        return redact_query(token);
    }

    // NAME=value, where NAME says it is a secret.
    if let Some((name, value)) = token.split_once('=') {
        if !value.is_empty() && names_a_secret(name.trim_start_matches('-')) {
            // A shell command sometimes glues a separator straight onto the
            // value with no space (`TOKEN=abc;./run.sh`) — keep a trailing
            // `;` or `,` so the line still reads as two commands instead of
            // losing the separator along with the secret.
            let trailing: String = value
                .chars()
                .rev()
                .take_while(|c| matches!(c, ';' | ','))
                .collect::<Vec<char>>()
                .into_iter()
                .rev()
                .collect();
            return format!("{name}=…{trailing}");
        }
    }

    token.to_string()
}

/// Every `scheme://user:password@host` inside `token` with its password
/// replaced, and every other byte — scheme, username, host, port, path,
/// query, fragment — left exactly as it was.
///
/// The shape, from RFC 3986:
///
/// ```text
/// scheme "://" authority [ "/" path ] [ "?" query ] [ "#" fragment ]
/// authority = [ userinfo "@" ] host [ ":" port ]
/// ```
///
/// A password can live in exactly one place, the authority, so the rule is
/// "find the authorities, redact inside them, copy everything else". Each of
/// that sentence's three parts has already cost this module a leak, so each
/// is stated deliberately:
///
/// * There can be more than one authority in a token, because a URI can carry
///   another URI in its path or its query
///   (`https://host/cb?url=https://user:pw@evil`). The nested password is as
///   live as the outer one and the key that carries it (`url`) says nothing
///   about it, so every `://` in the token is scanned, not just the first.
/// * An authority BEGINS right after `://` and ENDS at the first `/`, `?` or
///   `#` — whichever comes first, not `/` alone. Ending it only at `/` drags
///   a query or fragment into the authority: on a URI with no userinfo,
///   `https://host?q=SECRET:pw@evil` gets cut as though the query were a
///   password, and on one that does have userinfo, the query's '@' is taken
///   for the userinfo terminator and the real host is thrown away.
/// * Inside an authority, userinfo ends at the LAST '@', because a password
///   may contain an unescaped '@' and only the final one precedes the host.
///   Splitting on the first stops inside the password and ships its tail.
///
/// The known cost of following the RFC on that second point: a password with
/// a raw, unescaped '?' or '#' in it (`https://user:p?w@host`) ends the
/// authority early and is left alone. That is what every URI parser does with
/// such a string — the '?' starts the query — and buying that case back means
/// reading a query's '@' as a userinfo terminator again, which is the leak
/// this function was rewritten to close. A password containing '?' has to be
/// percent-encoded to work at all, and once encoded it redacts normally.
///
/// `None` when nothing was redacted, so the caller can tell "no credential
/// here" from "a credential that happened to rebuild identically".
fn redact_userinfo(token: &str) -> Option<String> {
    // The grammar first, always: when a token parses as a URI, its parse is
    // the right answer and the looser scan below never runs. The fallback
    // exists only for tokens the grammar finds no userinfo in at all.
    redact_userinfo_by_the_grammar(token).or_else(|| redact_userinfo_past_a_slash(token))
}

/// The strict parse: authority bounded by the first `/`, `?` or `#`.
fn redact_userinfo_by_the_grammar(token: &str) -> Option<String> {
    let mut out = String::new();
    let mut rest = token;
    let mut found_one = false;

    while let Some(scheme_end) = rest.find("://") {
        let (before_scheme, from_scheme) = rest.split_at(scheme_end);
        let after_scheme = &from_scheme["://".len()..];

        let authority_end = after_scheme
            .find(['/', '?', '#'])
            .unwrap_or(after_scheme.len());
        let (authority, after_authority) = after_scheme.split_at(authority_end);

        out.push_str(before_scheme);
        out.push_str("://");
        match redact_authority(authority) {
            Some(redacted) => {
                out.push_str(&redacted);
                found_one = true;
            }
            None => out.push_str(authority),
        }

        // Resume at the authority's end, never inside it: the path or query
        // that follows is where any further `://` can be.
        rest = after_authority;
    }

    if !found_one {
        return None;
    }
    out.push_str(rest);
    Some(out)
}

/// One authority — the text between `://` and the path, query, or fragment —
/// with its password replaced.
///
/// `None` when there is no password to hide: no '@' (a plain `host:8080`), or
/// a short userinfo with no ':' in it (a bare `user@host`, whose only colon,
/// if any, is the port's — and a port sits after the '@', outside the
/// userinfo).
///
/// A userinfo with NO ':' is not automatically innocent, though: the whole
/// field can BE the credential, as in
/// `https://ghp_16C7e…@github.com/o/r.git`, which is what `git clone` with a
/// personal access token looks like. Length is what tells that apart from a
/// username — 20 characters is longer than the `user`, `admin` and `git` that
/// appear in an ordinary `user@host`, and shorter than any token worth
/// stealing.
fn redact_authority(authority: &str) -> Option<String> {
    /// Shortest userinfo that is a credential rather than a username.
    const TOKEN_LENGTH: usize = 20;

    let at_pos = authority.rfind('@')?;
    // `host` keeps the '@' so the rebuilt string does not have to add one.
    let (userinfo, host) = authority.split_at(at_pos);

    // Every offset here is where an ASCII delimiter starts, so no slice can
    // land inside a multi-byte character of a non-ASCII username or host.
    match userinfo.split_once(':') {
        Some((user, _password)) => Some(format!("{user}:…{host}")),
        // Counted in characters, not bytes, so a non-ASCII name is measured
        // the way a person would read it.
        None if userinfo.chars().count() >= TOKEN_LENGTH => Some(format!("…{host}")),
        None => None,
    }
}

/// The fallback parse, for a password with a '/' in it.
///
/// The grammar above ends an authority at the first '/', which is correct —
/// but when the '/' is INSIDE the password (`https://user:sk/abc@host/db`,
/// and every base64 secret contains '/' eventually) the authority it cuts has
/// no '@' in it, no userinfo is found, and the token sails through untouched
/// with the password in the clear.
///
/// So: when the strict parse finds nothing anywhere in the token, scan again
/// with the span bounded by '?' or '#' ONLY, and read the LAST '@' in that
/// span as the userinfo terminator. This runs second, never instead, because
/// on a well-formed URI it would be wrong — `https://user:pw@host/path/to@thing`
/// would cut at the path's '@' — and the strict parse has already claimed
/// every such token before this is reached.
///
/// The '?'/'#' bound still holds, so a query's '@'
/// (`https://host?foo=SECRET:pw@evil`) is out of the span and stays put, and
/// a span with no ':' before its '@' (`https://x.com/a@b`) has no password
/// position to redact.
///
/// The known cost: a path that contains both a ':' and a later '@', such as
/// `https://host:8080/img@2x.png`, has no password in it but matches this
/// shape and comes back cut. That is a few characters of a URL — visible,
/// harmless, and paid to catch a real credential whose only sin is a slash.
fn redact_userinfo_past_a_slash(token: &str) -> Option<String> {
    let mut out = String::new();
    let mut rest = token;
    let mut found_one = false;

    while let Some(scheme_end) = rest.find("://") {
        let (before_scheme, from_scheme) = rest.split_at(scheme_end);
        let after_scheme = &from_scheme["://".len()..];

        let span_end = after_scheme.find(['?', '#']).unwrap_or(after_scheme.len());
        let (span, after_span) = after_scheme.split_at(span_end);

        out.push_str(before_scheme);
        out.push_str("://");
        match redact_slashed_userinfo(span) {
            Some(redacted) => {
                out.push_str(&redacted);
                found_one = true;
            }
            None => out.push_str(span),
        }

        rest = after_span;
    }

    if !found_one {
        return None;
    }
    out.push_str(rest);
    Some(out)
}

/// One `?`/`#`-bounded span with everything from its first ':' to its last
/// '@' replaced. `None` when the span has no '@', or no ':' before it.
fn redact_slashed_userinfo(span: &str) -> Option<String> {
    let at_pos = span.rfind('@')?;
    let (userinfo, host) = span.split_at(at_pos);
    let colon = userinfo.find(':')?;
    let user = &userinfo[..colon];
    Some(format!("{user}:…{host}"))
}

/// A URL with its secret query parameters replaced, the rest intact.
fn redact_query(token: &str) -> String {
    let (head, query) = match token.split_once('?') {
        Some(parts) => parts,
        None => ("", token),
    };
    let cleaned: Vec<String> = query
        .split('&')
        .map(|pair| match pair.split_once('=') {
            Some((k, v)) if !v.is_empty() => {
                if names_a_secret(k) {
                    format!("{k}=…")
                } else {
                    pair.to_string()
                }
            }
            _ => pair.to_string(),
        })
        .collect();
    if head.is_empty() {
        cleaned.join("&")
    } else {
        format!("{head}?{}", cleaned.join("&"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bearer_token_does_not_travel() {
        let out = redact("curl -H \"Authorization: Bearer sk-abc123DEF456ghi789\" https://api.example.com");
        assert!(!out.contains("sk-abc123DEF456ghi789"), "{out}");
        assert!(out.contains("Bearer"), "the shape stays, so the line still reads: {out}");
    }

    #[test]
    fn a_secret_environment_assignment_does_not_travel() {
        for raw in [
            "AWS_SECRET_ACCESS_KEY=wJalrXUtnFEMIK7MDENGbPxRfiCYEXAMPLEKEY make deploy",
            "GITHUB_TOKEN=ghp_16C7e42F292c6912E7710c838347Ae178B4a cargo publish",
            "export DATABASE_PASSWORD=hunter2correcthorse",
            "MY_API_KEY=abcdef123456 npm start",
        ] {
            let out = redact(raw);
            assert!(!out.contains("wJalrXUtnFEMIK7MDENGbPxRfiCYEXAMPLEKEY"), "{out}");
            assert!(!out.contains("ghp_16C7e42F292c6912E7710c838347Ae178B4a"), "{out}");
            assert!(!out.contains("hunter2correcthorse"), "{out}");
            assert!(!out.contains("abcdef123456"), "{out}");
        }
    }

    #[test]
    fn a_key_in_a_url_query_does_not_travel() {
        let out = redact("curl 'https://api.example.com/v1/thing?api_key=live_9f8e7d6c5b4a3210&page=2'");
        assert!(!out.contains("live_9f8e7d6c5b4a3210"), "{out}");
        assert!(out.contains("page=2"), "the harmless parameter survives: {out}");
    }

    #[test]
    fn a_password_flag_does_not_travel() {
        let out = redact("mysql -u root --password supersecretvalue mydb");
        assert!(!out.contains("supersecretvalue"), "{out}");
    }

    #[test]
    fn an_ordinary_command_is_left_alone() {
        // The cost of a false positive is a line that stops making sense, so
        // the common cases are asserted to pass through untouched.
        for raw in [
            "cargo test -p farcooler-core",
            "git commit -m \"fix the parser\"",
            "pnpm dev",
            "Would you like to run the following command?",
            "vim src/some/deeply/nested/module.rs",
        ] {
            assert_eq!(redact(raw), raw, "a harmless line was redacted");
        }
    }

    #[test]
    fn nothing_in_means_nothing_out() {
        assert_eq!(redact(""), "");
    }

    #[test]
    fn a_glued_authorization_header_does_not_travel() {
        // No space after the colon: `Authorization:Bearer` is one token, not
        // the two-word shape the bearer rule was written to match.
        let out = redact("curl -H \"Authorization:Bearer abc123\" https://x.com");
        assert!(!out.contains("abc123"), "{out}");
    }

    #[test]
    fn a_glued_authorization_header_redacts_schemes_other_than_bearer() {
        // `Bearer` survived the glued form only because it is matched with no
        // gate at all. The other three were gated on the PREVIOUS token being
        // the header, which cannot happen when the shell produced one token —
        // so every one of these shipped its credential in the clear.
        for (raw, secret) in [
            ("curl -H 'Authorization:Basic dXNlcjpodW50ZXIy' https://x.com", "dXNlcjpodW50ZXIy"),
            ("curl -H 'Authorization:token ghp_ABCDEFGHIJKLMNOP' https://x.com", "ghp_ABCDEFGHIJKLMNOP"),
            ("curl -H 'Authorization:ApiKey live_9f8e7d6c' https://x.com", "live_9f8e7d6c"),
            // The quoting a shell leaves behind varies; the header does not.
            ("curl -H \"Authorization:Basic dXNlcjpodW50ZXIy\" https://x.com", "dXNlcjpodW50ZXIy"),
            ("curl -H Authorization:token ghp_ABCDEFGHIJKLMNOP https://x.com", "ghp_ABCDEFGHIJKLMNOP"),
        ] {
            let out = redact(raw);
            assert!(!out.contains(secret), "{raw} -> {out}");
        }
    }

    #[test]
    fn the_glued_header_rule_does_not_fire_on_prose() {
        // The reason the scheme words were gated in the first place. Reading
        // the second-to-last ':' segment cannot reach any of these, because a
        // bare word has no second-to-last segment — but that is a claim worth
        // pinning, since "token" and "basic" are ordinary English.
        for raw in [
            "the token expired an hour ago",
            "git commit -m \"add auth token parsing\"",
            "basic auth is fine",
            "I used token based login",
            // A colon that is not a header's: neither segment names one.
            "note: token rotation is monthly",
            "https://x.com/basic guide",
        ] {
            assert_eq!(redact(raw), raw, "prose was read as a glued header");
        }
    }

    #[test]
    fn a_connection_uri_password_does_not_travel() {
        let out = redact("psql \"postgres://user:hunter2@localhost/db\"");
        assert!(!out.contains("hunter2"), "{out}");
        assert!(out.contains("localhost/db"), "the host and database survive: {out}");
    }

    #[test]
    fn a_connection_uri_without_a_password_is_left_alone() {
        for raw in [
            "psql \"postgres://localhost/db\"",
            "psql \"postgres://user@localhost/db\"",
            "curl https://x.com/a@b",
            // No `://`, so it must not be mistaken for userinfo: the `:` here
            // separates host from path in scp-style git syntax, not a password.
            "git clone git@github.com:owner/repo.git",
        ] {
            assert_eq!(redact(raw), raw, "a harmless URI was redacted");
        }
    }

    #[test]
    fn a_bare_secret_word_in_prose_does_not_eat_the_next_word() {
        // These are the agent's own words, not a command it wants to run —
        // redacting them would mangle the very question this crate exists to
        // carry intact.
        for raw in [
            "git commit -m \"add auth token parsing\"",
            "the token expired an hour ago",
        ] {
            assert_eq!(redact(raw), raw, "prose was mistaken for a flag");
        }
    }

    #[test]
    fn trailing_punctuation_after_a_secret_value_survives() {
        let out = redact("export TOKEN=abc; ./run.sh");
        assert!(!out.contains("abc"), "{out}");
        assert!(out.contains("; ./run.sh"), "the separator survives: {out}");
    }

    #[test]
    fn a_password_flag_using_equals_still_redacts() {
        let out = redact("mysql --password=hunter2 -u root mydb");
        assert!(!out.contains("hunter2"), "{out}");
    }

    #[test]
    fn whitespace_only_input_is_left_alone() {
        assert_eq!(redact("   "), "   ");
    }

    #[test]
    fn non_ascii_input_does_not_panic() {
        let _ = redact("--password=café");
        let _ = redact("このコマンドを実行してもよろしいですか");
    }

    #[test]
    fn a_userinfo_password_and_a_query_secret_both_redact() {
        // A URI can carry a secret in two places at once — the userinfo
        // redaction must not short-circuit the query redaction.
        let out = redact("postgres://user:hunter2@host/db?sslmode=require&api_key=live_9f8");
        assert!(!out.contains("hunter2"), "{out}");
        assert!(!out.contains("live_9f8"), "{out}");
        assert!(out.contains("sslmode=require"), "the harmless parameter survives: {out}");
    }

    #[test]
    fn a_second_userinfo_and_query_combination_both_redact() {
        let out = redact("https://user:pw@api.example.com/v1?token=abc123&page=2");
        assert!(!out.contains("pw"), "{out}");
        assert!(!out.contains("abc123"), "{out}");
        assert!(out.contains("page=2"), "the harmless parameter survives: {out}");
    }

    #[test]
    fn an_at_sign_inside_a_password_does_not_leak_a_fragment() {
        // Userinfo runs to the LAST '@' before the host — a password
        // containing its own unescaped '@' must not be mistaken for the
        // userinfo terminator, or everything after it ships in the clear.
        assert_eq!(redact("https://user:p@w:ord@host/db"), "https://user:…@host/db");
    }

    #[test]
    fn a_second_at_sign_inside_a_password_does_not_leak_a_fragment() {
        assert_eq!(redact("postgres://user:pa@ss@localhost/db"), "postgres://user:…@localhost/db");
    }

    #[test]
    fn an_at_sign_only_in_the_path_is_left_alone() {
        // No '@' appears before the first '/', so there is no userinfo at all.
        for raw in ["https://x.com/a@b", "https://x.com/path/to@thing"] {
            assert_eq!(redact(raw), raw, "a path '@' was mistaken for userinfo");
        }
    }

    #[test]
    fn userinfo_redaction_still_chains_into_query_redaction() {
        // The last-'@' fix must not reopen the round-2 bug where a userinfo
        // redaction suppressed a query redaction on the same token.
        let out = redact("postgres://user:hunter2@host/db?api_key=live_9f8");
        assert_eq!(out, "postgres://user:…@host/db?api_key=…");
    }

    #[test]
    fn userinfo_with_a_missing_username_or_password_is_handled() {
        assert_eq!(redact("postgres://:pw@host/db"), "postgres://:…@host/db");
        // An empty password is still a password *position* — no panic, and
        // the (empty) value at that position is still replaced.
        let out = redact("postgres://user:@host/db");
        assert!(!out.contains("user:@"), "{out}");
    }

    #[test]
    fn an_at_sign_with_no_scheme_is_left_alone() {
        // `user@host` with no leading `scheme://` isn't a URI at all — ssh's
        // own `user@host` syntax must not be misread as userinfo.
        assert_eq!(
            redact("ssh -i ~/.ssh/id_rsa user@host"),
            "ssh -i ~/.ssh/id_rsa user@host"
        );
    }

    #[test]
    fn non_ascii_userinfo_does_not_panic() {
        let out = redact("postgres://user:пароль@host/db");
        assert!(!out.contains("пароль"), "{out}");
    }

    #[test]
    fn a_query_or_fragment_ends_the_authority() {
        // The authority stops at the first '/', '?' or '#'. Stopping only at
        // '/' let a query or fragment be read as part of the userinfo, which
        // both leaked query text and destroyed the host.
        assert_eq!(redact("https://user:pw@host?q=a@b"), "https://user:…@host?q=a@b");
        assert_eq!(redact("https://user:pw@host#frag@ment"), "https://user:…@host#frag@ment");
        assert_eq!(redact("https://user:pw@host?a=1#f@g"), "https://user:…@host?a=1#f@g");
    }

    #[test]
    fn an_at_sign_in_a_query_is_not_userinfo() {
        // There is no userinfo here at all — the '@' is inside the query, so
        // the value before it must not be mistaken for a password and cut.
        // (A genuinely secret query key is the query rules' job, not this one.)
        for raw in [
            "https://host?foo=SECRETVALUE:pw@evil",
            "https://host?a=b@c&d=e@f",
            "https://host#anchor:with@sign",
        ] {
            assert_eq!(redact(raw), raw, "a query or fragment '@' was read as userinfo");
        }
    }

    #[test]
    fn a_password_and_a_query_secret_both_go_when_the_password_holds_an_at_sign() {
        assert_eq!(
            redact("https://user:p@w@host/db?token=abc&page=2"),
            "https://user:…@host/db?token=…&page=2"
        );
    }

    #[test]
    fn a_port_is_not_a_password() {
        // The ':' that matters is the one INSIDE the userinfo. A port colon
        // sits after the '@', so `user@host:5432` still has no password.
        assert_eq!(redact("postgres://user@localhost:5432/db"), "postgres://user@localhost:5432/db");
        assert_eq!(redact("https://host:8080/path"), "https://host:8080/path");
        assert_eq!(
            redact("postgres://user:pw@host:5432/db"),
            "postgres://user:…@host:5432/db"
        );
    }

    #[test]
    fn a_uri_missing_its_pieces_does_not_panic_or_mangle() {
        // Empty authority, empty host, empty everything: none of these carry a
        // password, and none of them may panic on a slice.
        for raw in ["https://host", "https://", "https:///path", "https://@host/db", "://"] {
            assert_eq!(redact(raw), raw, "a URI with a missing piece was altered");
        }
        // A password with no host after it is still a password.
        assert_eq!(redact("https://user:pw@"), "https://user:…@");
        assert_eq!(redact("https://user:pw@?q=1"), "https://user:…@?q=1");
        assert_eq!(redact("://user:pw@host"), "://user:…@host");
    }

    #[test]
    fn a_uri_nested_inside_another_uri_loses_its_password_too() {
        // A redirect parameter carries a whole second URI, and its userinfo is
        // just as live as the outer one's. The key (`url`) says nothing about
        // it, so the `scheme://user:pass@` shape is the only signal there is —
        // which is why every `://` in a token gets scanned, not just the first.
        assert_eq!(
            redact("https://host/cb?url=https://user:pw@evil.example"),
            "https://host/cb?url=https://user:…@evil.example"
        );
        assert_eq!(
            redact("https://host/fetch/https://user:pw@evil.example"),
            "https://host/fetch/https://user:…@evil.example"
        );
        assert_eq!(
            redact("https://host#u=https://user:pw@evil.example"),
            "https://host#u=https://user:…@evil.example"
        );
        // Both passwords and the secret query key, all in one token.
        assert_eq!(
            redact("https://user:pw@host/db?api_key=live&url=https://u:p@h"),
            "https://user:…@host/db?api_key=…&url=https://u:…@h"
        );
    }

    #[test]
    fn non_ascii_all_the_way_through_a_uri_does_not_panic() {
        // Every offset this module slices at comes from finding an ASCII
        // delimiter, so a multi-byte username, password, or host must never
        // land mid-character.
        let out = redact("postgres://пользователь:пароль@хост/db");
        assert_eq!(out, "postgres://пользователь:…@хост/db");
        let out = redact("https://用户:密码@例え.com/パス?q=値");
        assert!(!out.contains("密码"), "{out}");
        assert!(out.contains("例え.com"), "the host survives: {out}");
    }

    #[test]
    fn a_secret_word_inside_a_longer_name_is_not_a_secret() {
        // A name is matched a segment at a time, so an ordinary word that
        // merely CONTAINS a secret word — AUTHor, SECRET_santa, api_KEYword —
        // keeps its value. Substring matching mangled all of these.
        for raw in [
            "AUTHOR_NAME=jane ./release.sh",
            "SECRET_SANTA=alice ./run.sh",
            "API_KEYS_DIR=/etc/keys ./deploy.sh",
            "CREDENTIALING_ENABLED=true ./start.sh",
            "TOKEN_COUNT=5 ./x.sh",
            "curl 'https://example.com/search?api_keyword=turtles&page=2'",
        ] {
            assert_eq!(redact(raw), raw, "a name that only contains a secret word was redacted");
        }
    }

    #[test]
    fn a_name_whose_last_segment_is_a_secret_word_still_redacts() {
        for (raw, secret) in [
            ("AWS_SECRET_ACCESS_KEY=wJalrXUtnFEMI make deploy", "wJalrXUtnFEMI"),
            ("GITHUB_TOKEN=ghp_16C7e42F292 cargo publish", "ghp_16C7e42F292"),
            ("DATABASE_PASSWORD=hunter2correct psql", "hunter2correct"),
            ("MY_API_KEY=abcdef123456 npm start", "abcdef123456"),
            ("curl 'https://x.com/v1?api_key=live_9f8e7d&page=2'", "live_9f8e7d"),
            ("aws --secrets supersecretvalue s3 ls", "supersecretvalue"),
        ] {
            let out = redact(raw);
            assert!(!out.contains(secret), "{raw} -> {out}");
        }
    }

    #[test]
    fn a_slash_inside_a_password_does_not_save_it() {
        // Base64 alphabets include '/', so a password with a slash in it is an
        // ordinary shape — and it puts the authority's end (per the grammar,
        // the first '/') INSIDE the password.
        for (raw, secret) in [
            ("https://user:sk/abc@host/db", "sk/abc"),
            ("http://us/er:pw@host/path", "pw@"),
            ("postgres://admin:p4ss/w0rd@prod-db.internal:5432/app", "p4ss/w0rd"),
        ] {
            let out = redact(raw);
            assert!(!out.contains(secret), "{raw} -> {out}");
        }
        assert_eq!(redact("https://user:sk/abc@host/db"), "https://user:…@host/db");
        assert_eq!(
            redact("postgres://admin:p4ss/w0rd@prod-db.internal:5432/app"),
            "postgres://admin:…@prod-db.internal:5432/app"
        );
    }

    #[test]
    fn the_slashed_password_fallback_does_not_fire_on_ordinary_urls() {
        for raw in [
            "https://x.com/a@b",
            "https://x.com/a@b/c@d",
            "https://host?foo=SECRET:pw@evil",
        ] {
            assert_eq!(redact(raw), raw, "the fallback fired on a URL with no password");
        }
        // The strict parse wins first, so the fallback never sees this one and
        // the path's '@' survives.
        assert_eq!(
            redact("https://user:pw@host/path/to@thing"),
            "https://user:…@host/path/to@thing"
        );
    }

    #[test]
    fn an_authorization_header_redacts_schemes_other_than_bearer() {
        for (raw, secret) in [
            ("curl -H \"Authorization: Basic dXNlcjpodW50ZXIy\" https://x.com", "dXNlcjpodW50ZXIy"),
            ("curl -H \"Authorization: token ghp_16C7e42F292\" https://x.com", "ghp_16C7e42F292"),
            ("curl -H \"Authorization: ApiKey live_9f8e7d6c\" https://x.com", "live_9f8e7d6c"),
        ] {
            let out = redact(raw);
            assert!(!out.contains(secret), "{raw} -> {out}");
        }
    }

    #[test]
    fn an_auth_scheme_word_outside_a_header_is_still_ordinary_prose() {
        // "token", "basic" and "apikey" are English words. Only the preceding
        // `Authorization:` makes the word after them a credential.
        for raw in [
            "the token expired an hour ago",
            "git commit -m \"add auth token parsing\"",
            "the basic idea is fine",
            "token refresh happens hourly",
        ] {
            assert_eq!(redact(raw), raw, "an auth scheme word ate the next word in prose");
        }
    }

    #[test]
    fn a_userinfo_that_is_only_a_token_does_not_travel() {
        let out = redact("git clone https://ghp_16C7e42F292c6912E7710c838347Ae178B4a@github.com/o/r.git");
        assert_eq!(out, "git clone https://…@github.com/o/r.git");
    }

    #[test]
    fn a_short_userinfo_with_no_password_is_left_alone() {
        for raw in [
            "https://user@host/db",
            "ssh -i ~/.ssh/id_rsa user@host",
            "git clone git@github.com:owner/repo.git",
        ] {
            assert_eq!(redact(raw), raw, "an ordinary username was mistaken for a token");
        }
    }

    #[test]
    fn multiple_lines_keep_their_line_breaks() {
        let out = redact("export TOKEN=abc\ndo something else\nwith multiple   spaces");
        assert_eq!(out, "export TOKEN=…\ndo something else\nwith multiple   spaces");
    }

    #[test]
    fn redaction_is_idempotent() {
        // A redacted line should never change further if it is redacted a
        // second time — the relay or a retry path might run this twice.
        for raw in [
            "https://user:p@w:ord@host/db",
            "postgres://user:pa@ss@localhost/db",
            "https://x.com/a@b",
            "https://x.com/path/to@thing",
            "postgres://user:hunter2@host/db?api_key=live_9f8",
            "postgres://user:hunter2@localhost/db",
            "postgres://localhost/db",
            "postgres://user@localhost/db",
            "postgres://:pw@host/db",
            "postgres://user:@host/db",
            "git clone git@github.com:owner/repo.git",
            "ssh -i ~/.ssh/id_rsa user@host",
            "https://user:pw@api.example.com/v1?token=abc&page=2",
            "curl 'https://api.example.com/v1/thing?api_key=live_9f8&page=2'",
            "curl -H \"Authorization:Bearer abc123\" https://x.com",
            "curl -H 'Authorization:Basic dXNlcjpodW50ZXIy' https://x.com",
            "curl -H 'Authorization:token ghp_ABCDEFGHIJKLMNOP' https://x.com",
            "AWS_SECRET_ACCESS_KEY=wJalrXUtnFEMI make deploy",
            "export TOKEN=abc; ./run.sh",
            "the token expired an hour ago",
            "cargo test -p farcooler-core",
            "",
            "   ",
            "postgres://user:пароль@host/db",
            "https://user:pw@host?q=a@b",
            "https://user:pw@host#frag@ment",
            "https://host?foo=SECRETVALUE:pw@evil",
            "https://user:p@w@host/db?token=abc&page=2",
            "postgres://user@localhost:5432/db",
            "postgres://user:pw@host:5432/db",
            "https://host",
            "https://",
            "https://@host/db",
            "https://user:pw@",
            "https://host/cb?url=https://user:pw@evil.example",
            "https://host/fetch/https://user:pw@evil.example",
            "https://host#u=https://user:pw@evil.example",
            "https://user:pw@host/db?api_key=live&url=https://u:p@h",
            "https://user:pw@[::1]:5432/db",
            "postgres://пользователь:пароль@хост/db",
            "https://用户:密码@例え.com/パス?q=値",
            "AUTHOR_NAME=jane ./release.sh",
            "API_KEYS_DIR=/etc/keys ./deploy.sh",
            "curl 'https://example.com/search?api_keyword=turtles&page=2'",
            "https://user:sk/abc@host/db",
            "http://us/er:pw@host/path",
            "postgres://admin:p4ss/w0rd@prod-db.internal:5432/app",
            "https://x.com/a@b/c@d",
            "https://host?foo=SECRET:pw@evil",
            "https://user:pw@host/path/to@thing",
            "curl -H \"Authorization: Basic dXNlcjpodW50ZXIy\" https://x.com",
            "curl -H \"Authorization: token ghp_16C7e42F292\" https://x.com",
            "git clone https://ghp_16C7e42F292c6912E7710c838347Ae178B4a@github.com/o/r.git",
            "https://user@host/db",
            "export TOKEN=abc\ndo something else\nwith multiple   spaces",
        ] {
            let once = redact(raw);
            let twice = redact(&once);
            assert_eq!(once, twice, "redacting twice changed the output for {raw:?}: {once:?} -> {twice:?}");
        }
    }
}
