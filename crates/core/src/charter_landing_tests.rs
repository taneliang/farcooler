use super::*;

#[test]
fn globs_match_whole_paths() {
    assert!(glob_matches("services/site/**", "services/site/public/a.html"));
    assert!(!glob_matches("services/site/**", "services/sites/a"));
    assert!(glob_matches("*.md", "README.md"));
    assert!(!glob_matches("*.md", "docs/a.md"), "`*` stays in one folder");
    assert!(glob_matches("**/*.md", "docs/a.md"));
    assert!(glob_matches("**/*.md", "a.md"), "`**/` matches no folder too");
    assert!(glob_matches("docs/?.md", "docs/a.md"));
    assert!(!glob_matches("docs/?.md", "docs/ab.md"));
}
