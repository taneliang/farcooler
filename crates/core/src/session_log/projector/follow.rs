//! Reading a projection a page at a time, and following it by revision
//! (ov-366).
//!
//! A client attaches with a page of the newest rows (about a hundred, never
//! the whole session), scrolls back with `before`, and from then on asks only
//! for what changed after the revision its page carried. What changed comes
//! back as id-keyed inserts, updates and removals, so a client applies each
//! to the row it already drew.

use super::fold::Projection;
use super::rows::{Change, Row};

impl Projection {
    /// Up to `limit` rows before `ord` (or the newest, for `None`), oldest
    /// first: a page a client opens on, and scrolls back through. Retracted
    /// rows are skipped, so a page holds `limit` rows a client draws.
    pub fn page(&self, before: Option<u64>, limit: usize) -> Vec<&Row> {
        let end = before.map_or(self.rows.len(), |b| (b as usize).min(self.rows.len()));
        let mut page: Vec<&Row> = self.rows[..end].iter().rev().filter(|r| !r.retracted).take(limit).collect();
        page.reverse();
        page
    }

    /// Whether any row a client would draw sits before `ord`.
    pub fn any_before(&self, ord: u64) -> bool {
        self.rows[..(ord as usize).min(self.rows.len())].iter().any(|r| !r.retracted)
    }

    /// Every row changed after revision `rev`, in row order, retracted ones
    /// included.
    pub fn changed_since(&self, rev: u64) -> Vec<&Row> {
        self.rows.iter().filter(|r| r.rev > rev).collect()
    }

    /// What a follower at revision `rev` has to apply, in row order: rows
    /// added since as inserts, rows changed since as updates, rows retracted
    /// since as removals. A row both added and retracted since is nothing to
    /// it. `None` when more than `max` changed: sending them would be most of
    /// the session, and a page is cheaper.
    pub fn changes_since(&self, rev: u64, max: usize) -> Option<Vec<Change<'_>>> {
        let mut changes = Vec::new();
        for row in self.rows.iter().filter(|r| r.rev > rev) {
            let change = match (row.born > rev, row.retracted) {
                (true, true) => continue,
                (true, false) => Change::Insert(row),
                (false, true) => Change::Remove { id: &row.id, rev: row.rev },
                (false, false) => Change::Update(row),
            };
            if changes.len() == max {
                return None;
            }
            changes.push(change);
        }
        Some(changes)
    }

    /// Take row `i` back: a copy of a row the transcript wrote. It keeps its
    /// place, leaves the index, and is never confirmed or changed again.
    pub(super) fn retract(&mut self, i: usize) {
        if self.rows[i].retracted {
            return;
        }
        self.rows[i].retracted = true;
        self.rows[i].provisional = false;
        if self.index.get(&self.rows[i].id) == Some(&i) {
            self.index.remove(&self.rows[i].id);
        }
        self.touch(i);
    }
}
