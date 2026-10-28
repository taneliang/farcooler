//! `client.revoke`'s one optional argument (ov-855).

use serde_json::Value;

/// Whether the revoke also takes the devices this one added, and those they
/// added in turn (`ClientRevoke.with_added`, ov-838). Absent is false: the one
/// device, which is what every app before this sent.
pub(super) fn with_added(args: &Value) -> bool {
    args.get("withAdded").and_then(Value::as_bool).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn only_an_explicit_true_takes_the_devices_it_added() {
        assert!(with_added(&json!({ "clientId": "phone-1", "withAdded": true })));
        assert!(!with_added(&json!({ "clientId": "phone-1", "withAdded": false })));
        assert!(!with_added(&json!({ "clientId": "phone-1" })), "absent is the one device");
        assert!(!with_added(&json!({ "clientId": "phone-1", "withAdded": "true" })), "a string is not a yes");
    }
}
