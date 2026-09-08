//! The key people and agents actually use: `fc-42`.
//!
//! A repository's task key prefix is computed once, at registration, from its
//! name -- and then stored, never recomputed. See `derive_prefix` and
//! `Store::assign_task_key_prefix` for why: a prefix that answered to the
//! CURRENT name would change under a rename, and every key ever written into
//! a note, spoken aloud, or handed to an agent in its opening prompt would
//! stop resolving. Keys are the one identifier in this system that leaves the
//! database.

use rusqlite::{ErrorCode, params};
use uuid::Uuid;

use farcooler_core::Result;

use crate::error::map_err;
use crate::models::uuid_blob;
use crate::store::Store;

/// A repository's task key prefix, from its name.
///
/// Initials for a multi-word name, the first two letters otherwise. Short
/// because a person types these and says them out loud.
///
/// Computed ONCE, when a repository is registered, and stored. See
/// `renaming_a_repository_does_not_change_its_task_keys` in this module's
/// tests for why this must never become a function of the current name.
///
/// Word-splitting is `is_ascii_alphanumeric`, so a name with no ASCII
/// letters or digits in it -- a name written entirely in a non-Latin script,
/// for instance -- collapses to the `"t"` fallback below, same as `"---"`
/// does. Not a correctness bug: `assign_task_key_prefix`'s collision
/// resolution handles two repositories landing on the same prefix regardless
/// of why they did, including two differently-named repositories that both
/// fall back to `"t"`. But it does mean prefixes for such names carry no
/// resemblance to the name they came from, worth knowing going in rather
/// than discovering it from a support ticket.
pub fn derive_prefix(repository_name: &str) -> String {
    let words: Vec<&str> = repository_name
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();

    let prefix: String = if words.len() > 1 {
        words.iter().filter_map(|w| w.chars().next()).collect()
    } else {
        words.first().map(|w| w.chars().take(2).collect()).unwrap_or_default()
    };

    let prefix = prefix.to_ascii_lowercase();
    // A repository named `---` still gets keys. `t` for task, which is what
    // this is a key for, rather than a random string nobody can predict.
    if prefix.is_empty() { "t".to_string() } else { prefix }
}

/// True for any SQLite constraint failure (rusqlite's primary result code
/// does not distinguish UNIQUE from CHECK, NOT NULL, or a foreign key).
/// `assign_task_key_prefix` is the only caller, and the one constraint its
/// UPDATE can ever hit is `repositories_one_task_prefix`, so here this
/// specifically means "another repository already holds this prefix" --
/// retry with a different candidate rather than propagate.
fn is_unique_violation(err: &rusqlite::Error) -> bool {
    matches!(err, rusqlite::Error::SqliteFailure(e, _) if e.code == ErrorCode::ConstraintViolation)
}

impl Store {
    /// Assigns and stores this repository's task key prefix, derived once
    /// from its name at the moment this is called.
    ///
    /// A collision with a prefix already claimed by another repository on
    /// this runner is resolved by appending a digit and trying again. The
    /// database is the referee: `repositories_one_task_prefix` (the partial
    /// unique index over non-empty prefixes) is what actually notices a
    /// collision, not a count this function takes on faith, so two
    /// registrations racing each other still cannot both win the same
    /// prefix.
    ///
    /// Bumps `resource_version` in the same `UPDATE`, same as every other
    /// mutation in this crate, so a watcher or an RPC layer that treats an
    /// unchanged version as "nothing to refresh" notices this write too.
    /// There is no `expected_version` parameter here to check against --
    /// unlike this crate's versioned mutations, this one is not a client
    /// request replaying a version it read; it is called exactly once, from
    /// inside repository registration, against a row nothing else has had a
    /// chance to see yet.
    pub fn assign_task_key_prefix(&self, repo: Uuid) -> Result<String> {
        let name = self.get_repository(repo)?.display_name;
        let base = derive_prefix(&name);

        let mut candidate = base.clone();
        let mut attempt = 1u32;
        loop {
            let outcome = self.conn().execute(
                "UPDATE repositories SET task_key_prefix = ?1, resource_version = resource_version + 1
                 WHERE id = ?2",
                params![candidate, uuid_blob(repo)],
            );
            match outcome {
                Ok(_) => return Ok(candidate),
                Err(e) if is_unique_violation(&e) => {
                    attempt += 1;
                    candidate = format!("{base}{attempt}");
                }
                Err(e) => return Err(map_err(e)),
            }
        }
    }

