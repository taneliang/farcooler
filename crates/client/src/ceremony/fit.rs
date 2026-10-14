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
//! 2. Then a tunneled runner with an address goes as that address alone. It is
//!    granted exactly as it would have been before the tunnel existed. Runners
//!    whose address works anywhere go first ([`works_anywhere`]), because they
//!    lose least; LAN-only ones last, each group in list order. Which runners
//!    went this way is reported ([`Fitted::address_only`]), so no app grants
//!    one silently (ov-485 N1).
//! 3. Only a reply that still doesn't fit, which is the address-only reply, is
//!    refused.
//!
//! Step 3's reply is the smallest these runners can become, and it doesn't
//! depend on any token a runner answers with. So an app calls the reply ONCE
//! BEFORE it writes anything, with the runners as picked: a refusal there is
//! the only refusal the reply after the writes can give, and it comes before
//! a key reaches any `authorized_keys` and before any runner starts a tunnel.

use super::{CeremonyError, Manifest, Offer, Reach, RunnerEntry, manifest, manifest_fits};

/// A reply that fits, and the runners it had to give the tunnel up for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fitted {
    pub manifest: Manifest,
    /// The ids of runners granted through the tunnel that go as their address
    /// alone. Every app names these to the person (ov-485 N1).
    pub address_only: Vec<String>,
}

/// The reply for `runners`, degraded as the module says until it fits.
pub fn fit_manifest(
    offer: &Offer,
    runners: Vec<RunnerEntry>,
    budget_bytes: usize,
) -> Result<Fitted, CeremonyError> {
    let mut reply = manifest(offer, runners);
    let fitted = |reply: &Manifest, address_only: &[String]| Fitted {
        manifest: reply.clone(),
        address_only: address_only.to_vec(),
    };
    if manifest_fits(&reply, budget_bytes) {
        return Ok(fitted(&reply, &[]));
    }
    let original = reply.runners.clone();
    for i in 0..original.len() {
        if reply.runners[i].alternative.take().is_some() && manifest_fits(&reply, budget_bytes) {
            return Ok(fitted(&reply, &[]));
        }
    }
    let mut order: Vec<(usize, &str)> = original
        .iter()
        .enumerate()
        .filter_map(|(i, runner)| match (&runner.reach, &runner.alternative) {
            (Reach::Tailcat { .. }, Some(Reach::Direct { host, .. })) => Some((i, host.as_str())),
            _ => None,
        })
        .collect();
    // Stable: list order within each group.
    order.sort_by_key(|(_, host)| !works_anywhere(host));
    let mut address_only = Vec::new();
    for (i, _) in order {
        reply.runners[i].reach = original[i].alternative.clone().expect("filtered on Some");
        address_only.push(original[i].id.clone());
        if manifest_fits(&reply, budget_bytes) {
            return Ok(fitted(&reply, &address_only));
        }
    }
    Err(CeremonyError::TooLarge)
}

