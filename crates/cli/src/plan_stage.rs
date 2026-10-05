//! A lane's pull request stage in `farcooler plan` (ov-312): the words after a
//! card on `plan lane show`, and the `stage` of a lane and of a card in `--json`.
//!
//! A child of `plan.rs`, whose tests (`plan_tests.rs`) hold it. The runner chose
//! the words (`daemon::pr_stage`); this only lays them out.

use farcooler_protocol::v1 as pb;

use super::count;

pub(super) use farcooler_client::plan_json::with_stage;

/// " · Waiting on alice · PR 31 · 2 threads open": where a card's pull request
/// stands, after its slice on `plan lane show`. Empty when the runner said
/// nothing.
pub(super) fn card_words(c: &pb::LaneCard) -> String {
    let Some(s) = &c.stage else { return String::new() };
    let mut words = vec![s.label.clone()];
    if s.pr_number > 0 {
        words.push(format!("PR {}", s.pr_number));
    }
    match s.unresolved_threads {
        Some(0) => words.push("no threads open".into()),
        Some(n) => words.push(format!("{} open", count(n as usize, "thread"))),
        None => {}
    }
    format!(" \u{b7} {}", words.join(" \u{b7} "))
}
