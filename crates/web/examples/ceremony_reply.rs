//! A phone and a registered runner in software for the CSP browser walk.
//! The real ceremony code builds both signatures and the seal (ov-1020).
use farcooler_ceremony::{self as ceremony, signed};
use russh::keys::ssh_key::{PrivateKey, LineEnding, private::{Ed25519Keypair, KeypairData}};
use serde_json::json;

fn main() {
    let args: Vec<_> = std::env::args().collect();
    let offer: ceremony::Offer = serde_json::from_str(&args[1]).unwrap();
    let key = |seed| PrivateKey::new(KeypairData::Ed25519(Ed25519Keypair::from_seed(&[seed; 32])), "").unwrap();
    let (device, runner) = (key(1), key(2));
    let public = device.public_key().to_openssh().unwrap();
    let device_fp = ceremony::fingerprint(&public).unwrap();
    let runner_key = runner.public_key().to_openssh().unwrap();
    let runner_fp = ceremony::fingerprint(&runner_key).unwrap();
    let entry = ceremony::RunnerEntry {
        id: runner_fp.clone(), label: "studio".into(), alias: "studio".into(), user: args[3].clone(),
        host_key: args[2].clone(), reach: ceremony::Reach::Tailcat { token: "test".into(), client_key: String::new() },
        alternative: None, pending: false, pending_why: None,
    };
    let mut manifest = ceremony::manifest(&offer, vec![entry]);
    manifest.granted_by = Some("ana@example.com".into());
    let authority = signed::Authority { runner_key, signature: signed::signature(&runner, signed::RUNNER_NAMESPACE,
        &signed::authority_statement(&offer, &device_fp)).unwrap() };
    signed::sign(&mut manifest, &offer, &device.to_openssh(LineEnding::LF).unwrap(), vec![authority]).unwrap();
    if args.get(4).is_some_and(|a| a == "unsigned") { manifest.proof = None; }
    let (sealed, code) = ceremony::seal_reply(&offer, &manifest).unwrap();
    println!("{}", json!({ "code": code, "replies": [{ "sealed": sealed, "posted_by": "ana",
        "posted_by_email": "ana@example.com", "authority": { "devices": [device_fp], "runners": [runner_fp] } }] }));
}
