//! The two calls `farcooler join` makes to the relay (ov-1490): post the
//! sealed reply under the browser's ticket, and add this runner to the
//! account once the person has approved it in the browser.
//!
//! Neither carries a credential: the ticket, and the claim the relay answers a
//! reply with, only let this one command speak for the one runner the person
//! asked about. A runner's token comes back from the second, once, and is
//! saved as a push pairing.

use serde_json::Value;

/// A relay's address as this command will call it: https, or http on this
/// computer alone, so a ticket never crosses a network in the clear.
pub fn relay_origin(relay: &str) -> Result<String, String> {
    let url = reqwest::Url::parse(relay.trim()).map_err(|_| format!("{relay} isn't a relay address."))?;
    let local = matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"));
    if url.scheme() != "https" && !(url.scheme() == "http" && local) {
        return Err("The relay must be an https address.".into());
    }
    Ok(url.origin().ascii_serialization())
}

/// `POST <relay>/v1/join/<route>`, answered with the status and its JSON, or
/// `Null` for a body that isn't JSON.
pub async fn post(relay: &str, route: &str, body: &Value) -> Result<(u16, Value), String> {
    let client = reqwest::Client::builder().timeout(std::time::Duration::from_secs(15)).build().map_err(|e| e.to_string())?;
    let response = client
        .post(format!("{relay}/v1/join/{route}"))
        .json(body)
        .send()
        .await
        .map_err(|_| "Far Cooler couldn't reach the relay. Check this computer's connection and run the command again.".to_string())?;
    let status = response.status().as_u16();
    Ok((status, response.json().await.unwrap_or(Value::Null)))
}

#[cfg(test)]
mod tests {
    use super::relay_origin;

    #[test]
    fn a_ticket_never_crosses_a_network_in_the_clear() {
        assert_eq!(relay_origin("https://relay.farcooler.com/").unwrap(), "https://relay.farcooler.com");
        assert_eq!(relay_origin("http://127.0.0.1:8787").unwrap(), "http://127.0.0.1:8787");
        for bad in ["http://relay.farcooler.com", "http://127.0.0.1.evil.example", "ftp://localhost", "relay"] {
            assert!(relay_origin(bad).is_err(), "{bad}");
        }
    }
}
