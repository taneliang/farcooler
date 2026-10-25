//! Who a connection is told a task notice for (ov-705).
//!
//! The `notice` event is the banner a Mac posts for a task when the relay
//! won't reach it. Before ov-705 every connection heard every one, so a
//! member's Mac could post a banner about another member's decision, while
//! the relay pushed that decision to its member alone (ov-633). Now the event
//! goes as the push does:
//!
//! - the owner's connections hear every notice, as they always did;
//! - a member's hear only their own (`Notice.member` is their handle), and
//!   not one about their own write (`Notice.author` is their handle,
//!   ov-712): they did it, and the owner still hears it;
//! - a device that was a removed member's hears none.
//!
//! The owner's and nobody's tasks (an empty `member`) go to the owner alone,
//! as their pushes do. Who may answer is unchanged: any member may still
//! answer any item from Needs You, which lists everyone's. On a runner with
//! no members every connection is the owner's, so nothing changes.

use farcooler_protocol::v1::{Event, event::Payload};

use crate::people::Person;

/// Whether `viewer` is told a notice about the task of `member` (empty for
/// the owner's and nobody's) that is news of `author`'s own writes alone
/// (empty for anyone else's, ov-712).
pub fn admits(viewer: &Person, member: &str, author: &str) -> bool {
    match viewer {
        Person::Owner => true,
        Person::Member(handle) => !member.is_empty() && handle == member && handle != author,
        Person::Released => false,
    }
}

/// Whether the connection whose device is `viewer`'s is told `event`, of
/// the read state it carries (ov-740): a board's `board_reads_changed` goes
/// to the person whose marks moved alone, so no client merges another
/// person's Unread into its own. Every other event passes, and asks no
/// person. A device the store can't place hears none, as for a notice.
pub fn admits_reads(event: &Event, viewer: impl FnOnce() -> farcooler_core::Result<Person>) -> bool {
    let Some(Payload::BoardReadsChanged(reads)) = &event.payload else { return true };
    match viewer() {
        Ok(Person::Owner) => reads.member.is_empty(),
        Ok(Person::Member(handle)) => reads.member == handle,
        Ok(Person::Released) => false,
        Err(error) => {
            tracing::warn!(%error, "could not tell who a connection is, so it hears no read state");
            false
        }
    }
}

/// Whether the connection whose person `viewer` reads as is told `event`:
/// every event but a task notice is everyone's. `viewer` is asked only for a
/// notice, so fleet news costs no lookup.
pub fn admits_event(event: &Event, viewer: impl FnOnce() -> farcooler_core::Result<Person>) -> bool {
    let Some(Payload::Notice(notice)) = &event.payload else { return true };
    match viewer() {
        Ok(person) => admits(&person, &notice.member, &notice.author),
        // A person the store can't read hears nothing rather than everything
        // (a rep's pane on the local socket reads as the owner's socket does,
        // so guessing the owner would tell a member's rep every member's
        // news), but not silently (ov-717). On a team runner even the Mac
        // app's connection reads the store here (`caller_person`), so a
        // read that fails costs it this notice; it hears the next one.
        Err(error) => {
            tracing::warn!(%error, "could not tell who a connection is, so it hears no task notices");
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use farcooler_protocol::v1::{Notice, Terminal};

    fn notice(member: &str) -> Event {
        Event { payload: Some(Payload::Notice(Notice { member: member.into(), ..Notice::default() })), ..Event::default() }
    }

    #[test]
    fn the_owner_hears_every_notice_and_a_member_their_own() {
        let alice = Person::Member("alice".into());
        for member in ["", "alice", "bob"] {
            assert!(admits(&Person::Owner, member, ""), "{member}");
        }
        assert!(admits(&alice, "alice", ""));
        assert!(!admits(&alice, "bob", ""));
        assert!(!admits(&alice, "", ""), "the owner's and nobody's are the owner's to hear");
        assert!(!admits(&Person::Released, "alice", ""));
        assert!(!admits(&Person::Released, "", ""));
    }

    /// ov-712: a member isn't told of their own write, on their task or
    /// anyone's; the owner is, and so is the member whose task another
    /// member moved.
    #[test]
    fn a_member_hears_no_notice_of_their_own_write() {
        let alice = Person::Member("alice".into());
        let bob = Person::Member("bob".into());
        assert!(!admits(&alice, "alice", "alice"), "her own write on her own task");
        assert!(admits(&Person::Owner, "alice", "alice"), "the owner still hears it");
        assert!(admits(&Person::Owner, "", "alice"), "and her write on the owner's task");
        assert!(admits(&bob, "bob", "alice"), "Bob hears her move of his card");
        assert!(!admits(&alice, "bob", "alice"), "not Bob's task, nor her write");
        let mut event = notice("alice");
        if let Some(Payload::Notice(n)) = event.payload.as_mut() {
            n.author = "alice".into();
        }
        assert!(!admits_event(&event, || Ok(alice.clone())));
        assert!(admits_event(&event, || Ok(Person::Owner)));
    }

    /// ov-740: a board's read state goes to the person whose it is, and
    /// nobody else, the owner included.
    #[test]
    fn read_state_goes_to_its_person_alone() {
        use farcooler_protocol::v1::BoardReads;
        let reads = |member: &str| Event {
            payload: Some(Payload::BoardReadsChanged(BoardReads { member: member.into(), ..BoardReads::default() })),
            ..Event::default()
        };
        let (alice, bob) = (Person::Member("alice".into()), Person::Member("bob".into()));
        assert!(admits_reads(&reads(""), || Ok(Person::Owner)));
        assert!(!admits_reads(&reads(""), || Ok(alice.clone())), "the owner's marks reach no member");
        assert!(!admits_reads(&reads("alice"), || Ok(Person::Owner)), "a member's reach not the owner");
        assert!(admits_reads(&reads("alice"), || Ok(alice.clone())));
        assert!(!admits_reads(&reads("alice"), || Ok(bob.clone())), "nor another member");
        assert!(!admits_reads(&reads(""), || Ok(Person::Released)));
        assert!(!admits_reads(&reads(""), || Err(farcooler_core::DomainError::NotFound)));
        let fleet = Event { payload: Some(Payload::TerminalChanged(Terminal::default())), ..Event::default() };
        assert!(admits_reads(&fleet, || panic!("no lookup for fleet news")));
    }

    #[test]
    fn fleet_news_is_everyones_and_reads_no_person() {
        let fleet = Event { payload: Some(Payload::TerminalChanged(Terminal::default())), ..Event::default() };
        assert!(admits_event(&fleet, || panic!("no lookup for fleet news")));
        assert!(!admits_event(&notice("bob"), || Ok(Person::Member("alice".into()))));
        assert!(admits_event(&notice("bob"), || Ok(Person::Owner)));
        assert!(!admits_event(&notice(""), || Err(farcooler_core::DomainError::NotFound)));
    }
}

#[cfg(test)]
#[path = "notice_audience_pane_tests.rs"]
mod pane_tests;
