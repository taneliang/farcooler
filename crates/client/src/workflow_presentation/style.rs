//! Each platform sets its labels in its own case; the copy is written once, in sentence case.
//!
//! Apple sets buttons, menu items and headings in title case ("Cancel Run"), Material in sentence case
//! ("Cancel run"), and the web page follows Apple's habit today. A renderer says which it is and shows
//! what it gets, with no casing of its own. There is deliberately no default: a host that forgets its
//! style gets a refusal, not another platform's case.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Style { Apple, Material, Web }

impl Style {
    pub const ALL: [Style; 3] = [Style::Apple, Style::Material, Style::Web];

    /// A label (a button, a heading, a short name) from its sentence-case source.
    /// Body sentences go through as they are: they are not labels.
    pub fn label(self, sentence_case: &str) -> String {
        match self {
            Style::Apple | Style::Web => title_case(sentence_case),
            Style::Material => sentence_case.to_owned(),
        }
    }
}

/// Words Apple leaves lowercase inside a title: articles, coordinating conjunctions and prepositions of
/// four letters or fewer. The first and the last word are always capitalized.
const SMALL: &[&str] = &["a", "an", "the", "and", "but", "or", "nor", "for", "yet", "so", "as", "at", "by",
    "in", "of", "on", "to", "from", "with", "into", "onto", "per", "via"];

/// A word that already carries a capital after its first letter (CI, iOS) is a name and is left alone.
pub fn title_case(text: &str) -> String {
    let words: Vec<&str> = text.split(' ').collect();
    let last = words.iter().rposition(|w| w.chars().any(char::is_alphabetic));
    words.iter().enumerate().map(|(i, word)| {
        let name = word.chars().skip(1).any(char::is_uppercase);
        let small = SMALL.contains(word);
        if name || (small && i != 0 && Some(i) != last) { return (*word).to_owned(); }
        let mut chars = word.chars();
        match chars.next() {
            Some(first) => first.to_uppercase().chain(chars).collect(),
            None => String::new(),
        }
    }).collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apple_and_web_set_title_case_and_material_keeps_sentence_case() {
        let cases = [
            ("Awaiting approval", "Awaiting Approval"),
            ("Queued in a batch", "Queued in a Batch"),
            ("Waiting to land", "Waiting to Land"),
            ("Wait for base", "Wait for Base"),
            ("Inspect latest evidence…", "Inspect Latest Evidence…"),
            ("Check CI", "Check CI"),
            ("Wait for CI", "Wait for CI"),
            ("Cancel run", "Cancel Run"),
            ("Rework iteration", "Rework Iteration"),
            ("Worktree has changes", "Worktree Has Changes"),
            ("Open pull request", "Open Pull Request"),
        ];
        for (source, apple) in cases {
            assert_eq!(Style::Apple.label(source), apple, "{source}");
            assert_eq!(Style::Web.label(source), apple, "{source}");
            assert_eq!(Style::Material.label(source), source, "{source}");
        }
    }

    #[test]
    fn a_small_word_is_capitalized_at_either_edge_only() {
        assert_eq!(title_case("to do"), "To Do");
        assert_eq!(title_case("what to do with"), "What to Do With");
        assert_eq!(title_case("sign out…"), "Sign Out…");
    }

    #[test]
    fn a_style_is_never_assumed() {
        assert_eq!(serde_json::from_str::<Style>("\"material\"").unwrap(), Style::Material);
        assert!(serde_json::from_str::<Style>("\"Apple\"").is_err());
        assert!(serde_json::from_str::<Style>("null").is_err());
    }
}
