//! What a tool row opens to (ov-452): the call's input and its result, as a
//! person reads them, cut short enough to travel with the row.
//!
//! A row is served whole on every page and kept in every client's cache, so
//! neither part is the call's full text: each is cut at `DETAIL_CHARS`
//! characters or `DETAIL_LINES` lines, whichever comes first, with `…` where
//! it was cut. The terminal has the rest.

use serde_json::Value;

/// The most characters a tool row's input or result keeps.
pub const DETAIL_CHARS: usize = 2_000;

/// The most lines a tool row's input or result keeps.
pub const DETAIL_LINES: usize = 40;

/// `text`, cut to `DETAIL_LINES` lines and `DETAIL_CHARS` characters, line
/// breaks and indentation kept. `None` when nothing is left after trimming.
pub fn excerpt(text: &str) -> Option<String> {
    let text = text.trim_matches(|c: char| c == '\n' || c == '\r').trim_end();
    if text.trim().is_empty() {
        return None;
    }
    let mut end = text.len();
    if let Some((cut, _)) = text.match_indices('\n').nth(DETAIL_LINES - 1) {
        end = cut;
    }
    if let Some((cut, _)) = text[..end].char_indices().nth(DETAIL_CHARS) {
        end = cut;
    }
    Some(if end < text.len() { format!("{}…", text[..end].trim_end()) } else { text.to_string() })
}

/// A call's input as lines a person reads: `key: value` for each field, a
/// string's own words unquoted (a multi-line one on the lines after its key),
/// anything else as compact JSON. `None` for an empty input.
pub fn input_text(input: &Value) -> Option<String> {
    match input {
        Value::Object(fields) => fields_text(fields.iter().map(|(k, v)| (k.as_str(), v))),
        Value::Null => None,
        Value::String(s) => excerpt(s),
        other => excerpt(&other.to_string()),
    }
}

/// `input_text`, from the input's JSON text as the transcript wrote it, its
/// fields in the order they were written (a `Value`'s map sorts them).
pub fn input_text_of(raw: &str) -> Option<String> {
    match serde_json::from_str::<Ordered>(raw) {
        Ok(Ordered(fields)) => fields_text(fields.iter().map(|(k, v)| (k.as_str(), v))),
        Err(_) => serde_json::from_str::<Value>(raw).ok().as_ref().and_then(input_text),
    }
}

/// An object's fields, in the order written.
struct Ordered(Vec<(String, Value)>);

impl<'de> serde::Deserialize<'de> for Ordered {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> serde::de::Visitor<'de> for V {
            type Value = Ordered;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("an object")
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(self, mut map: A) -> Result<Ordered, A::Error> {
                let mut fields = Vec::new();
                while let Some(entry) = map.next_entry::<String, Value>()? {
                    fields.push(entry);
                }
                Ok(Ordered(fields))
            }
        }
        d.deserialize_map(V)
    }
}

fn fields_text<'v>(fields: impl Iterator<Item = (&'v str, &'v Value)>) -> Option<String> {
    let mut out = String::new();
    for (key, value) in fields {
        if out.len() > DETAIL_CHARS * 4 {
            break;
        }
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(key);
        out.push(':');
        match value {
            Value::String(s) if s.contains('\n') => {
                out.push('\n');
                out.push_str(s.trim_end());
            }
            Value::String(s) => {
                out.push(' ');
                out.push_str(s);
            }
            other => {
                out.push(' ');
                out.push_str(&other.to_string());
            }
        }
    }
    excerpt(&out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn an_input_reads_as_one_field_per_line_with_strings_unquoted() {
        let raw = r#"{"cron":"4 15 3 10 *","recurring":false,"prompt":"Restart chain.\nRe-arm it."}"#;
        assert_eq!(input_text_of(raw).as_deref(), Some("cron: 4 15 3 10 *\nrecurring: false\nprompt:\nRestart chain.\nRe-arm it."), "in the order written");
        assert_eq!(input_text(&json!({})), None, "an empty input opens to nothing");
        assert_eq!(input_text_of("{}"), None);
    }

    #[test]
    fn a_long_result_is_cut_at_its_line_or_character_limit_and_says_so() {
        let lines: String = (0..100).map(|n| format!("line {n}\n")).collect();
        let cut = excerpt(&lines).unwrap();
        assert_eq!(cut.lines().count(), DETAIL_LINES);
        assert!(cut.ends_with("line 39…"), "{cut}");
        let wide = "x".repeat(DETAIL_CHARS * 2);
        assert_eq!(excerpt(&wide).unwrap().chars().count(), DETAIL_CHARS + 1);
        assert_eq!(excerpt("  indented\n  kept").as_deref(), Some("  indented\n  kept"));
        assert_eq!(excerpt("\n \n"), None);
    }
}
