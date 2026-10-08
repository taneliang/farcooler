use super::*;

#[test]
fn http_and_https_pages_are_opened_as_given() {
    for url in [
        "https://github.com",
        "https://github.com/",
        "HTTPS://www.notion.so/acme/Roadmap-1a2b3c?pvs=4#heading",
        "http://localhost:3000/",
        "https://linear.app/acme/issue/ENG-12/a-title",
        "https://user@example.com:8443/path",
        "https://[::1]:8080/",
        "https://例え.jp/パス",
    ] {
        assert_eq!(checked_url(url).expect(url), url, "returned as given");
    }
}

#[test]
fn everything_else_is_refused() {
    let long = format!("https://example.com/{}", "a".repeat(LONGEST_URL));
    for url in [
        "",
        "github.com",
        "//github.com",
        "file:///etc/passwd",
        "javascript:alert(1)",
        "JavaScript://github.com/%0Aalert(1)",
        "data:text/html,<p>hi</p>",
        "about:blank",
        "linear://issue/ENG-12",
        "ftp://example.com/",
        "https:github.com",
        "https://",
        "https:///path",
        "https://user@/",
        "https://:443/",
        "https://exa mple.com/",
        "https://example.com/\n",
        "https://example.com/\u{7}",
        "https://example.com/\u{a0}",
        long.as_str(),
    ] {
        let refused = checked_url(url).expect_err(url);
        assert!(refused.to_string().contains("http or https"), "{url}: {refused}");
    }
}
