//! A device's step-up key (ov-856, slice 3 of `reports/ov-856/design.md`
//! section 3): the key it signs admin actions with, in its own table and never
//! read back from the SSH line (B3).
//!
//! - **`trusted` or `proposed`.** Only a stepped-up enroll by a trusted key of
//!   the same person, or an approval with someone present (ov-980), writes a
//!   trusted key, and the Mac app its own (`replace_step_up_key`). Every other
//!   key is a proposal, and a proposal signs nothing.
//! - **A proposal lives ten minutes** ([`PROPOSAL_MS`]): an approval after that
//!   is refused, so a sheet left open on someone's Mac can't trust a key
//!   proposed yesterday.
//! - **One key per device.** A second is refused `key_exists`; rotating needs
//!   a revoke and a new enroll, which clears the row.
//! - **`sign_count`** is a WebAuthn passkey's counter, kept so a cloned
//!   authenticator's lower count is refused ([`Store::use_sign_count`]).
//! - Trusting a key bumps the generation of the person the device is bound
//!   to, so a request decided before it is refused `access_changed`. Writing
//!   a key for a device that has none changes nobody's authority yet, and two
//!   people pairing at once must not refuse each other.

use rusqlite::{OptionalExtension, params};

use crate::error::map_err;
use crate::store::Store;
use farcooler_core::{DomainError, Result};

/// How long a proposed key waits for its approval.
pub const PROPOSAL_MS: i64 = 10 * 60 * 1000;

/// One device's step-up key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepUpKeyRow {
    pub client_id: String,
    /// `p256-sha256` or `webauthn-es256`.
    pub alg: String,
    /// The 65-byte uncompressed P-256 point.
    pub public_key: Vec<u8>,
    pub trusted: bool,
    pub registered_at: i64,
    pub sign_count: u32,
}

impl Store {
    /// `device`'s step-up key, if it has one.
    pub fn step_up_key(&self, device: &str) -> Result<Option<StepUpKeyRow>> {
        self.conn()
            .query_row(
                "SELECT client_id, alg, pubkey, state, registered_at, sign_count FROM step_up_keys WHERE client_id = ?1",
                [device],
                |r| {
                    Ok(StepUpKeyRow {
                        client_id: r.get(0)?,
                        alg: r.get(1)?,
                        public_key: r.get(2)?,
                        trusted: r.get::<_, String>(3)? == "trusted",
                        registered_at: r.get(4)?,
                        sign_count: r.get(5)?,
                    })
                },
            )
            .optional()
            .map_err(map_err)
    }

