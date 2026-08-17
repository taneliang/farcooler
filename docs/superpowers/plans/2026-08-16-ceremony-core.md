# Ceremony Core Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the two-QR ceremony's logic once, in Rust, so iOS, Android and macOS share one implementation of every rule that decides whether an enrollment is safe — and add the relay's two changes: proof of possession at registration, and the account lookup.

**Architecture:** A `ceremony` module in `crates/client` encodes and decodes both QR payloads and enforces every validation rule, exposed through the existing C FFI that `farcooler_client_generate_key` already uses. The apps get pixels and a camera; they get no say in whether a scan is acceptable. The relay gains one route and one column.

**Tech Stack:** Rust (`crates/client`), `serde`/`serde_json` and `base64` (already in tree), TypeScript on Cloudflare Workers with vitest and D1.

**Spec:** [`docs/superpowers/specs/2026-08-16-device-onboarding-design.md`](../specs/2026-08-16-device-onboarding-design.md) — "The ceremony", "Two gates, and only one of them is the account", "What the relay stores".

**Depends on:** all three previous plans.

## Global Constraints

- **US English throughout.** **Never run `cargo fmt`.** **`cargo` is at `~/.cargo/bin/cargo`.**
- **Far Cooler implements no cryptography itself**, here or anywhere. Signatures and hashes come from `ssh-key` and `sha2`, which are already in the tree.
- **Nothing in either QR is a secret.** The first carries public keys and an account id; the second carries addresses. A design that needs a secret in a QR has gone wrong — three drafts did, and the history section of the spec says how.
- **Freshness is judged by the scanner's clock**, never by a timestamp inside the code, which the displaying device controls.
- **The relay stores fingerprints, never keys**, and never an address.
- **Channel-aware:** `farcooler_protocol::CHANNEL` is bound into every payload so four deployments cannot accept each other's ceremonies.

---

### Task 1: The first QR payload

**Files:**
- Create: `crates/client/src/ceremony.rs`
- Modify: `crates/client/src/lib.rs`

**Interfaces:**
- Produces:
  - `pub struct Offer { pub v: u8, pub key_a: String, pub key_b: Option<String>, pub name: String, pub account: String, pub channel: String, pub ceremony: String }`
  - `pub fn offer(name: &str, account: &str, key_a: &str, key_b: Option<&str>) -> Offer` — generates `ceremony` as 128 random bits, hex.
  - `pub fn encode_offer(o: &Offer) -> String` and `pub fn decode_offer(s: &str) -> Result<Offer, CeremonyError>`.
  - `pub enum CeremonyError { Version(u8), Channel(String), Malformed(String), WrongCeremony, WrongTarget, Stale }`

- [ ] **Step 1: Write the failing tests**

```rust
    /// Nothing in the first code is a secret.
    ///
    /// Three drafts of this design put one there and each was broken by the
    /// same attack: a symmetric secret on a screen is a bearer token, and
    /// whoever films it holds what the scanner holds. This test exists so a
    /// future field cannot be added quietly.
    #[test]
    fn the_offer_carries_no_secret() {
        let o = offer("iPhone 17", "acct_1", KEY_A, None);
        let json = encode_offer(&o);
        assert!(json.contains("key_a"), "no key: {json}");
        for forbidden in ["secret", "token", "password", "seed", "jwt"] {
            assert!(!json.contains(forbidden), "{forbidden} in an offer: {json}");
        }
    }

    /// A ceremony id is a correlator, and two are never the same.
    #[test]
    fn every_offer_gets_its_own_ceremony_id() {
        let a = offer("iPhone", "acct_1", KEY_A, None);
        let b = offer("iPhone", "acct_1", KEY_A, None);
        assert_ne!(a.ceremony, b.ceremony);
        assert_eq!(a.ceremony.len(), 32, "128 bits as hex");
    }

    /// Four deployments cannot accept each other's ceremonies.
    #[test]
    fn an_offer_from_another_channel_is_refused() {
        let mut o = offer("iPhone", "acct_1", KEY_A, None);
        o.channel = "canary-but-we-are-not".into();
        let json = encode_offer(&o);
        assert!(matches!(decode_offer(&json), Err(CeremonyError::Channel(_))));
    }

    /// A future version is refused rather than half-understood.
    #[test]
    fn a_newer_version_is_refused_with_its_number() {
        let json = r#"{"v":99,"key_a":"x","name":"n","account":"a","channel":"stable","ceremony":"0"}"#;
        match decode_offer(json) {
            Err(CeremonyError::Version(99)) => {}
            other => panic!("a v99 offer was not refused by version: {other:?}"),
        }
    }

    /// A Mac offers both keys; a phone offers one.
    #[test]
    fn key_b_is_present_only_when_there_is_one() {
        assert!(offer("iPhone", "a", KEY_A, None).key_b.is_none());
        assert!(offer("MacBook", "a", KEY_A, Some(KEY_B)).key_b.is_some());
    }

    #[test]
    fn an_offer_round_trips() {
        let o = offer("MacBook Air", "acct_1", KEY_A, Some(KEY_B));
        let back = decode_offer(&encode_offer(&o)).expect("decode");
        assert_eq!(back.ceremony, o.ceremony);
        assert_eq!(back.key_a, o.key_a);
        assert_eq!(back.name, o.name);
    }
```

