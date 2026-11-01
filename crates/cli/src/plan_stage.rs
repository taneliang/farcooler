//! A ticket's pull request stage in `farcooler plan` (ov-312): the words on
//! `plan ticket show`.
//!
//! A child of `plan.rs`, whose tests (`plan_tests.rs`) hold it. The runner chose
//! the words (`daemon::pr_stage`); this only lays them out.

use farcooler_protocol::v1 as pb;

use super::count;

/// "Waiting on alice · PR 31 · 2 threads open": where a ticket's pull request
/// stands. Empty when the runner said nothing.
pub(super) fn pr_words(stage: Option<&pb::PrStage>) -> String {
    let Some(s) = stage else { return String::new() };
    let mut words = vec![s.label.clone()];
    if s.pr_number > 0 {
        words.push(format!("PR {}", s.pr_number));
    }
    match s.unresolved_threads {
        Some(0) => words.push("no threads open".into()),
        Some(n) => words.push(format!("{} open", count(n as usize, "thread"))),
        None => {}
    }
    words.retain(|w| !w.is_empty());
    words.join(" \u{b7} ")
}