    /// The next key this repository has not used yet: `<prefix>-<n>`.
    ///
    /// `n` is one more than the highest numeric suffix any task CURRENTLY in
    /// this repository carries, read fresh from `tasks` rather than kept in a
    /// counter column. There is deliberately no persisted high-water mark: if
    /// the task holding the highest number is later deleted, the next key
    /// issued reuses that number. The prefix itself is read from the
    /// repository row, never derived from its current name; see this
    /// module's doc for why.
    pub fn next_task_key(&self, repo: Uuid) -> Result<String> {
        let prefix = self.get_repository(repo)?.task_key_prefix;

        // A plain aggregate query always returns exactly one row, even when
        // nothing matches the WHERE clause -- it is the aggregate itself
        // (here, MAX) that comes back NULL, not the row that goes missing.
        let max: Option<i64> = self
            .conn()
            .query_row(
                "SELECT MAX(CAST(SUBSTR(key, LENGTH(?1) + 2) AS INTEGER))
                 FROM tasks WHERE repository_id = ?2 AND key LIKE ?1 || '-%'",
                params![prefix, uuid_blob(repo)],
                |r| r.get::<_, Option<i64>>(0),
            )
            .map_err(map_err)?;

        Ok(format!("{prefix}-{}", max.unwrap_or(0) + 1))
    }
}

#[cfg(test)]
impl Store {
    /// A repository with a real, assigned task key prefix -- not the schema's
    /// bare `''` default. Every collision and every prefix-stability test
    /// needs a genuinely non-empty prefix to have anything to prove.
    pub(crate) fn register_repository_for_test(&self, name: &str) -> Uuid {
        let host = Uuid::now_v7();
        // A fresh path per call: `repository_roots.path` is globally unique,
        // and a fixture registering more than one repository (every
        // collision test does) must not trip that constraint itself.
        let path = format!("/repos/test-{}", Uuid::now_v7());
        let root = self.create_repository_root(host, &path, 0).expect("root");
        let repo = self
            .create_repository(host, root.id, name, &format!("{path}/.git"), "")
            .expect("repo");
        self.assign_task_key_prefix(repo.id).expect("prefix");
        repo.id
    }

    pub(crate) fn rename_repository_for_test(&self, id: Uuid, new_name: &str) {
        let repo = self.get_repository(id).expect("repo");
        self.update_repository(id, repo.resource_version, new_name, &repo.canonical_git_dir, &repo.remote_summary)
            .expect("rename");
    }

