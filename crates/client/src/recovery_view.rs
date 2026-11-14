//! Account-owned passkey recovery presentation. The browser owns DOM and I/O;
//! this view owns the words and count/identity decisions (ov-1009, R-112).
use serde_json::{Value, json};

pub fn view(count: usize, current: bool) -> Value {
    json!({
        "title": "Saved Copies",
        "explanation": "Each passkey keeps one sealed recovery copy shared by its browsers. Clearing a copy removes recovery for those browsers; their local runners and passkey stay. An active browser may save a copy again after a touch.",
        "close": "Close",
        "loading": "Loading saved copies…",
        "loadFailed": "Far Cooler couldn’t load your saved copies. Close this window and try again.",
        "summary": match count {
            0 => "No saved copies.".to_owned(),
            1 => "1 saved copy.".to_owned(),
            n => format!("{n} saved copies."),
        },
        "passkey": if current { "This browser’s passkey" } else { "Another passkey" },
        "clear": "Clear Copy",
        "cleared": if count == 0 { "No saved copies." } else { "Saved copy cleared." },
        "clearFailed": "Far Cooler couldn’t clear that saved copy. Try again.",
        "signingOut": "Signing out…",
        "deleteTitle": "Your saved copy may still be at Far Cooler",
        "deleteDetail": "Far Cooler couldn’t confirm that your passkey’s shared recovery copy was deleted. It may still restore your runners in browsers using this passkey. Retry, or sign out and clear it later from Saved Copies.",
        "retry": "Retry Delete",
        "anyway": "Sign Out Anyway",
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_and_passkey_identity() {
        assert_eq!(view(0, false)["summary"], "No saved copies.");
        assert_eq!(view(1, true)["summary"], "1 saved copy.");
        assert_eq!(view(2, false)["summary"], "2 saved copies.");
        assert_eq!(view(1, true)["passkey"], "This browser’s passkey");
        assert_eq!(view(1, false)["passkey"], "Another passkey");
        assert_eq!(view(0, false)["cleared"], "No saved copies.");
        assert_eq!(view(1, false)["cleared"], "Saved copy cleared.");
    }

    #[test]
    fn bundled_view_is_the_shared_view() {
        let fixture: Value = serde_json::from_str(include_str!("testdata/recovery-view.json")).unwrap();
        assert_eq!(view(2, true), fixture);
    }
}