    /// Every device whose step-up key is trusted (ov-988: whether the owner
    /// already has one, so a first-key code has nothing left to do).
    pub fn trusted_step_up_devices(&self) -> Result<Vec<String>> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT client_id FROM step_up_keys WHERE state = 'trusted' ORDER BY client_id").map_err(map_err)?;
        let rows = stmt.query_map([], |r| r.get(0)).map_err(map_err)?;
        rows.collect::<rusqlite::Result<Vec<String>>>().map_err(map_err)
    }

    /// Record `device`'s key, `trusted` or proposed. A device with a trusted
    /// key is refused `key_exists`: rotating needs a revoke and a new enroll.
    /// A proposal is replaceable, by a new proposal or by a trusted key,
    /// because it signs nothing and expires after
    /// [`PROPOSAL_MS`]; were it not, a device whose proposal went stale could
    /// never ask again. Bumps nothing: see the module.
    pub fn put_step_up_key(&self, device: &str, alg: &str, public_key: &[u8], trusted: bool, now_ms: i64) -> Result<()> {
        let conn = self.conn();
        let state = if trusted { "trusted" } else { "proposed" };
        let added = conn
            .execute(
                "INSERT INTO step_up_keys (client_id, alg, pubkey, state, registered_at, sign_count) VALUES (?1, ?2, ?3, ?4, ?5, 0)
                 ON CONFLICT (client_id) DO UPDATE SET alg = ?2, pubkey = ?3, state = ?4, registered_at = ?5, sign_count = 0 WHERE state = 'proposed'",
                params![device, alg, public_key, state, now_ms],
            )
            .map_err(map_err)?;
        if added == 0 {
            return Err(DomainError::Conflict { what: "key_exists" });
        }
        Ok(())
    }

    /// Record `device`'s key as trusted, replacing whatever it had: the Mac
    /// app's own key on the runner's socket (ov-974), which only the app puts
    /// here (`step_up_rpc::set_key`). Resets the counter.
    pub fn replace_step_up_key(&self, device: &str, alg: &str, public_key: &[u8], now_ms: i64) -> Result<()> {
        self.conn()
            .execute(
                "INSERT INTO step_up_keys (client_id, alg, pubkey, state, registered_at, sign_count) VALUES (?1, ?2, ?3, 'trusted', ?4, 0)
                 ON CONFLICT (client_id) DO UPDATE SET alg = ?2, pubkey = ?3, state = 'trusted', registered_at = ?4, sign_count = 0",
                params![device, alg, public_key, now_ms],
            )
            .map_err(map_err)?;
        Ok(())
    }

    /// Trust `device`'s proposed key. Refused `not_found` with no proposal,
    /// and `proposal_expired` once [`PROPOSAL_MS`] has passed. A key that is
    /// trusted already stays so.
    pub fn trust_step_up_key(&self, device: &str, now_ms: i64) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction().map_err(map_err)?;
        let row: Option<(String, i64)> = tx
            .query_row("SELECT state, registered_at FROM step_up_keys WHERE client_id = ?1", [device], |r| Ok((r.get(0)?, r.get(1)?)))
            .optional()
            .map_err(map_err)?;
        match row {
            None => return Err(DomainError::NotFound),
            Some((state, _)) if state == "trusted" => return Ok(()),
            Some((_, at)) if now_ms - at > PROPOSAL_MS => return Err(DomainError::Conflict { what: "proposal_expired" }),
            Some(_) => {}
        }
        tx.execute("UPDATE step_up_keys SET state = 'trusted' WHERE client_id = ?1", [device]).map_err(map_err)?;
        crate::access_ops::bump_holder_in(&tx, device)?;
        tx.commit().map_err(map_err)
    }

    /// Record that `by` vouched for `device`'s trusted key with the owner
    /// present (ov-974, B1): the Mac app with Touch ID, or a device the owner
    /// already vouched for, with its own step-up. Bound to the key's bytes, so
    /// a new key under the id is vouched by nobody. Refused `key_untrusted`
    /// for a device whose key isn't trusted.
    pub fn vouch_step_up_key(&self, device: &str, by: &str) -> Result<()> {
        let key = self.step_up_key(device)?.filter(|k| k.trusted).ok_or(DomainError::Conflict { what: "key_untrusted" })?;
        self.conn()
            .execute(
                "INSERT INTO meta (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![vouch_key(device), format!("{} {by}", fingerprint(&key))],
            )
            .map_err(map_err)?;
        Ok(())
    }

    /// Who vouched for `device`'s key as it is now, if anyone did and it is
    /// still trusted. Kept in `meta` rather than a column, so no migration
    /// lands this close to the engine cutover.
    pub fn step_up_vouched_by(&self, device: &str) -> Result<Option<String>> {
        let Some(key) = self.step_up_key(device)?.filter(|k| k.trusted) else { return Ok(None) };
        let said: Option<String> =
            self.conn().query_row("SELECT value FROM meta WHERE key = ?1", [vouch_key(device)], |r| r.get(0)).optional().map_err(map_err)?;
        Ok(said.and_then(|said| said.strip_prefix(&format!("{} ", fingerprint(&key))).map(str::to_string)))
    }

    /// Forget who vouched for `device`'s key: its rows are going (a revoke).
    pub(crate) fn forget_vouch_in(tx: &rusqlite::Transaction, device: &str) -> Result<()> {
        tx.execute("DELETE FROM meta WHERE key = ?1", [vouch_key(device)]).map_err(map_err)?;
        Ok(())
    }

    /// Accept a WebAuthn signature counter of `count` for `device`: refused
    /// `step_up_replayed` when it didn't move forward (a cloned authenticator
    /// or a replay). A counter that is 0 before and after is an authenticator
    /// that keeps none (a synced passkey) and is accepted.
    pub fn use_sign_count(&self, device: &str, count: u32) -> Result<()> {
        let conn = self.conn();
        let before: u32 = conn
            .query_row("SELECT sign_count FROM step_up_keys WHERE client_id = ?1", [device], |r| r.get(0))
            .optional()
            .map_err(map_err)?
            .ok_or(DomainError::NotFound)?;
        if (before > 0 || count > 0) && count <= before {
            return Err(DomainError::Conflict { what: "step_up_replayed" });
        }
        conn.execute("UPDATE step_up_keys SET sign_count = ?2 WHERE client_id = ?1", params![device, count]).map_err(map_err)?;
        Ok(())
    }
}

fn vouch_key(device: &str) -> String {
    format!("step_up_vouch:{device}")
}

