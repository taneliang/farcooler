//! A device's Key A registration (ov-549) as the apps reach it: through the C
//! boundary. The statement and its rules are proved in `device_keys_tests.rs`;
//! these prove the answer's shape, that a refusal is a word, and that the
//! private key never comes back.

use std::ffi::CString;

use farcooler_client::device_keys::{REGISTER_NAMESPACE, register_statement};
use farcooler_client::ffi::{farcooler_client_device_registration, farcooler_client_generate_key};
use russh::keys::ssh_key::{HashAlg, PublicKey, SshSig};
use serde_json::Value;

const CHALLENGE: &str = "fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210";

fn generated() -> (String, String) {
    let comment = CString::new("farcooler-test").unwrap();
    let mut buffer = vec![0u8; 4096];
    let n = unsafe { farcooler_client_generate_key(comment.as_ptr(), buffer.as_mut_ptr(), buffer.len()) };
    let pair: Value = serde_json::from_slice(&buffer[..n]).unwrap();
    (pair["private_key"].as_str().unwrap().to_string(), pair["public_key"].as_str().unwrap().to_string())
}

fn register(private_key: &str, account: &str, challenge: &str) -> Value {
    let (key, account, challenge) =
        (CString::new(private_key).unwrap(), CString::new(account).unwrap(), CString::new(challenge).unwrap());
    let needed = unsafe {
        farcooler_client_device_registration(key.as_ptr(), account.as_ptr(), challenge.as_ptr(), std::ptr::null_mut(), 0)
    };
    let mut buffer = vec![0u8; needed];
    let written = unsafe {
        farcooler_client_device_registration(key.as_ptr(), account.as_ptr(), challenge.as_ptr(), buffer.as_mut_ptr(), buffer.len())
    };
    assert_eq!(written, needed);
    serde_json::from_slice(&buffer).unwrap()
}

/// The key an app generated signs a proof the relay can check, for that key.
#[test]
fn a_generated_key_proves_itself() {
    let (private_key, public_key) = generated();
    let answer = register(&private_key, "user_01ALICE", CHALLENGE);
    let key_a = answer["keyA"].as_str().unwrap();
    assert_eq!(key_a, public_key.trim_end_matches(" farcooler-test"));
    assert!(!answer.to_string().contains("PRIVATE KEY"));

    let public = PublicKey::from_openssh(key_a).unwrap();
    let fingerprint = public.fingerprint(HashAlg::Sha256).to_string();
    let statement = register_statement("user_01ALICE", CHALLENGE, &fingerprint, None, None);
    let signature = SshSig::from_pem(answer["proofA"].as_str().unwrap()).unwrap();
    public.verify(REGISTER_NAMESPACE, statement.as_bytes(), &signature).expect("the relay's check passes");
}

/// Every refusal is one word, and a short buffer gets nothing written.
#[test]
fn a_refusal_is_a_word() {
    let (private_key, _) = generated();
    assert_eq!(register(&private_key, "user_01ALICE", "not-a-challenge"), serde_json::json!({ "error": "challenge" }));
    assert_eq!(register(&private_key, "", CHALLENGE), serde_json::json!({ "error": "account" }));
    assert_eq!(register("not a key", "user_01ALICE", CHALLENGE), serde_json::json!({ "error": "key" }));

    let (key, account, challenge) =
        (CString::new(private_key).unwrap(), CString::new("user_01ALICE").unwrap(), CString::new(CHALLENGE).unwrap());
    let mut short = vec![0u8; 8];
    let needed = unsafe {
        farcooler_client_device_registration(key.as_ptr(), account.as_ptr(), challenge.as_ptr(), short.as_mut_ptr(), short.len())
    };
    assert!(needed > short.len());
    assert_eq!(short, vec![0u8; 8]);
    let null = unsafe {
        farcooler_client_device_registration(std::ptr::null(), std::ptr::null(), std::ptr::null(), std::ptr::null_mut(), 0)
    };
    assert!(null > 0, "NULL arguments answer a refusal, not a crash");
}
