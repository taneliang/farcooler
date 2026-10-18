//! Telling the phone that a pull request wants this runner's review (ov-315).
//!
//! It goes out as a task notice of class `review` (`kind: "task"`), so every
//! relay and phone that already knows the class delivers it, each device
//! only if it kept reviews on, and no relay change is needed. It has no task
//! key, so a tap opens the app, not a card.
//!
//! **What crosses.** A fixed title, and a body that counts: "A pull request
//! wants your review", or "3 pull requests want your review". Never a
//! repository, a PR's title or body, a number, a URL or a login: the relay
//! never sees a repository (`services/relay/README.md`). The notice id is
//! `t:pr-review` on every runner: the request belongs to the login, not to a
//! runner, and two of an owner's runners read the same ones, so a newer notice
//! from either replaces the older one on every device instead of stacking. It
//! keeps the `t:` prefix because every app reads a thread that begins with one
//! as "not a terminal" (`Destination`); another prefix would be read as a
//! pane's id.

use super::Watcher;

/// The notice's title: the same on every notice.
pub(crate) const TITLE: &str = "Review Requested";
/// The notice's id, the same on every runner. Short enough for any collapse id.
pub(crate) const NOTICE_ID: &str = "t:pr-review";

/// What the notice says of `new` pull requests.
pub(crate) fn body(new: usize) -> String {
    match new {
        1 => "A pull request wants your review".to_string(),
        n => format!("{n} pull requests want your review"),
    }
}

impl Watcher {
    /// Tell the relay that `new` pull requests want this runner's review. A
    /// runner that is not paired sends nothing.
    pub(crate) async fn announce_pr_review(&self, new: usize) {
        let Some(pairing) = self.audience() else { return };
        let counted = self.needs_you_counted().await;
        let (count, prs) = (counted.map(|c| c.0), counted.map(|c| c.1));
        let reviews = crate::review_ops::waiting(&self.service).await;
        let id = NOTICE_ID.to_string();
        let body = body(new);
        let event = super::task_notice::Class::Review;
        self.tap(super::Tapped {
            kind: Some("task"),
            title: TITLE.to_string(),
            needs_you: count,
            reviews,
            event: Some(event.event()),
            level: Some(event.level()),
            notice_id: Some(id.clone()),
            subtitle: body.clone(),
            ..super::Tapped::default()
        });
        let outgoing = crate::push::Outgoing {
            kind: Some("task"),
            title: TITLE,
            subtitle: &body,
            needs_you: count,
            reviews,
            pr_reviews: prs,
            notice_id: Some(&id),
            event: Some(event.event()),
            level: Some(event.level()),
            ..Default::default()
        };
        if self.deliver_push(pairing, outgoing).await
            && let (Some(count), Some(prs)) = (count, prs)
        {
            self.told(count, reviews, prs);
        }
    }
}

#[cfg(test)]
#[path = "pr_review_notice_tests.rs"]
mod tests;