/// A key as a vouch names it: its algorithm and its bytes, in hex.
fn fingerprint(key: &StepUpKeyRow) -> String {
    let hex: String = key.public_key.iter().map(|b| format!("{b:02x}")).collect();
    format!("{}:{hex}", key.alg)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> Store {
        let store = Store::open_in_memory().unwrap();
        store.set_member("bob", "Bob", &["pad-b".into()]).unwrap();
        store
    }

    /// One key a device, insert-only; a proposal trusts once, inside its ten
    /// minutes, and trusting moves the holder's generation.
    #[test]
    fn a_proposal_is_trusted_once_inside_its_window() {
        let s = store();
        s.put_step_up_key("pad-b", "p256-sha256", &[4; 65], false, 1_000).unwrap();
        assert!(!s.step_up_key("pad-b").unwrap().unwrap().trusted);
        let before = s.access_generation("bob").unwrap();
        assert!(matches!(s.trust_step_up_key("pad-b", 1_000 + PROPOSAL_MS + 1), Err(DomainError::Conflict { what: "proposal_expired" })));
        s.trust_step_up_key("pad-b", 1_000 + PROPOSAL_MS).unwrap();
        assert!(s.step_up_key("pad-b").unwrap().unwrap().trusted);
        assert_eq!(s.access_generation("bob").unwrap(), before + 1);
        assert!(matches!(s.trust_step_up_key("nowhere", 0), Err(DomainError::NotFound)));
    }

    /// A proposal is replaced by a new one, however old; a trusted key is
    /// never replaced by a write, so a stale proposal can be asked again and a
    /// trusted key cannot be swapped.
    #[test]
    fn a_proposal_is_replaceable_and_a_trusted_key_is_not() {
        let s = store();
        s.put_step_up_key("pad-b", "p256-sha256", &[4; 65], false, 1_000).unwrap();
        s.put_step_up_key("pad-b", "p256-sha256", &[5; 65], false, 2_000).unwrap();
        let row = s.step_up_key("pad-b").unwrap().unwrap();
        assert_eq!((row.public_key, row.registered_at, row.trusted), (vec![5; 65], 2_000, false));
        s.trust_step_up_key("pad-b", 2_000).unwrap();
        for trusted in [false, true] {
            assert!(matches!(s.put_step_up_key("pad-b", "p256-sha256", &[6; 65], trusted, 3_000), Err(DomainError::Conflict { what: "key_exists" })));
        }
        assert_eq!(s.step_up_key("pad-b").unwrap().unwrap().public_key, vec![5; 65]);
    }

    /// A vouch names the key it was given for: a proposal can't be vouched,
    /// a new key under the id (after a revoke) is vouched by nobody, and a
    /// revoke forgets it.
    #[test]
    fn a_vouch_holds_for_its_key_alone() {
        let s = store();
        s.put_step_up_key("pad-b", "p256-sha256", &[4; 65], false, 1_000).unwrap();
        assert!(matches!(s.vouch_step_up_key("pad-b", "local:app"), Err(DomainError::Conflict { what: "key_untrusted" })));
        assert_eq!(s.step_up_vouched_by("pad-b").unwrap(), None);
        s.trust_step_up_key("pad-b", 1_000).unwrap();
        assert_eq!(s.step_up_vouched_by("pad-b").unwrap(), None, "trusted is not vouched");
        s.vouch_step_up_key("pad-b", "local:app").unwrap();
        assert_eq!(s.step_up_vouched_by("pad-b").unwrap().as_deref(), Some("local:app"));
        // Another key under the id, its vouch written behind the store's back.
        s.conn().execute("UPDATE step_up_keys SET pubkey = ?1 WHERE client_id = 'pad-b'", [vec![5u8; 65]]).unwrap();
        assert_eq!(s.step_up_vouched_by("pad-b").unwrap(), None, "a vouch for another key");
        s.conn().execute("UPDATE step_up_keys SET pubkey = ?1 WHERE client_id = 'pad-b'", [vec![4u8; 65]]).unwrap();
        s.clear_device_rows("pad-b").unwrap();
        s.put_step_up_key("pad-b", "p256-sha256", &[4; 65], true, 2_000).unwrap();
        assert_eq!(s.step_up_vouched_by("pad-b").unwrap(), None, "a revoke forgets the vouch, even for the same key again");
    }

    /// A counter that stands still or goes back is refused; one that never
    /// leaves zero is an authenticator that keeps none.
    #[test]
    fn a_sign_count_only_moves_forward() {
        let s = store();
        s.put_step_up_key("pad-b", "webauthn-es256", &[4; 65], true, 0).unwrap();
        s.use_sign_count("pad-b", 0).unwrap();
        s.use_sign_count("pad-b", 0).unwrap();
        s.use_sign_count("pad-b", 5).unwrap();
        for stale in [5, 4, 0] {
            assert!(matches!(s.use_sign_count("pad-b", stale), Err(DomainError::Conflict { what: "step_up_replayed" })), "{stale}");
        }
        s.use_sign_count("pad-b", 6).unwrap();
    }
}