- [ ] **Step 2: Run to verify failure**

```bash
~/.cargo/bin/cargo test -p farcooler-client ceremony::
```

- [ ] **Step 3: Implement, then run**

Encode as compact JSON; the QR encoder in each app takes a string. Refuse on version mismatch before reading any other field, and on channel mismatch before returning.

- [ ] **Step 4: Commit**

```bash
git add crates/client/src/ceremony.rs crates/client/src/lib.rs
git commit -m "feat(client): the offer a new device shows, with nothing secret in it

Three drafts of this design put a secret in this code and each was
broken the same way: a symmetric secret on a screen is a bearer token,
and whoever films it holds what the scanner holds. A test asserts the
absence so a future field cannot add one quietly.

The ceremony id is a correlator rather than a secret. An earlier draft
removed it, reasoning that enrolling a key twice is enrolling it once --
true of this leg, and false of the reply, which then had nothing tying
it to the request."
```

---

### Task 2: The reply, and every rule that refuses one

This is the task that carries the design's security argument. Everything the spec says the reply leg must refuse lives here.

**Files:**
- Modify: `crates/client/src/ceremony.rs`

**Interfaces:**
- Produces:
  - `pub struct RunnerEntry { pub id: String, pub label: String, pub alias: String, pub address: String, pub user: String, pub port: u16, pub host_key: String, pub pending: bool }`
  - `pub struct Manifest { pub v: u8, pub ceremony: String, pub account: String, pub channel: String, pub target: String, pub runners: Vec<RunnerEntry> }` — `target` is Key A's fingerprint.
  - `pub fn manifest(offer: &Offer, runners: Vec<RunnerEntry>) -> Manifest`
  - `pub fn accept_manifest(encoded: &str, expecting: &Offer, already_taken: bool) -> Result<Manifest, CeremonyError>`
  - `pub fn manifest_fits(m: &Manifest, budget_bytes: usize) -> bool`

- [ ] **Step 1: Write the failing tests**

```rust
    /// A reply for another ceremony is not this ceremony's reply.
    ///
    /// Two devices onboarded in one room would otherwise scan each other's
    /// manifests, and yesterday's would be as acceptable as today's.
    #[test]
    fn a_manifest_for_another_ceremony_is_refused() {
        let mine = offer("iPhone", "acct_1", KEY_A, None);
        let theirs = offer("iPad", "acct_1", KEY_B, None);
        let reply = encode_manifest(&manifest(&theirs, vec![a_runner()]));
        assert!(matches!(
            accept_manifest(&reply, &mine, false),
            Err(CeremonyError::WrongCeremony)
        ));
    }

    /// A reply addressed to another key is refused even with the right id.
    #[test]
    fn a_manifest_for_another_target_is_refused() {
        let mine = offer("iPhone", "acct_1", KEY_A, None);
        let mut m = manifest(&mine, vec![a_runner()]);
        m.target = "SHA256:someone-else".into();
        assert!(matches!(
            accept_manifest(&encode_manifest(&m), &mine, false),
            Err(CeremonyError::WrongTarget)
        ));
    }

    /// One reply per ceremony, so a forged one cannot follow a real one.
    #[test]
    fn a_second_manifest_is_refused_once_one_is_taken() {
        let mine = offer("iPhone", "acct_1", KEY_A, None);
        let reply = encode_manifest(&manifest(&mine, vec![a_runner()]));
        assert!(accept_manifest(&reply, &mine, false).is_ok());
        assert!(accept_manifest(&reply, &mine, true).is_err());
    }

    /// Account and channel are bound here too, not only in the offer.
    #[test]
    fn a_manifest_from_another_account_is_refused() {
        let mine = offer("iPhone", "acct_1", KEY_A, None);
        let mut m = manifest(&mine, vec![a_runner()]);
        m.account = "acct_2".into();
        assert!(accept_manifest(&encode_manifest(&m), &mine, false).is_err());
    }

    /// The cap is measured bytes, never an assumed runner count.
    ///
    /// Fifteen records at 120 bytes is already about 1800 before versioning,
    /// the account, the target key, encoding and error correction. A count is
    /// a consequence; the budget is the mechanism.
    #[test]
    fn the_manifest_is_capped_by_measured_bytes() {
        let mine = offer("iPhone", "acct_1", KEY_A, None);
        let many: Vec<_> = (0..200).map(|i| a_runner_named(&format!("runner-{i}"))).collect();
        let m = manifest(&mine, many);
        assert!(!manifest_fits(&m, 1_800), "a 200-runner manifest claimed to fit");
        let few = manifest(&mine, vec![a_runner()]);
        assert!(manifest_fits(&few, 1_800));
    }

    /// A pinned host key travels with the address it belongs to.
    #[test]
    fn every_runner_carries_the_host_key_to_pin() {
        let mine = offer("iPhone", "acct_1", KEY_A, None);
        let m = manifest(&mine, vec![a_runner()]);
        assert!(m.runners[0].host_key.starts_with("SHA256:"));
    }
```