    /// A task whose key is whatever `next_task_key` currently says, so a test
    /// creating one and then asking for the next key exercises the exact same
    /// counting the real thing does.
    pub(crate) fn create_task_for_test(&self, repo: Uuid, title: &str) -> Uuid {
        let key = self.next_task_key(repo).expect("key");
        let id = Uuid::now_v7();
        self.conn()
            .execute(
                "INSERT INTO tasks (id, repository_id, key, title, status, status_since, created_at, resource_version)
                 VALUES (?1, ?2, ?3, ?4, 'backlog', 0, 0, 1)",
                params![uuid_blob(id), uuid_blob(repo), key, title],
            )
            .expect("insert task");
        id
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_prefix_is_letters_from_the_name_lowercased() {
        assert_eq!(derive_prefix("Far Cooler"), "fc");
        assert_eq!(derive_prefix("overnight"), "ov");
        assert_eq!(derive_prefix("my-web-app"), "mwa");
    }

    /// A name with nothing usable in it still needs a key.
    #[test]
    fn a_name_with_no_letters_falls_back_rather_than_producing_an_empty_prefix() {
        assert!(!derive_prefix("---").is_empty());
        assert!(!derive_prefix("").is_empty());
    }

    /// The reason the prefix is stored and not computed on read.
    ///
    /// A prefix derived on every read changes when a repository is renamed,
    /// and every key ever written into a note, spoken aloud, or pasted into a
    /// prompt stops resolving. Keys are the one identifier in this system that
    /// leaves the database.
    #[test]
    fn renaming_a_repository_does_not_change_its_task_keys() {
        let store = Store::open_in_memory().expect("store");
        let repo = store.register_repository_for_test("Far Cooler");
        let before = store.next_task_key(repo).expect("key");
        store.rename_repository_for_test(repo, "Something Else");
        let after = store.next_task_key(repo).expect("key");
        assert_eq!(
            before.split('-').next(),
            after.split('-').next(),
            "the prefix is the repository's, once, forever"
        );
    }

    #[test]
    fn keys_count_up_within_a_repository() {
        let store = Store::open_in_memory().expect("store");
        let repo = store.register_repository_for_test("Far Cooler");
        assert_eq!(store.next_task_key(repo).unwrap(), "fc-1");
        store.create_task_for_test(repo, "first");
        assert_eq!(store.next_task_key(repo).unwrap(), "fc-2");
    }

    /// A single repository proves the mechanism works; it proves nothing
    /// about collision resolution, since there is nothing to collide with.
    /// Two repositories that would derive the SAME prefix from their names
    /// ("Far Cooler" and "Far Corral" both reduce to "fc") is the fixture
    /// that actually exercises `assign_task_key_prefix`'s retry loop -- and
    /// distinguishes a real resolver from one that only ever returns the
    /// base prefix and lets the database's unique index fail the second
    /// registration outright.
    #[test]
    fn two_repositories_that_derive_the_same_prefix_do_not_collide() {
        let store = Store::open_in_memory().expect("store");
        let first = store.register_repository_for_test("Far Cooler");
        let second = store.register_repository_for_test("Far Corral");

        let first_prefix = store.get_repository(first).unwrap().task_key_prefix;
        let second_prefix = store.get_repository(second).unwrap().task_key_prefix;

        assert_eq!(first_prefix, "fc", "the first registrant gets the plain prefix");
        assert_ne!(
            second_prefix, first_prefix,
            "the second registrant must not silently share the first's prefix"
        );
        assert!(
            second_prefix.starts_with("fc"),
            "the resolved prefix is still recognizably derived from the name, got {second_prefix}"
        );

        // And each repository's tasks count up independently under its own,
        // now-distinct, prefix.
        assert_eq!(store.next_task_key(first).unwrap(), "fc-1");
        assert_eq!(store.next_task_key(second).unwrap(), format!("{second_prefix}-1"));
    }

    /// `assign_task_key_prefix` is a mutation like any other in this crate,
    /// and every other one bumps `resource_version` -- a watcher polling on
    /// version alone must be able to tell a bare `create_repository` (version
    /// 1, empty prefix) apart from a fully registered one (version 2, real
    /// prefix). The nearest wrong implementation is exactly what this
    /// function looked like before this test existed: an `UPDATE` that
    /// writes `task_key_prefix` and leaves `resource_version` untouched,
    /// which this test would catch by seeing `2` come back as `1`.
    #[test]
    fn assigning_a_prefix_bumps_the_repositorys_version() {
        let store = Store::open_in_memory().expect("store");
        let repo = store.register_repository_for_test("Far Cooler");
        assert_eq!(
            store.get_repository(repo).unwrap().resource_version,
            2,
            "create_repository left it at 1; assign_task_key_prefix must bump it to 2"
        );
    }

    /// Two tasks in two different repositories both counting from `-1` proves
    /// the count is scoped per repository, not global -- the nearest wrong
    /// implementation reads `MAX` over the whole `tasks` table.
    #[test]
    fn key_numbering_does_not_leak_across_repositories() {
        let store = Store::open_in_memory().expect("store");
        let one = store.register_repository_for_test("One Project");
        let two = store.register_repository_for_test("Two Project");

        store.create_task_for_test(one, "first in one");
        store.create_task_for_test(one, "second in one");

        // `two` has had no tasks created yet: if the count were global, this
        // would come back "tp-3", not "tp-1".
        assert_eq!(store.next_task_key(two).unwrap(), "tp-1");
        assert_eq!(store.next_task_key(one).unwrap(), "op-3");
    }
}
