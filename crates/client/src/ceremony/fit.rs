//! Fitting a reply into its code by giving things up in order, never by
//! refusing first (ov-485 H1).
//!
//! A reply that grants every runner through the tunnel with its address beside
//! it costs about 450 bytes a runner, so [`super::BUDGET`] holds three. A
//! fleet of seven that paired fine at its addresses must still pair, so a reply
//! that doesn't fit degrades:
//!
//! 1. Alternatives go first, in the order the runners are listed, which on
//!    every app is the order they were added: the oldest-paired runner loses
//!    its address beside the tunnel first. It still has the tunnel.
//! 2. Then a tunneled runner with an address goes as that address alone, in
//!    the same order. It is granted exactly as it would have been before the
//!    tunnel existed.
//! 3. Only a reply that still doesn't fit, which is the address-only reply, is
//!    refused.
//!
//! Step 3's reply is the smallest these runners can become, and it doesn't
//! depend on any token a runner answers with. So an app calls the reply ONCE
//! BEFORE it writes anything, with the runners as picked: a refusal there is
//! the only refusal the reply after the writes can give, and it comes before
//! a key reaches any `authorized_keys` and before any runner starts a tunnel.

use super::{CeremonyError, Manifest, Offer, Reach, RunnerEntry, manifest, manifest_fits};

/// The reply for `runners`, degraded as the module says until it fits.
pub fn fit_manifest(
    offer: &Offer,
    runners: Vec<RunnerEntry>,
    budget_bytes: usize,
) -> Result<Manifest, CeremonyError> {
    let mut reply = manifest(offer, runners);
    if manifest_fits(&reply, budget_bytes) {
        return Ok(reply);
    }
    let original = reply.runners.clone();
    for i in 0..original.len() {
        if reply.runners[i].alternative.take().is_some() && manifest_fits(&reply, budget_bytes) {
            return Ok(reply);
        }
    }
    for (i, runner) in original.iter().enumerate() {
        if let (Reach::Tailcat { .. }, Some(address @ Reach::Direct { .. })) =
            (&runner.reach, &runner.alternative)
        {
            reply.runners[i].reach = address.clone();
            if manifest_fits(&reply, budget_bytes) {
                return Ok(reply);
            }
        }
    }
    Err(CeremonyError::TooLarge)
}

#[cfg(test)]
mod tests {
    use super::super::{BUDGET, accept_manifest, encode_manifest, offer};
    use super::*;

    fn build_offer(name: &str, account: &str, key_a: &str, key_b: Option<&str>, node_key: &str) -> Offer {
        Offer { node_key: node_key.to_string(), ..offer(name, account, key_a, key_b) }
    }

    const KEY_A: &str =
        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIB1iLbeqDzK4CDeUC3t+ffVPDI9Gk+sBwIZqJZW1NfS5 device-a";
    const NODE_KEY: &str = "3q2-7wAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
    /// A production token's length, 184 bytes (`BUDGET`'s table).
    fn token() -> String {
        "t".repeat(184)
    }

    fn runner(i: usize, reach: Reach, alternative: Option<Reach>) -> RunnerEntry {
        RunnerEntry {
            id: format!("0198f0c3-0000-7000-8000-0000000000{i:02}"),
            label: format!("r{i}"),
            alias: format!("r{i}"),
            user: "you".into(),
            host_key: "SHA256:iDqoxaySm9gzxtvLrNXXpM5PimPLeBknaaNj0Rg7vz4".into(),
            reach,
            alternative,
            pending: false,
        }
    }

    fn address(i: usize) -> Reach {
        Reach::Direct { host: format!("r{i}.tail-1234.ts.net"), port: 22 }
    }

    fn tunneled(i: usize) -> RunnerEntry {
        runner(i, Reach::Tailcat { token: token(), client_key: String::new() }, Some(address(i)))
    }

    /// Seven runners, every one granted the tunnel with its address beside
    /// it: the fleet that paired at its addresses before ov-485 still pairs,
    /// and the reply the device takes keeps every runner.
    #[test]
    fn a_fleet_of_seven_tunneled_runners_still_pairs() {
        let device = build_offer("iPhone", "acct_1", KEY_A, None, NODE_KEY);
        let reply = fit_manifest(&device, (0..7).map(tunneled).collect(), BUDGET).expect("fits");
        assert_eq!(reply.runners.len(), 7, "a runner was dropped");
        assert!(manifest_fits(&reply, BUDGET));
        accept_manifest(&encode_manifest(&reply), &device, false).expect("the device takes it");
    }

    /// Alternatives go before tunnels, oldest first, and a fleet that fits
    /// keeps everything.
    #[test]
    fn alternatives_go_first_oldest_first() {
        let device = build_offer("iPhone", "acct_1", KEY_A, None, NODE_KEY);
        let whole = fit_manifest(&device, (0..3).map(tunneled).collect(), BUDGET).unwrap();
        assert!(whole.runners.iter().all(|r| r.alternative.is_some()), "dropped what fitted");

        let four = fit_manifest(&device, (0..4).map(tunneled).collect(), BUDGET).unwrap();
        assert!(four.runners.iter().all(|r| matches!(r.reach, Reach::Tailcat { .. })));
        assert!(four.runners[0].alternative.is_none(), "the oldest kept its address beside");
        assert!(four.runners[3].alternative.is_some(), "the newest lost its address first");
    }

    /// Beyond what alternatives can buy, a tunneled runner goes as its
    /// address alone: the grant it would have had before the tunnel.
    #[test]
    fn then_tunnels_fall_back_to_their_addresses() {
        let device = build_offer("iPhone", "acct_1", KEY_A, None, NODE_KEY);
        let reply = fit_manifest(&device, (0..7).map(tunneled).collect(), BUDGET).unwrap();
        assert!(matches!(reply.runners[0].reach, Reach::Direct { .. }), "oldest is still a tunnel");
    }

    /// The reply with the runners as picked, before any writes, refuses only
    /// what the reply after them would refuse: what the module calls the
    /// smallest reply. Eight addresses don't fit; seven do.
    #[test]
    fn the_check_before_the_writes_is_the_address_only_reply() {
        let device = build_offer("iPhone", "acct_1", KEY_A, None, NODE_KEY);
        let picked = |n: usize| (0..n).map(|i| runner(i, address(i), None)).collect::<Vec<_>>();
        assert!(fit_manifest(&device, picked(7), BUDGET).is_ok());
        assert!(matches!(fit_manifest(&device, picked(8), BUDGET), Err(CeremonyError::TooLarge)));
        // What fits before the writes fits after them, whatever the tokens.
        assert!(fit_manifest(&device, (0..7).map(tunneled).collect(), BUDGET).is_ok());
    }
}