- [ ] **Step 2: Run to verify failure, implement, run again**

```bash
~/.cargo/bin/cargo test -p farcooler-client ceremony::
```

- [ ] **Step 3: Commit**

```bash
git add crates/client/src/ceremony.rs
git commit -m "feat(client): a reply is refused unless it answers this ceremony

The reply leg is not authenticated -- coming off a screen identifies no
signer, and an earlier draft said otherwise and was wrong. What the
echoed ceremony id gives is correlation: it refuses a crossed ceremony,
an old manifest, and anyone who did not see the first code. The residual
risk is someone who filmed that code, is in the room, and presents a
forged reply first; that is in the threat model rather than argued away.

The cap is measured bytes rather than a runner count, because fifteen
records is already about 1800 bytes before versioning, the target key,
encoding and error correction."
```

---

### Task 3: The FFI the apps call

**Files:**
- Modify: `crates/client/src/ffi.rs`
- Modify: `crates/client/include/farcooler_client.h`

**Interfaces:**
- Produces, following the JSON-into-a-caller-supplied-buffer shape `farcooler_client_generate_key` already uses:
  - `farcooler_client_ceremony_offer(name, account, key_a, key_b, out, capacity) -> usize`
  - `farcooler_client_ceremony_accept(encoded, expecting_json, already_taken, out, capacity) -> usize` — writes the manifest, or `{"error":"wrong_ceremony"}`.
  - Errors are returned as JSON with a stable machine-readable code. The apps map codes to copy; **a raw Rust error string never reaches a screen.**

- [ ] **Step 1: Write the failing test**

An FFI round-trip in `crates/client/tests/`: build an offer through the C entry point, accept a manifest through it, assert the JSON shape. Then assert an error case returns `{"error":"wrong_ceremony"}` and not a Rust `Debug` string.

- [ ] **Step 2: Implement, run, and regenerate the header if it is generated**

```bash
~/.cargo/bin/cargo test -p farcooler-client
grep -n "ceremony" crates/client/include/farcooler_client.h
```

- [ ] **Step 3: Commit**

---

### Task 4: The relay proves possession and answers the account question

**Files:**
- Modify: `services/relay/src/index.ts` — `/v1/devices`, plus `/v1/devices/lookup`
- Create: `services/relay/migrations/0004_device_keys.sql`
- Modify: `services/relay/test/relay.test.ts`

**Interfaces:**
- Produces:
  - `devices.key_a_fingerprint TEXT`, `devices.state TEXT` (`pending` | `verified`)
  - `POST /v1/devices` now requires `{ fingerprint, signature }` where the signature is over the device id by Key A.
  - `POST /v1/devices/lookup` → `{ found: boolean, label?: string }`, scoped to the caller's account.
  - `POST /v1/devices/verify` → promotes `pending` to `verified`, called by the trusted device after enrolling.

- [ ] **Step 1: Write the migration**

```sql
-- The relay stores a fingerprint, never a key.
--
-- Its only use for a device key is the account lookup, and a hash answers
-- that. Storing the key would make "never install a key the relay handed you"
-- a rule to remember; storing a fingerprint makes it a fact about the schema.
ALTER TABLE devices ADD COLUMN key_a_fingerprint TEXT;
ALTER TABLE devices ADD COLUMN state TEXT NOT NULL DEFAULT 'verified';
CREATE UNIQUE INDEX devices_account_fingerprint
  ON devices (account_id, key_a_fingerprint)
  WHERE key_a_fingerprint IS NOT NULL;
```

