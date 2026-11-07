//! The rules a `route` step pins: which route a ticket takes, by the paths it changes (ov-959).
//!
//! The step is generic: it reads the paths, applies rules the definition carries, and answers with a
//! label that the definition routes. Nothing here knows about landing. A charter's `## Landing` section
//! is one place the rules can come from; a person can write them for any other reason.
use farcooler_core::charter_landing::{glob_matches, Route};
use serde::{Deserialize, Serialize};

/// The most rules one step carries, and the longest argument it may pin.
pub const MAX_RULES: usize = 32;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PathRules {
    pub default: String,
    /// First match wins, per file.
    pub rules: Vec<(String, String)>,
}

/// The label a route answers with. `pull_request` is a route no definition can take yet, so the step
/// answers `rejected` for it and the run ends instead of landing the wrong way.
pub fn label(route: Route) -> &'static str {
    match route { Route::Push => "push", Route::Ask => "ask", Route::PullRequest => "pull_request" }
}
fn parse(label: &str) -> Option<Route> {
    match label { "push" => Some(Route::Push), "ask" => Some(Route::Ask), "pull_request" => Some(Route::PullRequest), _ => None }
}
fn care(route: Route) -> u8 { match route { Route::Push => 0, Route::PullRequest => 1, Route::Ask => 2 } }

impl PathRules {
    pub fn new(default: Route, rules: &[(String, Route)]) -> Self {
        Self { default: label(default).into(), rules: rules.iter().map(|(g, r)| (g.clone(), label(*r).into())).collect() }
    }
    /// The text a step pins as its argument.
    pub fn argument(&self) -> String { serde_json::to_string(self).expect("rules serialize") }
    /// Rules a step may pin: readable, bounded, with a real label on each.
    pub fn read(argument: &str) -> Option<Self> {
        let rules: Self = serde_json::from_str(argument).ok()?;
        let sound = rules.rules.len() <= MAX_RULES && parse(&rules.default).is_some()
            && rules.rules.iter().all(|(glob, route)| !glob.is_empty() && glob.len() <= 256 && parse(route).is_some());
        sound.then_some(rules)
    }
    /// The route of a ticket that changes `paths`: its most careful file's, each file taking the first rule
    /// that matches it. A ticket that changes nothing takes the default.
    pub fn route_of(&self, paths: &[String]) -> Route {
        let default = parse(&self.default).unwrap_or(Route::Ask);
        let route_of_file = |path: &str| self.rules.iter().find(|(glob, _)| glob_matches(glob, path))
            .and_then(|(_, route)| parse(route)).unwrap_or(default);
        paths.iter().map(|p| route_of_file(p)).max_by_key(|r| care(*r)).unwrap_or(default)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn rules() -> PathRules { PathRules::new(Route::Push, &[("services/site/**".into(), Route::Ask), ("docs/**".into(), Route::Push)]) }
    fn paths(list: &[&str]) -> Vec<String> { list.iter().map(|p| p.to_string()).collect() }

    #[test]
    fn a_ticket_takes_its_most_careful_files_route() {
        let r = rules();
        assert_eq!(r.route_of(&paths(&["src/a.rs", "docs/b.md"])), Route::Push);
        assert_eq!(r.route_of(&paths(&["src/a.rs", "services/site/index.html"])), Route::Ask);
        assert_eq!(r.route_of(&[]), Route::Push, "no change takes the default");
    }
    #[test]
    fn the_first_matching_rule_decides_a_file() {
        let r = PathRules::new(Route::Ask, &[("docs/**".into(), Route::Push), ("**".into(), Route::Ask)]);
        assert_eq!(r.route_of(&paths(&["docs/a.md"])), Route::Push);
        assert_eq!(r.route_of(&paths(&["docs/a.md", "src/x"])), Route::Ask);
    }
    #[test]
    fn the_pinned_text_round_trips_and_a_bad_one_is_refused() {
        let r = rules();
        assert_eq!(PathRules::read(&r.argument()), Some(r));
        for bad in ["", "push", r#"{"default":"fly","rules":[]}"#, r#"{"default":"push","rules":[["","ask"]]}"#,
            r#"{"default":"push","rules":[["a","nope"]]}"#, r#"{"default":"push","rules":[],"extra":1}"#] {
            assert_eq!(PathRules::read(bad), None, "{bad}");
        }
        let many = PathRules { default: "push".into(), rules: (0..=MAX_RULES).map(|i| (format!("d{i}/**"), "ask".into())).collect() };
        assert_eq!(PathRules::read(&many.argument()), None);
    }
}