/// Whether an address is likely to answer from another network, judged from
/// its shape alone: what a device away from home can still reach.
///
/// - A name works anywhere unless it has no dot, or ends in `.local`, `.lan`,
///   `.home`, `.internal`, `.localdomain` or `.home.arpa`.
/// - An IPv4 literal works anywhere unless it is private (10/8, 172.16/12,
///   192.168/16), loopback or link-local. 100.64/10 counts as anywhere: it is
///   where Tailscale puts its nodes, which travel.
/// - An IPv6 literal works anywhere unless it is loopback, link-local
///   (`fe80::/10`) or unique-local (`fc00::/7`).
pub fn works_anywhere(host: &str) -> bool {
    if let Ok(ip) = host.parse::<std::net::Ipv4Addr>() {
        return !(ip.is_private() || ip.is_loopback() || ip.is_link_local() || ip.is_unspecified());
    }
    if let Ok(ip) = host.parse::<std::net::Ipv6Addr>() {
        let first = ip.segments()[0];
        return !(ip.is_loopback() || ip.is_unspecified() || first & 0xffc0 == 0xfe80 || first & 0xfe00 == 0xfc00);
    }
    let name = host.trim_end_matches('.').to_ascii_lowercase();
    let lan = [".local", ".lan", ".home", ".internal", ".localdomain", ".home.arpa"];
    name.contains('.') && !lan.iter().any(|suffix| name.ends_with(suffix))
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
        let reply = fit_manifest(&device, (0..7).map(tunneled).collect(), BUDGET).expect("fits").manifest;
        assert_eq!(reply.runners.len(), 7, "a runner was dropped");
        assert!(manifest_fits(&reply, BUDGET));
        accept_manifest(&encode_manifest(&reply), &device, false).expect("the device takes it");
    }

    /// Alternatives go before tunnels, oldest first, and a fleet that fits
    /// keeps everything.
    #[test]
    fn alternatives_go_first_oldest_first() {
        let device = build_offer("iPhone", "acct_1", KEY_A, None, NODE_KEY);
        let whole = fit_manifest(&device, (0..3).map(tunneled).collect(), BUDGET).unwrap().manifest;
        assert!(whole.runners.iter().all(|r| r.alternative.is_some()), "dropped what fitted");

        let four = fit_manifest(&device, (0..4).map(tunneled).collect(), BUDGET).unwrap().manifest;
        assert!(four.runners.iter().all(|r| matches!(r.reach, Reach::Tailcat { .. })));
        assert!(four.runners[0].alternative.is_none(), "the oldest kept its address beside");
        assert!(four.runners[3].alternative.is_some(), "the newest lost its address first");
    }

    /// Beyond what alternatives can buy, a tunneled runner goes as its
    /// address alone, and is reported: a runner given up the tunnel for is
    /// never granted silently (N1).
    #[test]
    fn then_tunnels_fall_back_to_their_addresses_and_are_named() {
        let device = build_offer("iPhone", "acct_1", KEY_A, None, NODE_KEY);
        let fitted = fit_manifest(&device, (0..7).map(tunneled).collect(), BUDGET).unwrap();
        let downgraded: Vec<_> = fitted
            .manifest
            .runners
            .iter()
            .filter(|r| matches!(r.reach, Reach::Direct { .. }))
            .map(|r| r.id.clone())
            .collect();
        assert!(!downgraded.is_empty(), "nothing fell back, so this tests nothing");
        assert_eq!(fitted.address_only, downgraded, "a runner went address-only unreported");
    }

    /// A runner whose address works anywhere gives up the tunnel before one
    /// stranded on a LAN, whatever the list order (N1).
    #[test]
    fn an_address_that_works_anywhere_falls_back_before_a_lan_one() {
        let device = build_offer("iPhone", "acct_1", KEY_A, None, NODE_KEY);
        let mut runners: Vec<_> = (0..5).map(tunneled).collect();
        for runner in &mut runners[..4] {
            runner.alternative = Some(Reach::Direct { host: format!("{}.local", runner.label), port: 22 });
        }
        runners[4].alternative = Some(Reach::Direct { host: "box.example.com".into(), port: 22 });
        let fitted = fit_manifest(&device, runners.clone(), BUDGET).unwrap();
        assert_eq!(fitted.address_only.first(), Some(&runners[4].id), "a LAN runner lost the tunnel first");
    }

    #[test]
    fn which_addresses_work_anywhere() {
        for host in ["box.example.com", "cosmo.tail23af.ts.net", "8.8.8.8", "100.101.102.103", "2606:4700::1111"] {
            assert!(works_anywhere(host), "{host}");
        }
        for host in ["cosmo.local", "cosmo", "nas.lan", "192.168.1.180", "10.0.0.4", "172.20.1.1", "127.0.0.1", "fe80::1", "fd12::1", "box.home.arpa"] {
            assert!(!works_anywhere(host), "{host}");
        }
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