Existing rows default to `verified`: they predate this and were created by a flow that had no pending state. They carry a NULL fingerprint, so the lookup below never matches them until the app re-registers.

**`ON CONFLICT … DO UPDATE SET` must gain the two new columns — CRITICAL.**
`services/relay/src/index.ts:249-254` upserts on `(platform, push_token)` and
names its updated columns explicitly. Adding columns in a migration does not add
them there, so an updated app would re-register successfully and silently stay
legacy: fingerprint NULL, invisible to every ceremony, with nothing on screen
saying why. Review caught this; the regression test below is what keeps it
caught.

**Re-registration needs no manual step.** The push token is stable across app
launches, so an updated app hits the conflict and updates its own row. A row
that never re-registers shows in Devices as **unverified**, pointing at updating
the app on that device rather than failing silently in a ceremony.

- [ ] **Step 2: Write the failing tests**

- `a_registration_without_a_signature_is_refused`
- `a_registration_whose_signature_does_not_verify_is_refused` — the point of the task: without it, a session-holder registers any fingerprint they have seen anywhere and the account gate checks registry membership rather than possession.
- `a_lookup_finds_only_this_accounts_devices` — same fingerprint under two accounts, each caller sees only its own.
- `a_lookup_never_says_whose_a_key_is_otherwise` — assert the response has no account field on a miss.
- `a_pending_device_is_not_found_by_lookup` — pending grants nothing.
- `verify_promotes_only_from_pending`
- **CRITICAL regression** — `an_existing_row_re_registering_gains_its_fingerprint_and_state`. Insert a row the way the old code did (no fingerprint, `verified`), re-register with the same `(platform, push_token)` and a signature, and assert the row now carries the fingerprint and the right state. Without the `DO UPDATE SET` fix above this fails, and in production it would fail silently.

- [ ] **Step 3: Implement**

Verify the ed25519 signature with WebCrypto (`crypto.subtle.importKey` with `Ed25519`, supported on Workers) over the device id bytes. If Workers' Ed25519 support is unavailable in this runtime, say so and use `@noble/ed25519` — a new dependency, and a decision to state in the commit message.

The lookup query is exactly:

```sql
SELECT id, label, state FROM devices
 WHERE key_a_fingerprint = ?1 AND account_id = ?2
```

**There is deliberately no `state` predicate**, and an earlier draft of this plan
had one — which made the lookup unable to match the very device being onboarded.
The row is `pending` until a ceremony completes, and the trusted device does this
lookup *before* enrolling and promoting, so filtering on `verified` answers
`found: false` for every legitimate onboarding. Promotion cannot move earlier
either: it requires a completed ceremony, which requires this lookup.

The predicate was the wrong half. This query answers **"is this key on my
account?"**, which is the gate, and it is satisfied the moment a device holding
that account's session proved possession. **"Has a ceremony completed?"** is a
different question — it governs the device list and eligibility for a later
grant — so `state` is *reported*, not filtered on. Anyone re-adding the filter as
hardening breaks every onboarding.

Scoping stays in the query rather than being compared afterwards, so two accounts
registering one fingerprint cannot confuse it, and a miss must carry no
information about whose a key is otherwise.

- [ ] **Step 4: Run and commit**

```bash
cd services/relay && npx vitest run
```

```bash
git add services/relay/
git commit -m "feat(relay): prove possession, and answer one account-scoped question

Registration used to record whatever fingerprint a session-holder sent,
so the account gate checked membership of a registry rather than that
the device in front of you holds the key it is showing. A signature over
the device id closes that.

The lookup is scoped to the caller's account in the query rather than
compared afterwards, so two accounts registering one fingerprint cannot
confuse it, and a miss never says whose a key is otherwise."
```

## Self-Review

**Spec coverage.** Implements "The ceremony" steps 1–5 (Tasks 1–3), and "Two gates" plus "What the relay stores" (Task 4). Local authentication at the confirmation, camera capture, `~/.ssh/config` writing and sign-out are app-side and belong to the final plan.

**Placeholders.** One conditional: Task 4 names a fallback dependency if Workers' native Ed25519 is unavailable, because that is a runtime fact this plan cannot verify from here — with the decision to be stated rather than made silently.

**Type consistency.** `Offer` is produced in Task 1 and consumed by `manifest` and `accept_manifest` in Task 2. `RunnerEntry.host_key` is the fingerprint the app pins. The FFI in Task 3 carries the same JSON shapes.
